//! Putting the accessibility tree and the recognized pixels together.
//!
//! The tree knows what every control says; the pixels know where things are.
//! Matching a few labels between them gives the offset of a window on screen,
//! and with that offset every control in that window gets a real position,
//! including the ones recognition never reads, such as an icon whose only text
//! is its accessible name.

use crate::atspi::Window;
use crate::index::{self, Element};

/// How far two matched labels may disagree about the offset, in pixels.
const OFFSET_TOLERANCE: i32 = 6;

/// A window is only placed when this many labels agree on where it sits.
const MIN_ANCHORS: usize = 2;

pub fn fuse(recognized: Vec<Element>, windows: &[Window]) -> Vec<Element> {
    let mut placed: Vec<Element> = Vec::new();
    let mut covered: Vec<(i32, i32, u32, u32)> = Vec::new();

    for window in windows {
        let Some((dx, dy)) = offset(&recognized, window) else {
            continue;
        };

        covered.push((dx, dy, window.width, window.height));
        for item in &window.items {
            placed.push(Element {
                id: 0,
                text: item.text.clone(),
                x: item.x + dx + item.width as i32 / 2,
                y: item.y + dy + item.height as i32 / 2,
                width: item.width,
                height: item.height,
            });
        }
    }

    let mut elements: Vec<Element> = recognized
        .into_iter()
        .filter(|element| {
            !covered.iter().any(|(x, y, width, height)| {
                element.x >= *x
                    && element.y >= *y
                    && element.x < x + *width as i32
                    && element.y < y + *height as i32
            })
        })
        .collect();

    elements.append(&mut placed);
    index::number(&mut elements);
    elements
}

/// Where this window sits on screen, from the labels that recognition and the
/// tree agree on. `None` when too few labels match to be sure.
fn offset(recognized: &[Element], window: &Window) -> Option<(i32, i32)> {
    let mut candidates: Vec<(i32, i32)> = Vec::new();

    for item in &window.items {
        if item.text.chars().count() < 3 {
            continue;
        }
        let mut matches = recognized
            .iter()
            .filter(|element| element.text.eq_ignore_ascii_case(&item.text));

        let (Some(element), None) = (matches.next(), matches.next()) else {
            continue;
        };
        candidates.push((
            element.x - item.x - item.width as i32 / 2,
            element.y - item.y - item.height as i32 / 2,
        ));
    }

    candidates
        .iter()
        .map(|candidate| {
            let agreeing: Vec<(i32, i32)> = candidates
                .iter()
                .copied()
                .filter(|other| {
                    (other.0 - candidate.0).abs() <= OFFSET_TOLERANCE
                        && (other.1 - candidate.1).abs() <= OFFSET_TOLERANCE
                })
                .collect();
            (agreeing.len(), average(&agreeing))
        })
        .filter(|(count, _)| *count >= MIN_ANCHORS)
        .max_by_key(|(count, _)| *count)
        .map(|(_, offset)| offset)
}

fn average(offsets: &[(i32, i32)]) -> (i32, i32) {
    let count = offsets.len() as i32;
    let sum = offsets
        .iter()
        .fold((0, 0), |sum, offset| (sum.0 + offset.0, sum.1 + offset.1));
    (sum.0 / count, sum.1 / count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atspi::Item;

    fn element(text: &str, x: i32, y: i32) -> Element {
        Element {
            id: 0,
            text: text.to_owned(),
            x,
            y,
            width: 40,
            height: 12,
        }
    }

    fn item(text: &str, x: i32, y: i32) -> Item {
        Item {
            text: text.to_owned(),
            x,
            y,
            width: 40,
            height: 12,
        }
    }

    fn window(items: Vec<Item>) -> Window {
        Window {
            title: "Files".to_owned(),
            width: 800,
            height: 600,
            items,
        }
    }

    #[test]
    fn two_agreeing_labels_place_the_window() {
        // The window is at 500,300; the tree reports everything from 0,0.
        let recognized = vec![
            element("Documents", 520, 320),
            element("Pictures", 520, 360),
        ];
        let tree = window(vec![
            item("Documents", 0, 0),
            item("Pictures", 0, 40),
            item("Trash", 0, 80),
        ]);

        let fused = fuse(recognized, &[tree]);
        let trash = fused.iter().find(|e| e.text == "Trash").expect("placed");
        assert_eq!((trash.x, trash.y), (520, 400));
    }

    #[test]
    fn exact_tree_text_replaces_what_recognition_read() {
        let recognized = vec![
            element("Documents", 520, 320),
            element("Pictures", 520, 360),
            element("Descargas", 520, 400),
        ];
        let tree = window(vec![
            item("Documents", 0, 0),
            item("Pictures", 0, 40),
            item("Descargas", 0, 80),
        ]);

        let fused = fuse(recognized, &[tree]);
        assert_eq!(fused.len(), 3, "{fused:?}");
        assert!(fused.iter().all(|element| element.y >= 320));
    }

    #[test]
    fn one_matching_label_is_not_enough() {
        let recognized = vec![element("Documents", 520, 320)];
        let tree = window(vec![item("Documents", 0, 0), item("Trash", 0, 80)]);

        let fused = fuse(recognized, &[tree]);
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].text, "Documents");
    }

    #[test]
    fn an_ambiguous_label_is_not_used_as_an_anchor() {
        let recognized = vec![
            element("Open", 100, 100),
            element("Open", 300, 100),
            element("Cancel", 520, 360),
        ];
        let tree = window(vec![item("Open", 0, 0), item("Cancel", 0, 40)]);

        let fused = fuse(recognized, &[tree]);
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn recognition_survives_where_the_tree_says_nothing() {
        let recognized = vec![element("Save", 10, 10)];
        let fused = fuse(recognized, &[]);
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].text, "Save");
    }
}
