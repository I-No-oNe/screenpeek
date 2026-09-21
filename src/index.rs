//! Scan results and the cache that lets a later `click` reuse them.

use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

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
}

impl fmt::Display for Element {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} @{},{}", self.id, self.text, self.x, self.y)
    }
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

    /// Resolves an id or a piece of text to one element. Exact match beats
    /// prefix beats substring; an ambiguous query is an error, not a guess.
    pub fn find(&self, query: &str) -> Result<&Element> {
        if let Ok(id) = query.parse::<usize>() {
            return self
                .elements
                .iter()
                .find(|element| element.id == id)
                .ok_or_else(|| anyhow!("the last scan has no element {id}"));
        }

        let needle = query.to_lowercase();
        let rules: [fn(&str, &str) -> bool; 3] = [
            |text, needle| text == needle,
            |text, needle| text.starts_with(needle),
            |text, needle| text.contains(needle),
        ];

        for matches in rules {
            let hits: Vec<&Element> = self
                .elements
                .iter()
                .filter(|element| matches(&element.text.to_lowercase(), &needle))
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

/// Sorts elements into reading order and renumbers them.
pub fn number(elements: &mut [Element]) {
    elements.sort_by_key(|element| (element.y, element.x));
    for (id, element) in elements.iter_mut().enumerate() {
        element.id = id;
    }
}

fn cache_path() -> Result<PathBuf> {
    let base = dirs::cache_dir().ok_or_else(|| anyhow!("no cache directory on this system"))?;
    Ok(base.join("screenpeek").join("last-scan.json"))
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
            })
            .collect();
        Snapshot::new(elements)
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
    fn missing_text_is_an_error() {
        assert!(snapshot(&["Save"]).find("quit").is_err());
        assert!(!snapshot(&["Save"]).can_resolve("quit"));
        assert!(snapshot(&["Save"]).can_resolve("Save"));
    }
}
