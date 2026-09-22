//! Place accessibility labels using window geometry or matching OCR text.

use super::atspi::Window;
use super::Placement;
use crate::capture::Region;
use crate::index::{self, Element, Source};

/// How far two matched labels may disagree about the offset, in pixels.
const OFFSET_TOLERANCE: i32 = 6;

/// A window is only placed when this many labels agree on where it sits.
const MIN_ANCHORS: usize = 2;

/// A window the tree describes and the compositor has located.
pub struct Placed {
    pub rect: Region,
    pub elements: Vec<Element>,
    /// How the window is remembered between looks: its title and size, the
    /// same key the tree cache uses.
    pub key: Option<(String, u32, u32)>,
}

/// Resolve a unique window; equal-size windows with different titles are not interchangeable.
pub fn placement_index(window: &Window, placements: &[Placement]) -> Option<usize> {
    let candidates: Vec<_> = placements
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            let same_title = candidate.title == window.title;
            let same_size = candidate.width == window.width && candidate.height == window.height;
            let score = if same_title && !window.title.is_empty() {
                2 + u8::from(same_size)
            } else if same_size && (window.title.is_empty() || candidate.title.is_empty()) {
                1
            } else {
                0
            };
            (score > 0).then_some((index, score))
        })
        .collect();
    let best = candidates.iter().map(|(_, score)| *score).max()?;
    let mut matches = candidates.iter().filter(|(_, score)| *score == best);
    let first = matches.next()?.0;
    matches.next().is_none().then_some(first)
}

/// Match each tree to one window by title and size, then either alone.
pub fn place(windows: &[Window], placements: &[Placement]) -> Vec<Placed> {
    let mut taken = vec![false; placements.len()];
    let mut placed = Vec::new();

    for window in windows.iter().filter(|window| !window.items.is_empty()) {
        let Some(index) = placement_index(window, placements) else {
            continue;
        };
        if taken[index] {
            continue;
        }
        taken[index] = true;
        let placement = &placements[index];

        placed.push(Placed {
            key: Some((window.title.clone(), window.width, window.height)),
            rect: placement.rect(),
            elements: window
                .items
                .iter()
                .map(|item| Element {
                    id: 0,
                    text: item.text.clone(),
                    x: placement.x + item.x + item.width as i32 / 2,
                    y: placement.y + item.y + item.height as i32 / 2,
                    width: item.width,
                    height: item.height,
                    source: Source::Tree,
                    ..Default::default()
                })
                .collect(),
        });
    }

    placed
}

pub fn fuse(recognized: Vec<Element>, windows: &[Window]) -> Vec<Element> {
    let mut placed: Vec<Element> = Vec::new();

    for window in windows.iter().filter(|window| !window.items.is_empty()) {
        let Some((dx, dy)) = offset(&recognized, window) else {
            continue;
        };

        for item in &window.items {
            placed.push(Element {
                id: 0,
                text: item.text.clone(),
                x: item.x + dx + item.width as i32 / 2,
                y: item.y + dy + item.height as i32 / 2,
                width: item.width,
                height: item.height,
                source: Source::Tree,
                ..Default::default()
            });
        }
    }

    index::merge_tree(recognized, placed)
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
    use crate::read::atspi::Item;

    fn element(text: &str, x: i32, y: i32) -> Element {
        Element {
            id: 0,
            text: text.to_owned(),
            x,
            y,
            width: 40,
            height: 12,
            source: Source::Ocr,
            ..Default::default()
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

    fn placement(title: &str, x: i32, y: i32) -> Placement {
        Placement {
            title: title.to_owned(),
            x,
            y,
            width: 800,
            height: 600,
            focused: false,
            pid: None,
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
    fn a_located_window_needs_no_recognition() {
        let tree = window(vec![item("Documents", 0, 0), item("Trash", 0, 80)]);
        let placed = place(&[tree], &[placement("Files", 500, 300)]);

        assert_eq!(placed.len(), 1, "matched on size");
        let trash = placed[0]
            .elements
            .iter()
            .find(|element| element.text == "Trash")
            .expect("placed");
        assert_eq!((trash.x, trash.y), (520, 386));
    }

    #[test]
    fn a_window_the_compositor_does_not_list_is_left_alone() {
        let tree = Window {
            title: "Files".to_owned(),
            width: 123,
            height: 456,
            items: vec![item("Documents", 0, 0)],
        };
        assert!(place(&[tree], &[placement("Editor", 0, 0)]).is_empty());
    }

    #[test]
    fn indistinguishable_windows_are_not_assigned_arbitrary_positions() {
        let first = window(vec![item("One", 0, 0)]);
        let second = window(vec![item("Two", 0, 0)]);
        let placed = place(
            &[first, second],
            &[placement("Files", 0, 0), placement("Files", 900, 0)],
        );

        assert!(placed.is_empty());
    }

    #[test]
    fn hidden_tree_cannot_claim_a_visible_window_of_the_same_size() {
        let tree = window(vec![item("Documents", 0, 0)]);
        assert!(place(&[tree], &[placement("Editor", 0, 0)]).is_empty());
    }

    #[test]
    fn recognition_survives_where_the_tree_says_nothing() {
        let recognized = vec![element("Save", 10, 10)];
        let fused = fuse(recognized, &[]);
        assert_eq!(fused.len(), 1);
        assert_eq!(fused[0].text, "Save");
    }
    #[test]
    fn empty_accessibility_windows_do_not_hide_ocr_text() {
        assert!(place(&[window(Vec::new())], &[placement("Files", 0, 0)]).is_empty());
    }
}
