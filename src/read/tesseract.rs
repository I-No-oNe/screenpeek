//! Read installed Tesseract languages through TSV output.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Context, Result};

use crate::capture::Capture;
use crate::index::{self, Element, Source};

/// Words below this confidence are noise rather than text.
const MIN_CONFIDENCE: f32 = 45.0;

/// Scale only when requested; preserve the original desktop coordinate system.
pub fn read_scaled(capture: &Capture, language: &str, scale: u32) -> Result<Vec<Element>> {
    if !(1..=4).contains(&scale) {
        bail!("scale must be between 1 and 4");
    }
    if scale == 1 {
        return read(capture, language);
    }
    let width = capture
        .image
        .width()
        .checked_mul(scale)
        .context("scaled image is too wide")?;
    let height = capture
        .image
        .height()
        .checked_mul(scale)
        .context("scaled image is too tall")?;
    let image = image::imageops::resize(
        &capture.image,
        width,
        height,
        image::imageops::FilterType::Lanczos3,
    );
    let mut elements = read(&Capture::from_image(image), language)?;
    for element in &mut elements {
        (element.x, element.y) =
            capture.to_desktop(element.x / scale as i32, element.y / scale as i32);
        element.width = element.width.div_ceil(scale);
        element.height = element.height.div_ceil(scale);
    }
    Ok(elements)
}

