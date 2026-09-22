//! Read installed Tesseract languages through TSV output.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Context, Result};

use image::{imageops, RgbaImage};
use rten_imageproc::Rect;

/// Words below this confidence are noise rather than text.
const MIN_CONFIDENCE: f32 = 45.0;

/// Gap between stacked lines, and padding around each, in source pixels.
const GAP: u32 = 12;
const PAD: i32 = 10;
/// Tesseract reads screen-sized glyphs better at twice their size.
const UPSCALE: u32 = 2;

/// Read text lines in `language` (such as `eng+heb`): each rectangle is cut
/// from the image, enlarged, stacked into one image and read in a single
/// Tesseract call. Returns one text per rectangle, empty when unread.
pub fn read_lines(image: &RgbaImage, lines: &[Rect], language: &str) -> Result<Vec<String>> {
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    Pending::start(language)?.read_lines(image, lines)
}

/// A Tesseract process started ahead of time: it loads its language models
/// while the caller is still finding the lines to give it.
pub struct Pending {
    child: Option<Child>,
    language: String,
}

impl Pending {
    pub fn start(language: &str) -> Result<Pending> {
        let child = command()
            .args(["stdin", "stdout", "-l", language, "--psm", "6"])
            .args(["-c", "tessedit_create_tsv=1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| anyhow!("cannot run tesseract: {error}"))?;
        Ok(Pending {
            child: Some(child),
            language: language.to_owned(),
        })
    }

    pub fn read_lines(mut self, image: &RgbaImage, lines: &[Rect]) -> Result<Vec<String>> {
        if lines.is_empty() {
            return Ok(Vec::new());
        }
        let crops: Vec<RgbaImage> = lines
            .iter()
            .map(|line| {
                let left = (line.left() - PAD).clamp(0, image.width() as i32) as u32;
                let top = (line.top() - PAD).clamp(0, image.height() as i32) as u32;
                let right = (line.right() + PAD).clamp(0, image.width() as i32) as u32;
                let bottom = (line.bottom() + PAD).clamp(0, image.height() as i32) as u32;
                let crop = imageops::crop_imm(
                    image,
                    left,
                    top,
                    (right - left).max(1),
                    (bottom - top).max(1),
                );
                let mut crop = imageops::resize(
                    &crop.to_image(),
                    (right - left).max(1) * UPSCALE,
                    (bottom - top).max(1) * UPSCALE,
                    imageops::FilterType::CatmullRom,
                );
                light_background(&mut crop);
                crop
            })
            .collect();

        let gap = GAP * UPSCALE;
        let width = crops.iter().map(RgbaImage::width).max().unwrap_or(1) + 2 * gap;
        let height = crops.iter().map(|crop| crop.height() + gap).sum::<u32>() + gap;
        let mut stack = RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]));
        let mut bands = Vec::with_capacity(crops.len());
        let mut y = gap;
        for crop in &crops {
            imageops::replace(&mut stack, crop, i64::from(gap), i64::from(y));
            bands.push((y, y + crop.height()));
            y += crop.height() + gap;
        }

        let child = self.child.take().context("tesseract already used")?;
        let tsv = finish(child, &stack, &self.language)?;
        let mut words: Vec<Vec<(i32, String)>> = vec![Vec::new(); lines.len()];
        for word in parse(&tsv, width, height) {
            let centre = word.top + word.height / 2;
            if let Some(band) = bands
                .iter()
                .position(|(top, bottom)| centre + gap / 2 >= *top && centre < bottom + gap / 2)
            {
                words[band].push((word.left as i32, word.text));
            }
        }
        Ok(words.into_iter().map(join).collect())
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Tesseract expects dark text on a light background; invert dark themes.
fn light_background(crop: &mut RgbaImage) {
    let sum: u64 = crop
        .pixels()
        .map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]))
        .sum();
    if sum < 3 * 128 * u64::from(crop.width()) * u64::from(crop.height()) {
        for pixel in crop.pixels_mut() {
            for channel in &mut pixel.0[..3] {
                *channel = 255 - *channel;
            }
        }
    }
}

/// Send an image to a started Tesseract and return its TSV output.
fn finish(mut child: Child, image: &RgbaImage, language: &str) -> Result<String> {
    // PPM avoids compressing pixels only for Leptonica to decompress them.
    let (width, height) = image.dimensions();
    let mut input = format!("P6\n{width} {height}\n255\n").into_bytes();
    input.reserve(image.as_raw().len() / 4 * 3);
    for pixel in image.as_raw().as_chunks::<4>().0 {
        input.extend_from_slice(&pixel[..3]);
    }
    let written = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("tesseract took no input"))?
        .write_all(&input);
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "tesseract could not read {language:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    written.context("cannot send the image to tesseract")?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `tesseract`, pointed at the user's language data when none is configured.
fn command() -> Command {
    let mut command = Command::new("tesseract");
    if std::env::var_os("TESSDATA_PREFIX").is_none() {
        if let Some(directory) = user_tessdata().filter(|directory| directory.is_dir()) {
            command.env("TESSDATA_PREFIX", directory);
        }
    }
    command
}

/// Where `scripts/fetch-models.sh` puts language data.
pub fn user_tessdata() -> Option<std::path::PathBuf> {
    Some(dirs::data_dir()?.join("tessdata"))
}

pub fn supports(language: &str) -> Result<()> {
    let available = installed()?;
    for part in language.split('+') {
        if !available.iter().any(|have| have == part) {
            bail!("tesseract has no {part:?} data installed");
        }
    }
    Ok(())
}

