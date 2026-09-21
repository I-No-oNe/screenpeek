//! Windows UI Automation. Control names and rectangles come straight from the
//! platform, so nothing is recognized and nothing is guessed. Recognition is
//! the fallback for windows that expose no tree.

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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_tree_can_be_read() {
        let elements = elements().expect("UI Automation should answer on a Windows session");
        assert!(
            !elements.is_empty(),
            "a Windows desktop always has some named control"
        );
    }

    #[test]
    fn elements_are_numbered_in_reading_order() {
        let elements = elements().expect("UI Automation should answer on a Windows session");
        for pair in elements.windows(2) {
            assert!(pair[0].id < pair[1].id);
            assert!((pair[0].y, pair[0].x) <= (pair[1].y, pair[1].x));
        }
    }

    #[test]
    fn every_element_has_text_and_a_clickable_size() {
        for element in elements().expect("UI Automation should answer on a Windows session") {
            assert!(!element.text.trim().is_empty());
            assert!(element.width > 0 && element.height > 0);
        }
    }
}
