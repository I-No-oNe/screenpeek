//! Read pixels with OCR and labels from platform accessibility APIs.

#[cfg(target_os = "linux")]
pub mod atspi;
#[cfg(target_os = "linux")]
pub mod fuse;
#[cfg(target_os = "linux")]
pub mod geometry;
#[cfg(target_os = "linux")]
mod geometry_helper;
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
use rten_imageproc::{bounding_rect, BoundingRect, Rect, RotatedRect};

use crate::capture::{Capture, Region};
use crate::index::{Element, Source};
pub use language::Language;

/// A visible window as the compositor reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
    pub pid: Option<u32>,
    /// The compositor's own name for the window, used to focus it.
    pub handle: Option<String>,
}

impl Placement {
    pub fn rect(&self) -> Region {
        Region {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

/// Visible windows, or none when the compositor cannot be asked.
pub fn placements() -> Vec<Placement> {
    #[cfg(target_os = "linux")]
    return geometry::windows().unwrap_or_default();
    #[cfg(not(target_os = "linux"))]
    Vec::new()
}

/// Sorted window edges, where recognition must stop joining text.
pub fn edges(placements: &[Placement]) -> Vec<i32> {
    let mut edges: Vec<i32> = placements
        .iter()
        .flat_map(|window| [window.x, window.x + window.width as i32])
        .collect();
    edges.sort_unstable();
    edges.dedup();
    edges
}

const MODEL_BASE_URL: &str = "https://ocrs-models.s3-accelerate.amazonaws.com";
const DETECTION_MODEL: &str = "text-detection.rten";
const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// The models are ~12 MB together; anything larger is an error page.
const MAX_MODEL_BYTES: u64 = 64 * 1024 * 1024;

/// Lines are cached by pixels, so scrolling re-reads only new lines.
const LINE_CACHE_SIZE: usize = 4096;

pub struct Engine {
    inner: OcrEngine,
    lines: Mutex<Lines>,
    /// Lines already re-read in another language, by pixels and language.
    reread: Mutex<HashMap<u64, String>>,
}

/// Keep two cache generations so overflow preserves recently reused lines.
#[derive(Default)]
struct Lines {
    current: HashMap<u64, String>,
    previous: HashMap<u64, String>,
}

impl Lines {
    fn get(&self, key: &u64) -> Option<&String> {
        self.current.get(key).or_else(|| self.previous.get(key))
    }

    fn extend(&mut self, additions: impl IntoIterator<Item = (u64, String)>) {
        let additions: Vec<_> = additions.into_iter().take(LINE_CACHE_SIZE).collect();
        if self.current.len() + additions.len() > LINE_CACHE_SIZE {
            self.previous = std::mem::take(&mut self.current);
        }
        self.current.extend(additions);
    }

    #[cfg(test)]
    fn keys(&self) -> impl Iterator<Item = &u64> {
        self.current.keys()
    }

    #[cfg(test)]
    fn remove(&mut self, key: &u64) {
        self.current.remove(key);
        self.previous.remove(key);
    }

    #[cfg(test)]
    fn clear(&mut self) {
        self.current.clear();
        self.previous.clear();
    }
}

impl Engine {
    #[cfg(test)]
    pub(crate) fn clear_cache_for_test(&self) {
        *self.lines.lock().unwrap() = Lines::default();
    }
    pub fn load() -> Result<Engine> {
        // Memory-map models; atomic file replacement keeps existing mappings valid.
        let detection = unsafe { Model::load_mmap(model_file(DETECTION_MODEL)?) }
            .map_err(|err| anyhow!("cannot load the text detection model: {err}"))?;
        let recognition = unsafe { Model::load_mmap(model_file(RECOGNITION_MODEL)?) }
            .map_err(|err| anyhow!("cannot load the text recognition model: {err}"))?;

        let inner = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection),
            recognition_model: Some(recognition),
            ..Default::default()
        })
        .map_err(|err| anyhow!("cannot start the OCR engine: {err}"))?;

        Ok(Engine {
            inner,
            lines: Mutex::new(Lines::default()),
            reread: Mutex::new(HashMap::new()),
        })
    }

    #[cfg(test)]
    pub fn read(&self, capture: &Capture) -> Result<Vec<Element>> {
        self.read_scaled(capture, 1, None)
    }

