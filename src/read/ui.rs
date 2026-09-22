//! Read Windows UI Automation labels and rectangles before falling back to OCR.

use anyhow::{anyhow, Context, Result};
use uiautomation::core::UICacheRequest;
use uiautomation::types::{ControlType, Handle, TreeScope, UIProperty};
use uiautomation::variants::Variant;
use uiautomation::{UIAutomation, UIElement};

use crate::capture::Region;
use crate::index::{self, Element};
use crate::read::Placement;

fn automation() -> Result<UIAutomation> {
    UIAutomation::new().map_err(|error| anyhow!("{error}"))
}

/// Ask for these properties with the search, so reading them afterwards
/// needs no further calls into each application.
fn cache(automation: &UIAutomation, properties: &[UIProperty]) -> Result<UICacheRequest> {
    let request = automation
        .create_cache_request()
        .map_err(|error| anyhow!("{error}"))?;
    for property in properties {
        request
            .add_property(*property)
            .map_err(|error| anyhow!("{error}"))?;
    }
    Ok(request)
}

/// Every named element that is visible: fetched window by window with one
/// cached query each, dropping what a window in front covers. The desktop
/// lists its windows front to back.
pub fn elements() -> Result<Vec<Element>> {
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
            UIProperty::ControlType,
            UIProperty::IsEnabled,
            UIProperty::HasKeyboardFocus,
            UIProperty::ToggleToggleState,
            UIProperty::SelectionItemIsSelected,
            UIProperty::ExpandCollapseExpandCollapseState,
        ],
    )?;
    let windows = root
        .find_all_build_cache(TreeScope::Children, &onscreen, &request)
        .map_err(|error| anyhow!("{error}"))?;

    let mut in_front: Vec<Region> = Vec::new();
    let mut elements = Vec::new();
    for window in &windows {
        let Ok(rect) = window.get_cached_bounding_rectangle() else {
            continue;
        };
        let own = Region {
            x: rect.get_left(),
            y: rect.get_top(),
            width: rect.get_width().max(0) as u32,
            height: rect.get_height().max(0) as u32,
        };
        let visible = |element: &Element| {
            own.contains(element.x, element.y)
                && !in_front
                    .iter()
                    .any(|front| front.contains(element.x, element.y))
        };
        elements.extend(readable(window).filter(|element| visible(element)));
        if let Ok(found) = window.find_all_build_cache(TreeScope::Descendants, &onscreen, &request)
        {
            elements.extend(
                found
                    .iter()
                    .filter_map(readable)
                    .filter(|element| visible(element)),
            );
        }
        in_front.push(own);
    }
    index::number(&mut elements);
    Ok(elements)
}

fn readable(element: &UIElement) -> Option<Element> {
    let text = element.get_cached_name().ok()?.trim().to_owned();
    if text.is_empty() {
        return None;
    }
    let rect = element.get_cached_bounding_rectangle().ok()?;
    if rect.get_width() <= 0 || rect.get_height() <= 0 {
        return None;
    }

    let flag = |property| -> Option<bool> {
        element
            .get_cached_property_value(property)
            .ok()?
            .try_into()
            .ok()
    };
    let number = |property| -> Option<i32> {
        element
            .get_cached_property_value(property)
            .ok()?
            .try_into()
            .ok()
    };
    let mut states = Vec::new();
    if number(UIProperty::ToggleToggleState) == Some(1) {
        states.push("checked".to_owned());
    }
    if flag(UIProperty::HasKeyboardFocus) == Some(true) {
        states.push("focused".to_owned());
    }
    if flag(UIProperty::SelectionItemIsSelected) == Some(true) {
        states.push("selected".to_owned());
    }
    // ExpandCollapseState: 1 is expanded.
    if number(UIProperty::ExpandCollapseExpandCollapseState) == Some(1) {
        states.push("expanded".to_owned());
    }
    if flag(UIProperty::IsEnabled) == Some(false) {
        states.push("disabled".to_owned());
    }

    Some(Element {
        id: 0,
        text,
        x: rect.get_left() + rect.get_width() / 2,
        y: rect.get_top() + rect.get_height() / 2,
        width: rect.get_width() as u32,
        height: rect.get_height() as u32,
        source: crate::index::Source::Tree,
        role: element
            .get_cached_control_type()
            .ok()
            .and_then(role_name)
            .map(str::to_owned),
        states,
    })
}

fn role_name(control: ControlType) -> Option<&'static str> {
    Some(match control {
        ControlType::Button => "button",
        ControlType::CheckBox => "checkbox",
        ControlType::ComboBox => "combobox",
        ControlType::Edit => "entry",
        ControlType::Hyperlink => "link",
        ControlType::Image => "image",
        ControlType::ListItem => "listitem",
        ControlType::MenuItem => "menuitem",
        ControlType::RadioButton => "radio",
        ControlType::Slider => "slider",
        ControlType::Spinner => "spinbutton",
        ControlType::TabItem => "tab",
        ControlType::Text => "label",
        ControlType::TreeItem => "treeitem",
        _ => return None,
    })
}

/// Top-level windows that are on screen, with the foreground one marked.
pub fn windows() -> Result<Vec<Placement>> {
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

pub fn focus(handle: &str) -> Result<()> {
    let handle: isize = handle.parse().context("bad window handle")?;
    automation()?
        .element_from_handle(Handle::from(handle))
        .and_then(|window| window.set_focus())
        .map_err(|error| anyhow!("cannot focus the window: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows desktop"]
    fn the_desktop_tree_reads_as_clickable_elements() {
        let elements = elements().expect("UI Automation must be available");
        assert!(!elements.is_empty(), "a desktop always has a named control");
        for element in &elements {
            assert!(!element.text.trim().is_empty());
            assert!(element.width > 0 && element.height > 0);
        }
        assert!(!windows().unwrap().is_empty());
    }
}
