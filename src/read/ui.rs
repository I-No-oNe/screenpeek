//! Read Windows UI Automation labels and rectangles before falling back to OCR.

use anyhow::{anyhow, Result};
use uiautomation::types::TreeScope;
use uiautomation::{UIAutomation, UIElement};

use crate::index::{self, Element};

pub fn elements() -> Result<Vec<Element>> {
    let automation = UIAutomation::new().map_err(|error| anyhow!("{error}"))?;
    let root = automation
        .get_root_element()
        .map_err(|error| anyhow!("{error}"))?;
    let anything = automation
        .create_true_condition()
        .map_err(|error| anyhow!("{error}"))?;
    let found = root
        .find_all(TreeScope::Subtree, &anything)
        .map_err(|error| anyhow!("{error}"))?;

    let mut elements: Vec<Element> = found.iter().filter_map(readable).collect();
    index::number(&mut elements);
    Ok(elements)
}

fn readable(element: &UIElement) -> Option<Element> {
    if element.is_offscreen().unwrap_or(true) {
        return None;
    }

    let text = element.get_name().ok()?.trim().to_owned();
    if text.is_empty() {
        return None;
    }

    let rect = element.get_bounding_rectangle().ok()?;
    if rect.get_width() <= 0 || rect.get_height() <= 0 {
        return None;
    }

    Some(Element {
        id: 0,
        text,
        x: rect.get_left() + rect.get_width() / 2,
        y: rect.get_top() + rect.get_height() / 2,
        width: rect.get_width() as u32,
        height: rect.get_height() as u32,
        source: crate::index::Source::Tree,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows desktop"]
    fn the_desktop_tree_reads_as_clickable_elements() {
        let elements = elements().expect("UI Automation must be available");
        assert!(!elements.is_empty(), "a desktop always has a named control");

        for pair in elements.windows(2) {
            assert!(pair[0].id < pair[1].id);
            assert!((pair[0].y, pair[0].x) <= (pair[1].y, pair[1].x));
        }
        for element in &elements {
            assert!(!element.text.trim().is_empty());
            assert!(element.width > 0 && element.height > 0);
        }
    }
}
