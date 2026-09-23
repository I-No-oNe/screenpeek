//! Reading the screen: scans, and finding what a command names.

use anyhow::{Context, Result};

use crate::capture::{self, Region};
use crate::cli::Area;
use crate::index::{self, Element, Snapshot};
use crate::{caller, daemon, read};

/// How long a `wait` step in `run` waits.
pub(crate) const WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Scan until the target appears (or, with `gone`, disappears).
pub(crate) fn wait(
    target: &str,
    timeout: std::time::Duration,
    gone: bool,
    area: &Area,
) -> Result<Snapshot> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let snapshot = Snapshot::new(scan(area)?);
        if snapshot.matches(target).is_empty() == gone {
            return Ok(snapshot);
        }
        if std::time::Instant::now() >= deadline {
            match gone {
                true => anyhow::bail!("{target:?} is still on screen after {timeout:?}"),
                false => anyhow::bail!("nothing matched {target:?} within {timeout:?}"),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

pub(crate) fn scan(area: &Area) -> Result<Vec<Element>> {
    let windows = read::placements();
    let region = area.region(&windows)?;
    let language = resolve_language(area.lang.as_deref())?;
    let excluded = caller::regions(&windows);
    let mut elements = match controls(region, area.monitor) {
        Some(elements) => elements,
        None => match daemon::ask(region, area.monitor, language.clone(), excluded.clone()) {
            Some(elements) => elements,
            None => {
                // The tree walk overlaps capture and recognition.
                let (tree, recognized) = std::thread::scope(|scope| {
                    let tree = scope.spawn(tree);
                    let recognized = (|| {
                        let mut capture = capture::screen(area.monitor, region)?;
                        capture.exclude(&excluded);
                        read::Engine::load()?.read_within(
                            &capture,
                            &read::edges(&windows),
                            language.as_ref(),
                        )
                    })();
                    (tree.join().unwrap_or_default(), recognized)
                });
                with_tree_text(recognized?, tree, &windows)
            }
        },
    };
    if let Some(region) = region {
        elements.retain(|element| region.contains(element.x, element.y));
        index::number(&mut elements);
    }
    caller::filter(&mut elements, &excluded);
    if let Some(previous) = Snapshot::load() {
        index::keep_ids(&mut elements, &previous.elements);
    }
    Snapshot::new(elements.clone())
        .save()
        .context("cannot cache this scan")?;
    Ok(elements)
}

/// A snapshot that can answer `target`, scanning again only when needed.
pub(crate) fn resolve(target: &str, fresh: bool, area: &Area) -> Result<Snapshot> {
    if !fresh && area.region.is_none() && area.monitor.is_none() && !area.focused {
        if let Some(mut snapshot) = Snapshot::load() {
            caller::filter(
                &mut snapshot.elements,
                &caller::regions(&read::placements()),
            );
            if snapshot.can_resolve(target) {
                return Ok(snapshot);
            }
        }
    }

    scan_for(&[target], area)
}

/// A fresh scan for `targets`: the focused window first, several times faster
/// than the whole screen, which is read only when the window lacks a target.
pub(crate) fn scan_for(targets: &[&str], area: &Area) -> Result<Snapshot> {
    let whole = area.region.is_none() && area.monitor.is_none() && !area.focused;
    // Ids number the last whole listing, so they need the whole screen.
    let names = targets.iter().all(|t| t.parse::<usize>().is_err());
    if whole && names {
        let window = Area {
            focused: true,
            ..area.clone()
        };
        if let Ok(elements) = scan(&window) {
            let snapshot = Snapshot::new(elements);
            if targets.iter().all(|t| snapshot.can_resolve(t)) {
                return Ok(snapshot);
            }
        }
    }
    Ok(Snapshot::new(scan(area)?))
}

/// The languages to read besides the built-in reader: the ones asked for,
/// or the ones chosen at install time, re-read only when needed.
pub(crate) fn resolve_language(requested: Option<&str>) -> Result<Option<read::Language>> {
    use read::language::{configured, every, with_english};
    let auto = |codes| Some(read::Language { codes, auto: true });
    let explicit = |codes| Some(read::Language { codes, auto: false });
    Ok(match requested {
        None => configured().and_then(auto),
        Some("none") => None,
        Some("auto") => match configured() {
            Some(codes) => auto(codes),
            None => auto(every(&read::tesseract::installed()?)),
        },
        Some("all") => explicit(every(&read::tesseract::installed()?)),
        Some(code) => {
            read::tesseract::supports(code)?;
            explicit(with_english(code, &read::tesseract::installed()?))
        }
    })
}

#[cfg(target_os = "linux")]
type Tree = Vec<read::atspi::Window>;
#[cfg(not(target_os = "linux"))]
type Tree = ();

#[cfg(target_os = "linux")]
fn tree() -> Tree {
    read::atspi::windows().unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn tree() -> Tree {}

/// Place tree labels using compositor geometry or matching OCR text.
#[cfg(target_os = "linux")]
fn with_tree_text(
    elements: Vec<Element>,
    tree: Tree,
    placements: &[read::Placement],
) -> Vec<Element> {
    if tree.is_empty() {
        return elements;
    }
    if placements.is_empty() {
        return read::fuse::fuse(elements, &tree);
    }
    let located = read::fuse::place(&tree, placements)
        .into_iter()
        .flat_map(|window| window.elements)
        .collect();
    index::merge_tree(elements, located)
}

#[cfg(not(target_os = "linux"))]
fn with_tree_text(
    elements: Vec<Element>,
    _tree: Tree,
    _placements: &[read::Placement],
) -> Vec<Element> {
    elements
}

#[cfg(windows)]
fn controls(region: Option<Region>, monitor: Option<usize>) -> Option<Vec<Element>> {
    let region = match monitor {
        Some(index) => Some(capture::monitor_bounds(index).ok()?),
        None => region,
    };
    let elements = match read::ui::elements() {
        Ok(elements) if !elements.is_empty() => elements,
        Ok(_) => return None,
        Err(error) => {
            eprintln!("screenpeek: UI Automation unavailable, reading the pixels instead: {error}");
            return None;
        }
    };
    Some(match region {
        Some(region) => elements
            .into_iter()
            .filter(|element| region.contains(element.x, element.y))
            .collect(),
        None => elements,
    })
}

#[cfg(not(windows))]
fn controls(_region: Option<Region>, _monitor: Option<usize>) -> Option<Vec<Element>> {
    None
}
