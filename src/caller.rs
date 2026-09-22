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
        let matches: Vec<_> = windows
            .iter()
            .filter(|window| window.pid == Some(pid))
            .map(Placement::rect)
            .collect();
        if !matches.is_empty() {
            return matches;
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
    fn cached_terminal_matches_are_removed_without_hiding_other_windows() {
        let element = |x| Element {
            id: x as usize,
            text: "click d".into(),
            x,
            y: 20,
            width: 30,
            height: 12,
            source: crate::index::Source::Ocr,
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
