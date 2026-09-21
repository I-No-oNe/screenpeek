//! Text recognition with ocrs. The models are downloaded to the cache
//! directory on first use rather than shipped in the binary.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use rten::Model;

use crate::index::Element;
use crate::screen::Capture;

const MODEL_BASE_URL: &str = "https://ocrs-models.s3-accelerate.amazonaws.com";
const DETECTION_MODEL: &str = "text-detection.rten";
const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// The models are ~12 MB together; anything larger is an error page.
const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;

pub struct Engine {
    inner: OcrEngine,
}

impl Engine {
    /// Loads the OCR models, downloading them if this is the first run.
    pub fn load() -> Result<Engine> {
        let detection = Model::load_file(model_file(DETECTION_MODEL)?)
            .map_err(|err| anyhow!("cannot load the text detection model: {err}"))?;
        let recognition = Model::load_file(model_file(RECOGNITION_MODEL)?)
            .map_err(|err| anyhow!("cannot load the text recognition model: {err}"))?;

        let inner = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection),
            recognition_model: Some(recognition),
            ..Default::default()
        })
        .map_err(|err| anyhow!("cannot start the OCR engine: {err}"))?;

        Ok(Engine { inner })
    }

    /// Reads a capture into numbered elements in desktop coordinates.
    pub fn read(&self, capture: &Capture) -> Result<Vec<Element>> {
        self.read_scaled(capture, 1)
    }

    pub fn read_scaled(&self, capture: &Capture, scale: u32) -> Result<Vec<Element>> {
        let scale = scale.max(1);
        let rgb = image::DynamicImage::ImageRgba8(capture.image.clone()).into_rgb8();
        let rgb = if scale == 1 {
            rgb
        } else {
            image::imageops::resize(
                &rgb,
                rgb.width() * scale,
                rgb.height() * scale,
                image::imageops::FilterType::Lanczos3,
            )
        };
        let source = ImageSource::from_bytes(rgb.as_raw(), rgb.dimensions())
            .map_err(|err| anyhow!("cannot read the captured image: {err}"))?;
        let input = self
            .inner
            .prepare_input(source)
            .map_err(|err| anyhow!("cannot prepare the captured image: {err}"))?;

        let words = self
            .inner
            .detect_words(&input)
            .map_err(|err| anyhow!("text detection failed: {err}"))?;
        let lines = self.inner.find_text_lines(&input, &words);
        let recognized = self
            .inner
            .recognize_text(&input, &lines)
            .map_err(|err| anyhow!("text recognition failed: {err}"))?;

        let mut elements = Vec::new();
        for line in recognized.into_iter().flatten() {
            let text = line.to_string().trim().to_owned();
            if text.is_empty() {
                continue;
            }
            let rect = line.bounding_rect();
            let center = rect.center();
            let (x, y) = capture.to_desktop(center.x / scale as i32, center.y / scale as i32);
            elements.push(Element {
                id: 0,
                text,
                x,
                y,
                width: rect.width().max(0) as u32 / scale,
                height: rect.height().max(0) as u32 / scale,
            });
        }

        crate::index::number(&mut elements);
        Ok(elements)
    }
}

/// The path to a model, fetching it on first use.
fn model_file(name: &str) -> Result<PathBuf> {
    let path = dirs::cache_dir()
        .ok_or_else(|| anyhow!("no cache directory on this system"))?
        .join("screenpeek")
        .join(name);
    if path.exists() {
        return Ok(path);
    }

    let url = format!("{MODEL_BASE_URL}/{name}");
    eprintln!("screenpeek: downloading {name} (first run only)");
    download(&url, &path).with_context(|| format!("cannot download {url}"))?;
    Ok(path)
}

fn download(url: &str, path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;

    let mut body = Vec::new();
    ureq::get(url)
        .call()?
        .into_body()
        .into_reader()
        .take(MAX_MODEL_BYTES)
        .read_to_end(&mut body)?;
    if body.is_empty() {
        bail!("the download was empty");
    }

    // Rename into place so an interrupted download leaves no half-written model.
    let partial = path.with_extension("partial");
    fs::write(&partial, &body).with_context(|| format!("cannot write {}", partial.display()))?;
    fs::rename(&partial, path).with_context(|| format!("cannot write {}", path.display()))
}