pub fn installed() -> Result<Vec<String>> {
    static INSTALLED: OnceLock<Result<Vec<String>, String>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| list_langs().map_err(|error| error.to_string()))
        .clone()
        .map_err(|error| anyhow!(error))
}

fn list_langs() -> Result<Vec<String>> {
    let listed = command()
        .arg("--list-langs")
        .output()
        .map_err(|error| anyhow!("cannot run tesseract: {error}"))?;

    if !listed.status.success() {
        bail!(
            "cannot list tesseract languages: {}",
            String::from_utf8_lossy(&listed.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&listed.stdout)
        .lines()
        .skip(1)
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty() && line != "osd" && line != "equ")
        .collect())
}

struct Word {
    left: u32,
    top: u32,
    height: u32,
    text: String,
}

/// Confident, well-formed words from Tesseract TSV output.
fn parse(tsv: &str, width: u32, height: u32) -> Vec<Word> {
    tsv.lines()
        .skip(1)
        .filter_map(|row| {
            let fields: Vec<&str> = row.split('\t').collect();
            if fields.len() < 12 || fields[0] != "5" {
                return None;
            }
            let confidence: f32 = fields[10].parse().ok()?;
            if !confidence.is_finite() || confidence < MIN_CONFIDENCE {
                return None;
            }
            let [left, top, w, h] = [6, 7, 8, 9].map(|i| fields[i].parse::<u32>().ok());
            let (left, top, w, h) = (left?, top?, w?, h?);
            if w == 0 || h == 0 || left.checked_add(w)? > width || top.checked_add(h)? > height {
                return None;
            }
            // Invisible direction marks would break matching.
            let text: String = fields[11]
                .trim()
                .chars()
                .filter(|c| !matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}'))
                .collect();
            (!text.is_empty()).then_some(Word {
                left,
                top,
                height: h,
                text,
            })
        })
        .collect()
}

/// Restore reading order for Hebrew and Arabic lines.
pub(super) fn join(mut words: Vec<(i32, String)>) -> String {
    let letters = |text: &str| text.chars().filter(|c| c.is_alphabetic()).count();
    let right_to_left: usize = words
        .iter()
        .map(|(_, text)| letters(text) * usize::from(text.chars().any(rtl)))
        .sum();
    let total: usize = words.iter().map(|(_, text)| letters(text)).sum();

    // Ties go right-to-left: `حفظ PDF` is an Arabic label.
    if right_to_left * 2 >= total && right_to_left > 0 {
        words.sort_by_key(|(left, _)| std::cmp::Reverse(*left));
    } else {
        words.sort_by_key(|(left, _)| *left);
    }

    let mut text = String::new();
    for (_, word) in words {
        if !(text.chars().next_back().is_some_and(unspaced)
            || word.chars().next().is_some_and(unspaced)
            || text.is_empty())
        {
            text.push(' ');
        }
        text.push_str(&word);
    }
    text
}

fn rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x05FF | 0x0600..=0x06FF | 0x0700..=0x074F | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

// Chinese and Japanese do not need TSV word separators. Korean does.
fn unspaced(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF66..=0xFF9D | 0x20000..=0x2EBEF)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str =
        "level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext\n";

    fn words(rows: &[&str]) -> Vec<String> {
        parse(&format!("{HEADER}{}", rows.join("\n")), 200, 100)
            .into_iter()
            .map(|word| word.text)
            .collect()
    }

    #[test]
    fn multilingual_words_keep_reading_order_and_spacing() {
        for (first, second, expected) in [
            ("保存", "する", "保存する"),
            ("PDF", "を保存", "PDFを保存"),
            ("파일", "저장", "파일 저장"),
            ("Save", "שם", "Save שם"),
        ] {
            assert_eq!(
                join(vec![(10, first.into()), (60, second.into())]),
                expected
            );
        }
        // Right-to-left lines read from the rightmost word.
        assert_eq!(
            join(vec![(10, "קובץ".into()), (60, "שמור".into())]),
            "שמור קובץ"
        );
        assert_eq!(
            join(vec![(10, "PDF".into()), (60, "حفظ".into())]),
            "حفظ PDF"
        );
    }

    #[test]
    fn noise_and_malformed_boxes_are_dropped() {
        assert_eq!(
            words(&[
                "5\t1\t1\t1\t1\t1\t10\t20\t40\t12\t96\t\u{200E}Datei\u{200F}",
                "5\t1\t1\t1\t1\t2\t10\t20\t40\t12\t3\tnoise",
                "5\t1\t1\t1\t1\t3\t10\t20\t40\t12\tNaN\tbad",
                "5\t1\t1\t1\t1\t4\t4294967295\t20\t40\t12\t96\tbad",
                "5\t1\t1\t1\t1\t5\t190\t20\t40\t12\t96\tbad",
            ]),
            ["Datei"]
        );
    }

    #[test]
    fn dark_crops_are_inverted_and_light_ones_kept() {
        let mut dark = RgbaImage::from_pixel(4, 4, image::Rgba([20, 20, 20, 255]));
        light_background(&mut dark);
        assert_eq!(dark.get_pixel(0, 0).0, [235, 235, 235, 255]);
        let mut light = RgbaImage::from_pixel(4, 4, image::Rgba([240, 240, 240, 255]));
        light_background(&mut light);
        assert_eq!(light.get_pixel(0, 0).0, [240, 240, 240, 255]);
    }
}
