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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Element {
    /// Position in the scan, from the top left of the screen.
    pub id: usize,
    pub text: String,
    /// Centre of the text in desktop coordinates: where a click lands.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub source: Source,
    /// Accessible role, such as `button` or `checkbox`, when the tree knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Notable states: `checked`, `disabled`, `focused`, `selected`, `expanded`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<String>,
}

impl fmt::Display for Element {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} @{},{}", self.id, self.text, self.x, self.y)?;
        if !self.states.is_empty() {
            write!(f, " [{}]", self.states.join(" "))?;
        }
        Ok(())
    }
}

/// Prefer tree labels for matching controls without dropping unrelated OCR text.
pub fn merge_tree(mut pixels: Vec<Element>, tree: Vec<Element>) -> Vec<Element> {
    pixels.retain(|pixel| !tree.iter().any(|control| same_control(pixel, control)));
    let tree = without_echoing_icons(tree);
    pixels.extend(tree);
    number(&mut pixels);
    pixels
}

/// Drop icons named after the label beside them, which would make every
/// such label ambiguous.
fn without_echoing_icons(tree: Vec<Element>) -> Vec<Element> {
    let echoes = |icon: &Element| {
        icon.role.as_deref() == Some("image")
            && tree.iter().any(|label| {
                label.role.as_deref() != Some("image")
                    && label.text == icon.text
                    && (label.y - icon.y).abs() <= icon.height.max(label.height) as i32 / 2
                    && (label.x - icon.x).abs() <= 200
            })
    };
    let keep: Vec<bool> = tree.iter().map(|element| !echoes(element)).collect();
    tree.into_iter()
        .zip(keep)
        .filter_map(|(element, keep)| keep.then_some(element))
        .collect()
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

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
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

    /// Resolve an ID or text: exact, prefix, substring, then folded matching.
    /// More than one hit is an error that lists them.
    pub fn find(&self, query: &str) -> Result<&Element> {
        match self.matches(query).as_slice() {
            [] if query.parse::<usize>().is_ok() => bail!("the last scan has no element {query}"),
            [] => bail!("nothing on screen matches {query:?}"),
            [hit] => Ok(hit),
            hits => {
                let listing = hits
                    .iter()
                    .map(|hit| hit.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                bail!(
                    "{} elements match {query:?}, click one by id:\n{listing}",
                    hits.len()
                )
            }
        }
    }

    /// Every element the first matching rule finds.
    pub fn matches(&self, query: &str) -> Vec<&Element> {
        if let Ok(id) = query.parse::<usize>() {
            return self.elements.iter().filter(|e| e.id == id).collect();
        }
        let lowered = query.to_lowercase();
        let folded = fold(query);
        let plain: Vec<String> = self
            .elements
            .iter()
            .map(|e| e.text.to_lowercase())
            .collect();
        type Rule = fn(&str, &str) -> bool;
        let rules: [Rule; 3] = [
            |text, needle| text == needle,
            |text, needle| text.starts_with(needle),
            |text, needle| text.contains(needle),
        ];
        for rule in rules {
            let hits: Vec<&Element> = self
                .elements
                .iter()
                .zip(&plain)
                .filter(|(_, text)| rule(text, &lowered))
                .map(|(element, _)| element)
                .collect();
            if !hits.is_empty() {
                return hits;
            }
        }
        // OCR-tolerant folding only after ordinary matching fails.
        self.elements
            .iter()
            .filter(|element| fold(&element.text).contains(&folded))
            .collect()
    }

    pub fn can_resolve(&self, query: &str) -> bool {
        self.matches(query).len() == 1
    }
}

/// Keep IDs from the previous scan for elements that did not move, so an ID
/// stays valid across scans. New elements take the smallest free IDs.
pub fn keep_ids(elements: &mut [Element], previous: &[Element]) {
    const NEAR: i32 = 8;
    let mut used = std::collections::HashSet::new();
    let mut fresh = Vec::new();
    for (index, element) in elements.iter_mut().enumerate() {
        let kept = previous.iter().find(|old| {
            old.text == element.text
                && (old.x - element.x).abs() <= NEAR
                && (old.y - element.y).abs() <= NEAR
                && !used.contains(&old.id)
        });
        match kept {
            Some(old) => {
                element.id = old.id;
                used.insert(old.id);
            }
            None => fresh.push(index),
        }
    }
    let mut next = 0;
    for index in fresh {
        while used.contains(&next) {
            next += 1;
        }
        elements[index].id = next;
        used.insert(next);
    }
}

/// Normalize accents and common OCR substitutions without conflating scripts.
fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    let mut last_space = true;

    for character in text.nfkd().flat_map(char::to_lowercase) {
        // Drop accents, niqqud, harakat and joining controls.
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
                ..Default::default()
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
            ..Default::default()
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
    fn an_icon_named_like_its_label_is_not_a_second_match() {
        let at = |text: &str, x, role: &str| Element {
            text: text.into(),
            x,
            y: 706,
            width: 16,
            height: 36,
            source: Source::Tree,
            role: Some(role.into()),
            ..Default::default()
        };
        let merged = merge_tree(
            Vec::new(),
            vec![
                at("Documents", 26, "image"),
                at("Documents", 99, "label"),
                at("Trash", 26, "image"),
            ],
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(Snapshot::new(merged).find("Documents").unwrap().x, 99);
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
    fn unmoved_elements_keep_their_ids() {
        let before = snapshot(&["Save", "Cancel", "Help"]).elements;
        let mut after = snapshot(&["New", "Save", "Help"]).elements;
        for element in &mut after {
            if element.text != "New" {
                let old = before.iter().find(|b| b.text == element.text).unwrap();
                (element.x, element.y) = (old.x + 3, old.y);
            }
        }
        keep_ids(&mut after, &before);
        let id = |text: &str| after.iter().find(|e| e.text == text).unwrap().id;
        assert_eq!((id("Save"), id("Help"), id("New")), (0, 2, 1));
    }

    #[test]
    fn states_follow_the_position() {
        let mut element = snapshot(&["Dark mode"]).elements.remove(0);
        element.states = vec!["checked".into()];
        assert_eq!(element.to_string(), "0 Dark mode @0,0 [checked]");
    }

    #[test]
    fn missing_text_is_an_error() {
        assert!(snapshot(&["Save"]).find("quit").is_err());
        assert!(!snapshot(&["Save"]).can_resolve("quit"));
        assert!(snapshot(&["Save"]).can_resolve("Save"));
    }
}
