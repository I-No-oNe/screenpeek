//! Recognition in languages the built-in model does not cover.
//!
//! The bundled models read English. Tesseract reads a hundred languages and is
//! packaged everywhere, so `--lang` hands the capture to it instead. It is
//! slower than the built-in path, which is why it is only used when asked for.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};

use crate::capture::Capture;
use crate::index::{self, Element};

/// Words below this confidence are noise rather than text.
const MIN_CONFIDENCE: f32 = 45.0;

/// Reads a capture in the given language, for example `deu`, `heb` or
/// `chi_sim`. Several can be combined with `+`.
pub fn read(capture: &Capture, language: &str) -> Result<Vec<Element>> {
    let mut png = Vec::new();
    capture
        .image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .context("cannot encode the capture")?;

    let mut child = Command::new("tesseract")
        .args(["stdin", "stdout", "-l", language, "--psm", "11", "tsv"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| anyhow!("cannot run tesseract: {error}"))?;

    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("tesseract took no input"))?
        .write_all(&png)?;

    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("tesseract could not read the screen in {language:?}");
    }

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

/// The language data tesseract has, or an error when it is not installed.
pub fn installed() -> Result<Vec<String>> {
    let listed = Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .map_err(|error| anyhow!("cannot run tesseract: {error}"))?;

    Ok(String::from_utf8_lossy(&listed.stdout)
        .lines()
        .skip(1)
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
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
        text: String,
    }

    let mut lines: Vec<Line> = Vec::new();

    for row in tsv.lines().skip(1) {
        let fields: Vec<&str> = row.split('\t').collect();
        if fields.len() < 12 || fields[0] != "5" {
            continue;
        }

        let text = fields[11].trim();
        let confidence: f32 = fields[10].parse().unwrap_or(0.0);
        if text.is_empty() || confidence < MIN_CONFIDENCE {
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

        let id = fields[2..5].join("/");
        match lines.last_mut() {
            Some(line) if line.id == id => {
                line.left = line.left.min(left);
                line.top = line.top.min(top);
                line.right = line.right.max(left + width);
                line.bottom = line.bottom.max(top + height);
                line.text.push(' ');
                line.text.push_str(text);
            }
            _ => lines.push(Line {
                id,
                left,
                top,
                right: left + width,
                bottom: top + height,
                text: text.to_owned(),
            }),
        }
    }

    let mut elements: Vec<Element> = lines
        .into_iter()
        .map(|line| {
            let (x, y) =
                capture.to_desktop((line.left + line.right) / 2, (line.top + line.bottom) / 2);
            Element {
                id: 0,
                text: line.text,
                x,
                y,
                width: (line.right - line.left).max(0) as u32,
                height: (line.bottom - line.top).max(0) as u32,
            }
        })
        .collect();

    index::number(&mut elements);
    elements
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn capture() -> Capture {
        Capture {
            image: RgbaImage::new(1, 1),
            origin: (100, 50),
        }
    }

    const TSV: &str = "level\tpage\tblock\tpar\tline\tword\tleft\ttop\twidth\theight\tconf\ttext\n\
5\t1\t1\t1\t1\t1\t10\t20\t40\t12\t96\tDatei\n\
5\t1\t1\t1\t1\t2\t60\t20\t50\t12\t95\tspeichern\n\
5\t1\t1\t1\t2\t1\t10\t40\t30\t12\t92\tAbbrechen\n\
5\t1\t1\t1\t3\t1\t10\t60\t30\t12\t3\tnoise\n";

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
