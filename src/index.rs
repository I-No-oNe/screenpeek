//! Scan results and the cache that lets a later `click` reuse them.

use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// Distinguish OCR text from application-provided labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Read from pixels, so it can be misread.
    #[default]
    Ocr,
    /// Reported by the platform, so it is the application's own label.
    Tree,
}

/// One line of text found on screen.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Element {
    /// Position in the scan, from the top left of the screen.
    pub id: usize,
    pub text: String,
    /// Centre of the text in desktop coordinates: where a click lands.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Absent in a cache written before this field existed, which reads as
    /// recognized text.
    #[serde(default)]
    pub source: Source,
}

impl fmt::Display for Element {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} @{},{}", self.id, self.text, self.x, self.y)
    }
}

/// Prefer tree labels for matching controls without dropping unrelated OCR text.
pub fn merge_tree(mut pixels: Vec<Element>, tree: Vec<Element>) -> Vec<Element> {
    pixels.retain(|pixel| !tree.iter().any(|control| same_control(pixel, control)));
    pixels.extend(tree);
    number(&mut pixels);
    pixels
}

fn same_control(pixel: &Element, control: &Element) -> bool {
    let dx = (i64::from(pixel.x) - i64::from(control.x)).abs();
    let dy = (i64::from(pixel.y) - i64::from(control.y)).abs();
    if 2 * dx > i64::from(control.width) || 2 * dy > i64::from(control.height) {
        return false;
    }
    let pixel_text = fold(&pixel.text);
    let tree_text = fold(&control.text);
    !pixel_text.is_empty()
        && !tree_text.is_empty()
        && (pixel_text.contains(&tree_text) || tree_text.contains(&pixel_text))
}

/// One saved scan.
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Seconds since the Unix epoch.
    pub taken_at: u64,
    pub elements: Vec<Element>,
}

impl Snapshot {
    pub fn new(elements: Vec<Element>) -> Snapshot {
        let taken_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|age| age.as_secs())
            .unwrap_or(0);
        Snapshot { taken_at, elements }
    }

    /// The last scan, or `None`. A cache from an older version counts as absent.
    pub fn load() -> Option<Snapshot> {
        let raw = fs::read(cache_path().ok()?).ok()?;
        serde_json::from_slice(&raw).ok()
    }

    pub fn save(&self) -> Result<()> {
        let path = cache_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let json = serde_json::to_vec(self)?;
        fs::write(&path, json).with_context(|| format!("cannot write {}", path.display()))
    }

    /// Resolve IDs or text using exact, prefix, substring, then folded matching; reject ambiguity.
    pub fn find(&self, query: &str) -> Result<&Element> {
        if let Ok(id) = query.parse::<usize>() {
            return self
                .elements
                .iter()
                .find(|element| element.id == id)
                .ok_or_else(|| anyhow!("the last scan has no element {id}"));
        }

        let lowered = query.to_lowercase();
        let folded = fold(query);
        // Try OCR-tolerant folding only after ordinary matching fails.
        type Rule = (bool, fn(&str, &str) -> bool);
        let rules: [Rule; 4] = [
            (false, |text, needle| text == needle),
            (false, |text, needle| text.starts_with(needle)),
            (false, |text, needle| text.contains(needle)),
            (true, |text, needle| text.contains(needle)),
        ];

        for (folding, matches) in rules {
            let needle = if folding { &folded } else { &lowered };
            let hits: Vec<&Element> = self
                .elements
                .iter()
                .filter(|element| {
                    let text = if folding {
                        fold(&element.text)
                    } else {
                        element.text.to_lowercase()
                    };
                    matches(&text, needle)
                })
                .collect();

            match hits.as_slice() {
                [] => continue,
                [hit] => return Ok(hit),
                hits => {
                    let listing = hits
                        .iter()
                        .map(|hit| hit.to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    bail!(
                        "{} elements match {query:?}, click one by id:\n{listing}",
                        hits.len()
                    );
                }
            }
        }

        bail!("nothing on screen matches {query:?}")
    }

    /// Whether this snapshot can answer a query.
    pub fn can_resolve(&self, query: &str) -> bool {
        self.find(query).is_ok()
    }
}

/// Normalize accents and common OCR substitutions without conflating scripts.
fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    let mut last_space = true;

    for character in text.nfkd().flat_map(char::to_lowercase) {
        // Combining marks carry the accents, niqqud and harakat that a query
        // rarely repeats; the joining controls carry nothing at all.
        if is_combining_mark(character)
            || matches!(character, '\u{0640}' | '\u{200C}' | '\u{200D}' | '\u{00AD}')
        {
            continue;
        }
        if character.is_whitespace() {
            if !last_space {
                folded.push(' ');
                last_space = true;
            }
            continue;
        }
        last_space = false;
        // The ligatures are two letters that a query almost always spells out.
        if let Some(spelled) = match character {
            'æ' => Some("ae"),
            'œ' => Some("oe"),
            'ß' => Some("ss"),
            _ => None,
        } {
            folded.push_str(spelled);
            continue;
        }
        folded.push(match character {
            // Arabic spells the same word with any of these.
            '\u{0623}' | '\u{0625}' | '\u{0622}' | '\u{0671}' => '\u{0627}',
            '\u{0629}' => '\u{0647}',
            '\u{0649}' => '\u{064A}',
            // Recognition confuses these Latin shapes with each other.
            'l' | 'i' | '1' | '|' => 'i',
            'o' | '0' => 'o',
            other => other,
        });
    }

    folded.replace("rn", "m").trim().to_owned()
}