    /// Split neighboring controls at known window edges before recognition.
    pub fn read_within(
        &self,
        capture: &Capture,
        edges: &[i32],
        language: Option<&Language>,
    ) -> Result<Vec<Element>> {
        self.read_inner(capture, 1, edges, language)
    }

    pub fn read_scaled(
        &self,
        capture: &Capture,
        scale: u32,
        language: Option<&Language>,
    ) -> Result<Vec<Element>> {
        self.read_inner(capture, scale, &[], language)
    }

    fn read_inner(
        &self,
        capture: &Capture,
        scale: u32,
        edges: &[i32],
        language: Option<&Language>,
    ) -> Result<Vec<Element>> {
        let scale = scale.max(1);
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

        // Tesseract loads its models while the lines are being found.
        let mut pending = match language {
            Some(language) if !language.auto => Some(tesseract::Pending::start(&language.codes)?),
            _ => None,
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
        // Edges arrive in desktop coordinates; convert to image pixels.
        let local: Vec<f32> = edges
            .iter()
            .map(|edge| ((edge - capture.origin.0) * scale as i32) as f32)
            .collect();
        let lines = separate_controls(&self.inner.find_text_lines(&input, &words), &local);

        let keys: Vec<(u64, Rect)> = lines.iter().map(|line| line_key(pixels, line)).collect();
        // Snapshot hits once: parallel reads and duplicate lines must not change
        // which recognition result belongs to each missing line.
        let cached: Vec<Option<String>> = {
            let cache = self
                .lines
                .lock()
                .map_err(|_| anyhow!("line cache lock poisoned"))?;
            keys.iter()
                .map(|(key, _)| cache.get(key).cloned())
                .collect()
        };
        let unread: Vec<_> = lines
            .iter()
            .zip(&cached)
            .filter(|(_, text)| text.is_none())
            .map(|(line, _)| line.clone())
            .collect();
        // An explicit language re-reads every line, so Tesseract starts now
        // and runs while the built-in model recognizes the same lines.
        let explicit = language.filter(|language| !language.auto);
        let (recognized, early) = std::thread::scope(|scope| {
            let early = explicit.map(|language| {
                let rects: Vec<Rect> = keys.iter().map(|(_, rect)| *rect).collect();
                let keys: Vec<u64> = keys.iter().map(|(key, _)| *key).collect();
                let pending = pending.take();
                scope.spawn(move || self.reread_lines(pixels, &keys, &rects, language, pending))
            });
            let recognized = self.inner.recognize_text(&input, &unread);
            (recognized, early.map(|handle| handle.join()))
        });
        let recognized = recognized.map_err(|err| anyhow!("text recognition failed: {err}"))?;
        let mut fresh = recognized.into_iter();
        let mut additions = Vec::new();
        let mut found: Vec<(u64, Rect, String)> = Vec::new();
        for ((key, rect), cached) in keys.into_iter().zip(cached) {
            let text = cached.or_else(|| {
                let text = fresh
                    .next()
                    .flatten()
                    .map(|line| line.to_string().trim().to_owned())?;
                additions.push((key, text.clone()));
                Some(text)
            });
            found.push((key, rect, text.unwrap_or_default()));
        }
        let rereads = match (early, language) {
            (Some(done), _) => Some(done.map_err(|_| anyhow!("tesseract thread panicked"))??),
            (None, Some(language)) => {
                let texts: Vec<&str> = found.iter().map(|(_, _, text)| text.as_str()).collect();
                if language::worth_rereading(&texts) {
                    let keys: Vec<u64> = found.iter().map(|(key, _, _)| *key).collect();
                    let rects: Vec<Rect> = found.iter().map(|(_, rect, _)| *rect).collect();
                    Some(self.reread_lines(pixels, &keys, &rects, language, None)?)
                } else {
                    None
                }
            }
            (None, None) => None,
        };
        for (line, text) in found.iter_mut().zip(rereads.into_iter().flatten()) {
            if language::prefer(&line.2, &text) {
                line.2 = text;
            }
        }

        let mut elements = Vec::new();
        for (_, rect, text) in found {
            if text.is_empty() {
                continue;
            }

            let center = rect.center();
            let (x, y) = capture.to_desktop(center.x / scale as i32, center.y / scale as i32);
            elements.push(Element {
                id: 0,
                text,
                x,
                y,
                width: rect.width().max(0) as u32 / scale,
                height: rect.height().max(0) as u32 / scale,
                source: Source::Ocr,
                ..Default::default()
            });
        }

        self.lines
            .lock()
            .map_err(|_| anyhow!("line cache lock poisoned"))?
            .extend(additions);
        crate::index::number(&mut elements);
        Ok(elements)
    }

    /// Tesseract's reading of each line in `language`, cached by pixels.
    fn reread_lines(
        &self,
        pixels: &image::RgbaImage,
        keys: &[u64],
        rects: &[Rect],
        language: &Language,
        pending: Option<tesseract::Pending>,
    ) -> Result<Vec<String>> {
        let mut hasher = DefaultHasher::new();
        language.codes.hash(&mut hasher);
        let tag = hasher.finish();
        let poisoned = || anyhow!("reread cache lock poisoned");

        let mut texts: Vec<Option<String>> = {
            let cache = self.reread.lock().map_err(|_| poisoned())?;
            keys.iter()
                .map(|key| cache.get(&(key ^ tag)).cloned())
                .collect()
        };
        let missing: Vec<usize> = (0..texts.len()).filter(|&i| texts[i].is_none()).collect();
        let wanted: Vec<Rect> = missing.iter().map(|&i| rects[i]).collect();
        let fresh = match pending {
            Some(pending) => pending.read_lines(pixels, &wanted)?,
            None => tesseract::read_lines(pixels, &wanted, &language.codes)?,
        };

        let mut cache = self.reread.lock().map_err(|_| poisoned())?;
        if cache.len() + fresh.len() > LINE_CACHE_SIZE {
            cache.clear();
        }
        for (&i, text) in missing.iter().zip(fresh) {
            cache.insert(keys[i] ^ tag, text.clone());
            texts[i] = Some(text);
        }
        Ok(texts.into_iter().map(Option::unwrap_or_default).collect())
    }
}

/// Maximum word gap relative to text height, tuned on the dense fixture.
const WORD_GAP_RATIO: f32 = 0.8;
/// Fragments whose tops and baselines both differ by more than this fraction
/// of the taller one belong to different controls, however close they sit.
const CONTROL_SHAPE_TOLERANCE: f32 = 0.3;
/// A fragment less than this fraction of its neighbour's height is a
/// different control, such as body text beside a heading.
const MIN_HEIGHT_RATIO: f32 = 0.55;

// ponytail: spacing heuristic; use accessibility boundaries when widely spaced labels need grouping.
fn separate_controls(lines: &[Vec<RotatedRect>], edges: &[f32]) -> Vec<Vec<RotatedRect>> {
    lines
        .iter()
        .flat_map(|line| {
            line.chunk_by(|left, right| {
                let (a, b) = (left.bounding_rect(), right.bounding_rect());
                let tallest = left.height().max(right.height());
                !edges
                    .iter()
                    .any(|edge| *edge > a.right() && *edge < b.left())
                    && b.left() - a.right() <= WORD_GAP_RATIO * tallest
                    && same_line(a, b, tallest)
            })
            .map(<[RotatedRect]>::to_vec)
        })
        .collect()
}

/// Words of one label share their top (capitals, ascenders) or their
/// baseline, even when one has descenders or only small letters.
fn same_line(a: Rect<f32>, b: Rect<f32>, tallest: f32) -> bool {
    let tolerance = CONTROL_SHAPE_TOLERANCE * tallest;
    let aligned =
        (a.top() - b.top()).abs() <= tolerance || (a.bottom() - b.bottom()).abs() <= tolerance;
    aligned && a.height().min(b.height()) >= MIN_HEIGHT_RATIO * tallest
}

/// Key a line by its pixels, independent of position.
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

    let left = (rect.left().max(0) as u32).min(image.width());
    let top = (rect.top().max(0) as u32).min(image.height());
    let width = (rect.width().max(0) as u32).min(image.width().saturating_sub(left));
    let height = (rect.height().max(0) as u32).min(image.height().saturating_sub(top));

    let mut hasher = DefaultHasher::new();
    (width, height).hash(&mut hasher);
    for y in top..top + height {
        let start = (y as usize * image.width() as usize + left as usize) * 4;
        image.as_raw()[start..start + width as usize * 4].hash(&mut hasher);
    }
    (hasher.finish(), rect)
}

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

    let partial = path.with_extension("partial");
    fs::write(&partial, &body).with_context(|| format!("cannot write {}", partial.display()))?;
    fs::rename(&partial, path).with_context(|| format!("cannot write {}", path.display()))
}

#[cfg(test)]
mod tests;
