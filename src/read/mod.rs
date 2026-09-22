//! Reading a screen: recognition from pixels, and the exact text the platform
//! exposes through its accessibility tree.
//!
//! The recognition models are downloaded to the cache directory on first use
//! rather than shipped in the binary.

#[cfg(target_os = "linux")]
pub mod atspi;
#[cfg(target_os = "linux")]
pub mod fuse;
#[cfg(target_os = "linux")]
pub mod geometry;
pub mod language;
pub mod tesseract;
#[cfg(windows)]
pub mod ui;

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use rten_imageproc::{bounding_rect, Rect, RotatedRect};

use crate::capture::Capture;
use crate::index::Element;

const MODEL_BASE_URL: &str = "https://ocrs-models.s3-accelerate.amazonaws.com";
const DETECTION_MODEL: &str = "text-detection.rten";
const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// The models are ~12 MB together; anything larger is an error page.
const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;

/// Lines already read are remembered by their pixels. A list that scrolls by
/// one row then costs one line of recognition instead of a screenful.
const LINE_CACHE_SIZE: usize = 4096;

pub struct Engine {
    inner: OcrEngine,
    lines: Mutex<HashMap<u64, String>>,
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

        Ok(Engine {
            inner,
            lines: Mutex::new(HashMap::new()),
        })
    }

    /// Reads a capture into numbered elements in desktop coordinates.
    pub fn read(&self, capture: &Capture) -> Result<Vec<Element>> {
        self.read_scaled(capture, 1)
    }

    pub fn read_scaled(&self, capture: &Capture, scale: u32) -> Result<Vec<Element>> {
        let scale = scale.max(1);
        // ocrs takes RGBA as it comes, so the common path hands it the capture
        // without copying or converting anything.
        let scaled;
        let pixels = if scale == 1 {
            &capture.image
        } else {
            scaled = image::imageops::resize(
                &capture.image,
                capture.image.width() * scale,
                capture.image.height() * scale,
                image::imageops::FilterType::Lanczos3,
            );
            &scaled
        };

        let source = ImageSource::from_bytes(pixels.as_raw(), pixels.dimensions())
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

        let keys: Vec<(u64, Rect)> = lines.iter().map(|line| line_key(pixels, line)).collect();
        let unread: Vec<Vec<RotatedRect>> = lines
            .iter()
            .zip(&keys)
            .filter(|(_, (key, _))| !self.remembered(*key))
            .map(|(line, _)| line.clone())
            .collect();

        let recognized = self
            .inner
            .recognize_text(&input, &unread)
            .map_err(|err| anyhow!("text recognition failed: {err}"))?;
        let mut fresh = recognized.into_iter();

        let mut elements = Vec::new();
        for (key, rect) in keys {
            let text = match self.remember_or_read(key, || {
                fresh
                    .next()
                    .flatten()
                    .map(|line| line.to_string().trim().to_owned())
            }) {
                Some(text) if !text.is_empty() => text,
                _ => continue,
            };

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

impl Engine {
    fn remembered(&self, key: u64) -> bool {
        self.lines
            .lock()
            .map(|lines| lines.contains_key(&key))
            .unwrap_or(false)
    }

    /// The text for a line, from the cache or from the reader, whichever has
    /// it. A line that is already known does not consume a fresh reading.
    fn remember_or_read(&self, key: u64, read: impl FnOnce() -> Option<String>) -> Option<String> {
        let Ok(mut lines) = self.lines.lock() else {
            return read();
        };

        if let Some(text) = lines.get(&key) {
            return Some(text.clone());
        }

        let text = read()?;
        if lines.len() >= LINE_CACHE_SIZE {
            lines.clear();
        }
        lines.insert(key, text.clone());
        Some(text)
    }
}

/// A line is identified by the pixels under it, so the same line matches
/// wherever it has moved to.
fn line_key(image: &image::RgbaImage, line: &[RotatedRect]) -> (u64, Rect) {
    let rect = bounding_rect(line.iter())
        .map(|rect| {
            Rect::from_tlbr(
                rect.top().round() as i32,
                rect.left().round() as i32,
                rect.bottom().round() as i32,
                rect.right().round() as i32,
            )
        })
        .unwrap_or(Rect::from_tlhw(0, 0, 0, 0));

    let left = rect.left().max(0) as u32;
    let top = rect.top().max(0) as u32;
    let width = (rect.width().max(0) as u32).min(image.width().saturating_sub(left));
    let height = (rect.height().max(0) as u32).min(image.height().saturating_sub(top));

    let mut hasher = DefaultHasher::new();
    (width, height).hash(&mut hasher);
    for y in top..top + height {
        for x in left..left + width {
            image.get_pixel(x, y).0.hash(&mut hasher);
        }
    }
    (hasher.finish(), rect)
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
