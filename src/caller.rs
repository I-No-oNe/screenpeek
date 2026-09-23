//! Keep the invoking terminal out of scans and cached click targets.
use crate::{capture::Region, index::Element, read::Placement};

pub fn filter(elements: &mut Vec<Element>, excluded: &[Region]) {
    elements.retain(|element| {
        !excluded
            .iter()
            .any(|rect| rect.contains(element.x, element.y))
    });
}

/// The windows of the terminal that launched this command, found by walking
/// up the process tree to the first ancestor that owns a visible window.
#[cfg(target_os = "linux")]
pub fn regions(windows: &[Placement]) -> Vec<Region> {
    let mut pid = std::process::id();
    for _ in 0..64 {
        let terminal: Vec<_> = windows
            .iter()
            .filter(|window| window.pid == Some(pid))
            .collect();
        if !terminal.is_empty() {
            // ponytail: only the focused window is known to be in front of the terminal.
            let front = windows
                .iter()
                .find(|window| window.focused && window.pid != Some(pid))
                .filter(|_| !terminal.iter().any(|window| window.focused));
            return terminal
                .iter()
                .flat_map(|window| match front {
                    Some(front) => minus(window.rect(), front.rect()),
                    None => vec![window.rect()],
                })
                .collect();
        }
        let Some(parent) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .and_then(|stat| parent_pid(&stat))
        else {
            break;
        };
        if parent == 0 || parent == pid {
            break;
        }
        pid = parent;
    }
    Vec::new()
}

/// The parts of `area` outside `hole`, as up to four rectangles.
fn minus(area: Region, hole: Region) -> Vec<Region> {
    let (left, top) = (area.x.max(hole.x), area.y.max(hole.y));
    let right = (area.x + area.width as i32).min(hole.x + hole.width as i32);
    let bottom = (area.y + area.height as i32).min(hole.y + hole.height as i32);
    if left >= right || top >= bottom {
        return vec![area];
    }
    let (area_right, area_bottom) = (area.x + area.width as i32, area.y + area.height as i32);
    let rect = |x: i32, y: i32, r: i32, b: i32| Region {
        x,
        y,
        width: (r - x) as u32,
        height: (b - y) as u32,
    };
    [
        rect(area.x, area.y, area_right, top),
        rect(area.x, bottom, area_right, area_bottom),
        rect(area.x, top, left, bottom),
        rect(right, top, area_right, bottom),
    ]
    .into_iter()
    .filter(|part| part.width > 0 && part.height > 0)
    .collect()
}

#[cfg(target_os = "linux")]
fn parent_pid(stat: &str) -> Option<u32> {
    // comm may contain spaces and parentheses; ppid follows the final ')'.
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(not(target_os = "linux"))]
pub fn regions(_windows: &[Placement]) -> Vec<Region> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn process_names_do_not_confuse_parent_lookup() {
        assert_eq!(parent_pid("42 (terminal (test)) S 123 0 0"), Some(123));
        assert_eq!(parent_pid("invalid"), None);
    }

    #[test]
    fn a_window_in_front_is_cut_out_of_the_terminal() {
        let region = |x, y, width, height| Region {
            x,
            y,
            width,
            height,
        };
        let parts = minus(region(0, 0, 100, 100), region(20, 30, 50, 200));
        assert!(!parts.iter().any(|part| part.contains(40, 50)));
        assert!(parts.iter().any(|part| part.contains(10, 50)));
        assert!(parts.iter().any(|part| part.contains(80, 50)));
        assert!(parts.iter().any(|part| part.contains(40, 10)));
        let area: u32 = parts.iter().map(|part| part.width * part.height).sum();
        assert_eq!(area, 100 * 100 - 50 * 70);
        assert_eq!(minus(region(0, 0, 10, 10), region(50, 50, 5, 5)).len(), 1);
    }

    #[test]
    fn cached_terminal_matches_are_removed_without_hiding_other_windows() {
        let element = |x| Element {
            id: x as usize,
            text: "click d".into(),
            x,
            y: 20,
            width: 30,
            height: 12,
            source: crate::index::Source::Ocr,
            ..Default::default()
        };
        let mut items = vec![element(20), element(220)];
        filter(
            &mut items,
            &[Region {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            }],
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].x, 220);
    }
}