/// Sorts elements into reading order and renumbers them.
pub fn number(elements: &mut [Element]) {
    elements.sort_by_key(|element| (element.y, element.x));
    for (id, element) in elements.iter_mut().enumerate() {
        element.id = id;
    }
}

fn cache_path() -> Result<PathBuf> {
    let base = dirs::cache_dir().ok_or_else(|| anyhow!("no cache directory on this system"))?;
    Ok(base.join("screenpeek").join("last-scan-v2.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(texts: &[&str]) -> Snapshot {
        let elements = texts
            .iter()
            .enumerate()
            .map(|(id, text)| Element {
                id,
                text: (*text).to_owned(),
                x: id as i32 * 10,
                y: id as i32 * 20,
                width: 40,
                height: 12,
                source: Source::Ocr,
            })
            .collect();
        Snapshot::new(elements)
    }

    #[test]
    fn partial_trees_preserve_unmatched_text_and_replace_only_local_matches() {
        let element = |text: &str, x, y, source| Element {
            id: 0,
            text: text.into(),
            x,
            y,
            width: 60,
            height: 20,
            source,
        };
        let merged = merge_tree(
            vec![
                element("Fi1e", 20, 20, Source::Ocr),
                element("10.6 KB", 20, 60, Source::Ocr),
                element("File", 200, 20, Source::Ocr),
            ],
            vec![element("File", 20, 20, Source::Tree)],
        );
        assert_eq!(merged.len(), 3);
        assert!(merged
            .iter()
            .any(|e| e.text == "10.6 KB" && e.source == Source::Ocr));
        assert!(merged
            .iter()
            .any(|e| e.text == "File" && e.x == 200 && e.source == Source::Ocr));
        assert!(merged
            .iter()
            .any(|e| e.text == "File" && e.x == 20 && e.source == Source::Tree));
    }

    #[test]
    fn finds_by_id() {
        let snapshot = snapshot(&["Save", "Cancel"]);
        assert_eq!(snapshot.find("1").unwrap().text, "Cancel");
        assert!(snapshot.find("7").is_err());
    }

    #[test]
    fn exact_match_wins_over_prefix_and_substring() {
        let snapshot = snapshot(&["Save As...", "Save", "Autosave"]);
        assert_eq!(snapshot.find("save").unwrap().id, 1);
    }

    #[test]
    fn prefix_match_wins_over_substring() {
        let snapshot = snapshot(&["Autosave every minute", "Save As..."]);
        assert_eq!(snapshot.find("save a").unwrap().id, 1);
    }

    #[test]
    fn ambiguous_queries_are_an_error_that_lists_the_candidates() {
        let snapshot = snapshot(&["Save file", "Save copy"]);
        let error = snapshot.find("save").unwrap_err().to_string();
        assert!(error.contains("0 Save file @0,0"), "{error}");
        assert!(error.contains("1 Save copy @10,20"), "{error}");
    }

    #[test]
    fn a_misread_letter_still_resolves() {
        // OCR reads `1` for `l`, or `0` for `O`.
        assert_eq!(snapshot(&["Fi1e", "Edit"]).find("File").unwrap().id, 0);
        assert_eq!(snapshot(&["Zo0m", "Edit"]).find("Zoom").unwrap().id, 0);
    }

    #[test]
    fn an_accent_the_query_omits_still_resolves() {
        let snapshot = snapshot(&["Paramètres :", "Fichier"]);
        assert_eq!(snapshot.find("Parametres").unwrap().id, 0);
        assert_eq!(snapshot.find("Paramètres").unwrap().id, 0);
    }

    #[test]
    fn folding_never_beats_a_real_match() {
        // `Fi1e` folds onto `File`, but the exact text is what was asked for.
        let snapshot = snapshot(&["Fi1e", "File"]);
        assert_eq!(snapshot.find("File").unwrap().id, 1);
    }

    #[test]
    fn latin_folding_leaves_other_scripts_alone() {
        // Cyrillic с/о look like Latin c/o and are different letters.
        let snapshot = snapshot(&["Сохранить", "Cohpahutb"]);
        assert_eq!(snapshot.find("Сохранить").unwrap().id, 0);
        assert!(snapshot.find("Coxpahutb").is_err());
    }

    #[test]
    fn arabic_spellings_and_diacritics_fold_together() {
        let snapshot = snapshot(&["حَفِظ", "إلغاء"]);
        assert_eq!(snapshot.find("حفظ").unwrap().id, 0);
        assert_eq!(snapshot.find("الغاء").unwrap().id, 1);
    }

    #[test]
    fn a_folded_tie_still_lists_the_candidates() {
        let error = snapshot(&["Fi1e", "Fiie"])
            .find("File")
            .unwrap_err()
            .to_string();
        assert!(error.contains("2 elements match"), "{error}");
    }

    #[test]
    fn missing_text_is_an_error() {
        assert!(snapshot(&["Save"]).find("quit").is_err());
        assert!(!snapshot(&["Save"]).can_resolve("quit"));
        assert!(snapshot(&["Save"]).can_resolve("Save"));
    }
}