/// Reads a capture in the given language, for example `deu`, `heb` or
/// `chi_sim`. Several can be combined with `+`.
pub fn read(capture: &Capture, language: &str) -> Result<Vec<Element>> {
    // PPM avoids compressing pixels only for Leptonica to decompress them.
    let (width, height) = capture.image.dimensions();
    let mut input = format!("P6\n{width} {height}\n255\n").into_bytes();
    input.reserve(capture.image.as_raw().len() / 4 * 3);
    for pixel in capture.image.as_raw().as_chunks::<4>().0 {
        let alpha = u32::from(pixel[3]);
        for channel in &pixel[..3] {
            input.push(((u32::from(*channel) * alpha + 255 * (255 - alpha)) / 255) as u8);
        }
    }

    let mut child = Command::new("tesseract")
        .args([
            "stdin",
            "stdout",
            "-l",
            language,
            "--psm",
            "11",
            "-c",
            "tessedit_create_tsv=1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| anyhow!("cannot run tesseract: {error}"))?;

    let written = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("tesseract took no input"))?
        .write_all(&input);

    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "tesseract could not read the screen in {language:?}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    written.context("cannot send capture to tesseract")?;
    Ok(elements(&String::from_utf8_lossy(&output.stdout), capture))
}

/// Whether tesseract is installed, and whether it has the language.
pub fn supports(language: &str) -> Result<()> {
    let available = installed()?;
    for part in language.split('+') {
        if !available.iter().any(|have| have == part) {
            bail!("tesseract has no {part:?} data installed");
        }
    }
    Ok(())
}

/// Cache installed Tesseract languages for this process.
pub fn installed() -> Result<Vec<String>> {
    static INSTALLED: OnceLock<Result<Vec<String>, String>> = OnceLock::new();
    INSTALLED
        .get_or_init(|| list_langs().map_err(|error| error.to_string()))
        .clone()
        .map_err(|error| anyhow!(error))
}

fn list_langs() -> Result<Vec<String>> {
    let listed = Command::new("tesseract")
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

/// Tesseract reports one row per word, with the line it belongs to. Words are
/// joined back into lines so the output matches the built-in reader.
fn elements(tsv: &str, capture: &Capture) -> Vec<Element> {
    struct Line {
        id: String,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        words: Vec<(i32, String)>,
    }

    let mut lines: Vec<Line> = Vec::new();

    for row in tsv.lines().skip(1) {
        let fields: Vec<&str> = row.split('\t').collect();
        if fields.len() < 12 || fields[0] != "5" {
            continue;
        }

        let text = fields[11].trim();
        let confidence: f32 = fields[10].parse().unwrap_or(0.0);
        if text.is_empty() || !confidence.is_finite() || confidence < MIN_CONFIDENCE {
            continue;
        }

        let (Ok(left), Ok(top), Ok(width), Ok(height)) = (
            fields[6].parse::<i32>(),
            fields[7].parse::<i32>(),
            fields[8].parse::<i32>(),
            fields[9].parse::<i32>(),
        ) else {
            continue;
        };

        if left < 0 || top < 0 || width <= 0 || height <= 0 {
            continue;
        }
        let (Some(right), Some(bottom)) = (left.checked_add(width), top.checked_add(height)) else {
            continue;
        };
        if right as u32 > capture.image.width() || bottom as u32 > capture.image.height() {
            continue;
        }
        // The direction marks are invisible and would only ever stop a label
        // matching what a person types.
        let text: String = text
            .chars()
            .filter(|character| !matches!(character, '\u{200E}' | '\u{200F}' | '\u{061C}'))
            .collect();
        if text.is_empty() {
            continue;
        }

        let id = fields[1..5].join("/");
        match lines.last_mut() {
            Some(line) if line.id == id => {
                line.left = line.left.min(left);
                line.top = line.top.min(top);
                line.right = line.right.max(right);
                line.bottom = line.bottom.max(bottom);
                line.words.push((left, text));
            }
            _ => lines.push(Line {
                id,
                left,
                top,
                right,
                bottom,
                words: vec![(left, text)],
            }),
        }
    }

    let mut elements: Vec<Element> = lines
        .into_iter()
        .map(|line| {
            let (x, y) = capture.to_desktop(
                line.left + (line.right - line.left) / 2,
                line.top + (line.bottom - line.top) / 2,
            );
            Element {
                id: 0,
                text: join(line.words),
                x,
                y,
                width: (line.right - line.left).max(0) as u32,
                height: (line.bottom - line.top).max(0) as u32,
                source: Source::Ocr,
            }
        })
        .collect();

    index::number(&mut elements);
    elements
}

/// Restore reading order for Hebrew and Arabic lines.
fn join(mut words: Vec<(i32, String)>) -> String {
    let letters = |text: &str| text.chars().filter(|c| c.is_alphabetic()).count();
    let right_to_left: usize = words
        .iter()
        .map(|(_, text)| letters(text) * usize::from(text.chars().any(rtl)))
        .sum();
    let total: usize = words.iter().map(|(_, text)| letters(text)).sum();

    // A tie goes to right-to-left: `حفظ PDF` is an Arabic label with a Latin
    // word in it, not the other way round.
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

/// Hebrew, Arabic and the other scripts written right to left.
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
    use image::RgbaImage;

    fn capture() -> Capture {
        Capture {
            image: RgbaImage::new(200, 100),
            origin: (100, 50),
        }
    }

    const TSV: &str = "level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t10\t20\t40\t12\t96\tDatei\n\
5\t1\t1\t1\t1\t2\t60\t20\t50\t12\t95\tspeichern\n\
5\t1\t1\t1\t2\t1\t10\t40\t30\t12\t92\tAbbrechen\n\
5\t1\t1\t1\t3\t1\t10\t60\t30\t12\t3\tnoise\n";

    #[test]
    fn multilingual_words_preserve_reading_order_and_spacing() {
        for (first, second, expected) in [
            ("保存", "する", "保存する"),
            ("PDF", "を保存", "PDFを保存"),
            ("保存", "PDF", "保存PDF"),
            ("파일", "저장", "파일 저장"),
            // Mostly Latin with one Hebrew word is still a left-to-right line.
            ("Save", "שם", "Save שם"),
        ] {
            let tsv = TSV.replace("Datei", first).replace("speichern", second);
            assert_eq!(elements(&tsv, &capture())[0].text, expected);
        }
    }

    /// Check RTL order against word coordinates from the Hebrew fixture.
    #[test]
    fn right_to_left_lines_are_joined_in_reading_order() {
        for (rightmost, leftmost, expected) in
            [("שמור", "קובץ", "שמור קובץ"), ("حفظ", "PDF", "حفظ PDF")]
        {
            let tsv = TSV
                .replace("\t10\t20\t40\t12\t96\tDatei", "\t60\t20\t40\t12\t96\tDatei")
                .replace(
                    "\t60\t20\t50\t12\t95\tspeichern",
                    "\t10\t20\t50\t12\t95\tspeichern",
                )
                .replace("Datei", rightmost)
                .replace("speichern", leftmost);
            assert_eq!(elements(&tsv, &capture())[0].text, expected);
        }
    }

    /// Invisible direction marks would stop a label matching what is typed.
    #[test]
    fn direction_marks_are_dropped() {
        let tsv = TSV.replace("Datei", "\u{200E}Datei\u{200F}");
        assert_eq!(elements(&tsv, &capture())[0].text, "Datei speichern");
    }

    #[test]
    fn malformed_boxes_and_confidences_cannot_become_click_targets() {
        for row in [
            "5\t1\t1\t1\t1\t1\t10\t20\t40\t12\tNaN\tbad",
            "5\t1\t1\t1\t1\t1\t2147483647\t20\t40\t12\t96\tbad",
            "5\t1\t1\t1\t1\t1\t10\t20\t-40\t12\t96\tbad",
            "5\t1\t1\t1\t1\t1\t190\t20\t40\t12\t96\tbad",
        ] {
            assert!(elements(&format!("header\n{row}\n"), &capture()).is_empty());
        }
    }

    #[test]
    fn words_join_back_into_lines() {
        let read = elements(TSV, &capture());
        assert_eq!(read.len(), 2, "{read:?}");
        assert_eq!(read[0].text, "Datei speichern");
        assert_eq!(read[1].text, "Abbrechen");
    }

    #[test]
    fn coordinates_are_in_desktop_space() {
        let read = elements(TSV, &capture());
        assert_eq!((read[0].x, read[0].y), (100 + 60, 50 + 26));
    }

    #[test]
    fn unreadable_words_are_dropped() {
        assert!(elements(TSV, &capture())
            .iter()
            .all(|element| element.text != "noise"));
    }

    #[test]
    fn a_header_alone_reads_as_nothing() {
        assert!(elements("level\tpage\n", &capture()).is_empty());
    }
}
