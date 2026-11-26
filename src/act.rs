//! Acting on the screen: steps, scrolling, windows and checked clicks.

use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::capture::{self, Region, Watch};
use crate::cli::Area;
use crate::desktop;
use crate::index::{Element, Snapshot};
use crate::look::{resolve, scan, scan_for, wait, WAIT_TIMEOUT};
use crate::pointer::{Button, Pointer};

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
                if verb == "fill" {
                    fill(&mut pointer, element, text)?;
                } else {
                    pointer.click(element.x, element.y, Button::Left, 1)?;
                }
                out!("{element}");
            }
            "wait" => {
                let known = wait(argument, WAIT_TIMEOUT, false, area)?;
                if let Some(element) = known.matches(argument).first() {
                    out!("{element}");
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
                out!("{start}\n{end}");
            }
            "focus" => {
                let windows = desktop::placements();
                let window = pick_window(&windows, argument)?;
                if !window.focused {
                    desktop::focus(window)?;
                    await_focus(window);
                }
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

pub(crate) fn describe(window: &desktop::Placement) -> String {
    let focused = if window.focused { " [focused]" } else { "" };
    format!(
        "{} @{},{} {}x{}{focused}",
        window.title, window.x, window.y, window.width, window.height
    )
}

/// The one window whose title matches: exactly, else containing the text.
pub(crate) fn pick_window<'a>(
    windows: &'a [desktop::Placement],
    title: &str,
) -> Result<&'a desktop::Placement> {
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

/// Longest wait for a clicked widget or window to take focus before typing.
const FOCUS_DELAY: Duration = Duration::from_millis(120);

/// Longest wait for a checked action to show on screen.
const CHECK_DELAY: Duration = Duration::from_millis(250);

/// Click a field, wait for it to take focus, and type into it.
pub(crate) fn fill(pointer: &mut Pointer, element: &Element, text: &str) -> Result<()> {
    settled(&near(element), FOCUS_DELAY, || {
        pointer.click(element.x, element.y, Button::Left, 1)
    })?;
    pointer.type_text(text)
}

/// Around the target, or beside it when the target is near the desktop's edge.
fn near(element: &Element) -> [Region; 2] {
    [
        Region {
            x: element.x - 200,
            y: element.y - 100,
            width: 400,
            height: 200,
        },
        Region {
            x: element.x,
            y: element.y,
            width: 200,
            height: 100,
        },
    ]
}

/// Wait until the desktop reports `window` focused, plus a frame for it to repaint.
fn await_focus(window: &desktop::Placement) {
    let started = Instant::now();
    while started.elapsed() < FOCUS_DELAY {
        let windows = desktop::placements();
        if windows
            .iter()
            .any(|w| w.focused && w.handle == window.handle)
        {
            sleep(capture::STILL.min(FOCUS_DELAY.saturating_sub(started.elapsed())));
            return;
        }
        sleep(capture::POLL);
    }
}

/// Act, then wait until the first capturable area holds still, at most `max`.
/// Gives the area's fingerprint before and after, when it could be captured.
fn settled(
    areas: &[Region],
    max: Duration,
    act: impl FnOnce() -> Result<()>,
) -> Result<Option<(u64, u64)>> {
    let watch = areas.iter().find_map(|area| Watch::new(*area));
    act()?;
    let Some((mut watch, before)) = watch else {
        sleep(max);
        return Ok(None);
    };
    Ok(watch.settle(before, max).map(|after| (before, after)))
}

/// Run an action and warn when the pixels around the target stay the same.
pub(crate) fn checked(
    pointer: &mut Pointer,
    element: &Element,
    act: impl FnOnce(&mut Pointer) -> Result<()>,
) -> Result<()> {
    match settled(&near(element), CHECK_DELAY, || act(pointer))? {
        Some((before, after)) if before == after => eprintln!(
            "screenpeek: nothing changed near {:?} after the action",
            element.text
        ),
        None => eprintln!("screenpeek: cannot capture to check the action"),
        _ => {}
    }
    Ok(())
}
