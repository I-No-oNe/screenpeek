//! Windows: top-level windows and focus through UI Automation.

use anyhow::{anyhow, Context, Result};
use uiautomation::types::{Handle, TreeScope, UIProperty};
use uiautomation::variants::Variant;
use uiautomation::{UIAutomation, UIElement};

use super::Placement;
use crate::read::ui::{automation, cache};

/// Top-level windows that are on screen, with the foreground one marked.
pub(super) fn windows() -> Result<Vec<Placement>> {
    let automation = automation()?;
    let root = automation
        .get_root_element()
        .map_err(|error| anyhow!("{error}"))?;
    let onscreen = automation
        .create_property_condition(UIProperty::IsOffscreen, Variant::from(false), None)
        .map_err(|error| anyhow!("{error}"))?;
    let request = cache(
        &automation,
        &[
            UIProperty::Name,
            UIProperty::BoundingRectangle,
            UIProperty::NativeWindowHandle,
            UIProperty::ProcessId,
        ],
    )?;
    let found = root
        .find_all_build_cache(TreeScope::Children, &onscreen, &request)
        .map_err(|error| anyhow!("{error}"))?;
    let foreground = foreground(&automation, &root);

    Ok(found
        .iter()
        .filter_map(|window| {
            let rect = window.get_cached_bounding_rectangle().ok()?;
            if rect.get_width() <= 0 || rect.get_height() <= 0 {
                return None;
            }
            let handle: isize = window.get_cached_native_window_handle().ok()?.into();
            Some(Placement {
                title: window.get_cached_name().unwrap_or_default(),
                x: rect.get_left(),
                y: rect.get_top(),
                width: rect.get_width() as u32,
                height: rect.get_height() as u32,
                focused: foreground == Some(handle),
                pid: window
                    .get_cached_property_value(UIProperty::ProcessId)
                    .ok()
                    .and_then(|pid| TryInto::<i32>::try_into(pid).ok())
                    .and_then(|pid| u32::try_from(pid).ok()),
                handle: Some(handle.to_string()),
                stack: None,
            })
        })
        .collect())
}

/// The top-level window holding keyboard focus.
fn foreground(automation: &UIAutomation, root: &UIElement) -> Option<isize> {
    let walker = automation.get_control_view_walker().ok()?;
    let mut element = automation.get_focused_element().ok()?;
    for _ in 0..64 {
        let parent = walker.get_parent(&element).ok()?;
        if automation.compare_elements(&parent, root).ok()? {
            return Some(element.get_native_window_handle().ok()?.into());
        }
        element = parent;
    }
    None
}

pub(super) fn focus(handle: &str) -> Result<()> {
    let handle: isize = handle.parse().context("bad window handle")?;
    automation()?
        .element_from_handle(Handle::from(handle))
        .and_then(|window| window.set_focus())
        .map_err(|error| anyhow!("cannot focus the window: {error}"))
}
