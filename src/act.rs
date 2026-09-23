//! Acting on the screen: steps, scrolling, windows and checked clicks.

use anyhow::{Context, Result};

use crate::capture::{self, Region};
use crate::cli::Area;
use crate::index::{Element, Snapshot};
use crate::look::{resolve, scan, scan_for, wait, WAIT_TIMEOUT};
use crate::pointer::{Button, Pointer};
use crate::read;

pub(crate) fn run(steps: &[String], area: &Area) -> Result<()> {
    let mut pointer = Pointer::new()?;
    let mut snapshot: Option<Snapshot> = None;
    // A snapshot that can answer every target, scanning only when needed.
    let current = |snapshot: &mut Option<Snapshot>, targets: &[&str]| -> Result<Snapshot> {
        match snapshot.take() {
            Some(known) if targets.iter().all(|t| known.can_resolve(t)) => Ok(known),
            // Ids name elements of the last scan, so that scan still answers them.
            _ if targets.iter().all(|t| t.parse::<usize>().is_ok()) => {
                let known = resolve(targets[0], false, area)?;
                match targets.iter().all(|t| known.can_resolve(t)) {
                    true => Ok(known),
                    false => Ok(Snapshot::new(scan(area)?)),
                }
            }
            _ => scan_for(targets, area),
        }
    };

    for step in steps {
        let (verb, argument) = step.split_once(' ').unwrap_or((step.as_str(), ""));
        match verb {
            "click" | "fill" => {
                let (target, text) = match verb {
                    "fill" => argument.split_once(" with ").unwrap_or((argument, "")),
                    _ => (argument, ""),
                };
                let known = current(&mut snapshot, &[target])?;
                let element = known.find(target)?;
                pointer.click(element.x, element.y, Button::Left, 1)?;
                println!("{element}");
                if verb == "fill" {
                    pointer.wait_for_focus();
                    pointer.type_text(text)?;
                }
            }
            "wait" => {
                let known = wait(argument, WAIT_TIMEOUT, false, area)?;
                if let Some(element) = known.matches(argument).first() {
                    println!("{element}");
                }
                snapshot = Some(known);
                continue;
            }
            "scroll" => {
                let (direction, amount) = argument.split_once(' ').unwrap_or((argument, "3"));
                let amount = amount.parse().context("scroll amount must be a number")?;
                scroll(&mut pointer, direction, amount)?;
            }
            "drag" => {
                let (from, to) = argument
                    .split_once(" to ")
                    .context("drag steps look like \"drag A to B\"")?;
                let known = current(&mut snapshot, &[from, to])?;
                let (start, end) = (known.find(from)?, known.find(to)?);
                pointer.drag((start.x, start.y), (end.x, end.y))?;
                println!("{start}\n{end}");
            }
            "focus" => {
                let windows = read::placements();
                focus_window(pick_window(&windows, argument)?)?;
                pointer.wait_for_focus();
            }
            "type" => pointer.type_text(argument)?,
            "key" => pointer.press(argument)?,
            other => anyhow::bail!("unknown step {other:?} in {step:?}"),
        }
        // Anything but a wait may have changed the screen.
        snapshot = None;
    }

    Ok(())
}

pub(crate) fn describe(window: &read::Placement) -> String {
    let focused = if window.focused { " [focused]" } else { "" };
    format!(
        "{} @{},{} {}x{}{focused}",
        window.title, window.x, window.y, window.width, window.height
    )
}

/// The one window whose title matches: exactly, else containing the text.
pub(crate) fn pick_window<'a>(
    windows: &'a [read::Placement],
    title: &str,
) -> Result<&'a read::Placement> {
    if windows.is_empty() {
        anyhow::bail!("this desktop does not report its windows");
    }
    let wanted = title.to_lowercase();
    let exact: Vec<_> = windows
        .iter()
        .filter(|w| w.title.to_lowercase() == wanted)
        .collect();
    let hits = if exact.is_empty() {
        windows
            .iter()
            .filter(|w| w.title.to_lowercase().contains(&wanted))
            .collect()
    } else {
        exact
    };
    match hits.as_slice() {
        [] => anyhow::bail!("no window title contains {title:?}; see `screenpeek windows`"),
        [window] => Ok(window),
        many => anyhow::bail!(
            "{} windows match {title:?}, be more specific:\n{}",
            many.len(),
            many.iter()
                .map(|w| describe(w))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn focus_window(window: &read::Placement) -> Result<()> {
    read::geometry::focus(window)
}

#[cfg(windows)]
pub(crate) fn focus_window(window: &read::Placement) -> Result<()> {
    read::ui::focus(
        window
            .handle
            .as_deref()
            .context("the window has no handle")?,
    )
}

#[cfg(not(any(target_os = "linux", windows)))]
pub(crate) fn focus_window(_window: &read::Placement) -> Result<()> {
    anyhow::bail!("focusing windows is not supported on this platform yet")
}

pub(crate) fn scroll(pointer: &mut Pointer, direction: &str, amount: u32) -> Result<()> {
    let steps = i32::try_from(amount).context("scroll amount is too large")?;
    match direction {
        "down" => pointer.scroll(steps, false),
        "up" => pointer.scroll(-steps, false),
        "right" => pointer.scroll(steps, true),
        "left" => pointer.scroll(-steps, true),
        other => anyhow::bail!("unknown scroll direction {other:?}; use up, down, left or right"),
    }
}

/// Run an action and warn when the pixels around the target stay the same.
pub(crate) fn checked(
    pointer: &mut Pointer,
    element: &Element,
    act: impl FnOnce(&mut Pointer) -> Result<()>,
) -> Result<()> {
    let near = || {
        let around = Region {
            x: element.x - 200,
            y: element.y - 100,
            width: 400,
            height: 200,
        };
        capture::screen(None, Some(around))
            .or_else(|_| {
                capture::screen(
                    None,
                    Some(Region {
                        x: element.x,
                        y: element.y,
                        width: 200,
                        height: 100,
                    }),
                )
            })
            .ok()
    };
    let before = near();
    act(pointer)?;
    std::thread::sleep(std::time::Duration::from_millis(250));
    match (before, near()) {
        (Some(before), Some(after)) if before.image == after.image => {
            eprintln!(
                "screenpeek: nothing changed near {:?} after the action",
                element.text
            )
        }
        (None, _) | (_, None) => eprintln!("screenpeek: cannot capture to check the action"),
        _ => {}
    }
    Ok(())
}
