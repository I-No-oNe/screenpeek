//! Read AT-SPI labels and window-relative rectangles.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rayon::prelude::*;
use zbus::blocking::connection::Builder;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";

/// Window-relative coordinates, as the toolkit reports them.
const COORDS_WINDOW: u32 = 1;

/// Bound each walk: a deep tree is not worth a slow scan.
const MAX_NODES: usize = 1500;
const MAX_TIME: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub role: Option<&'static str>,
    pub states: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub items: Vec<Item>,
}

/// Every window the accessibility bus is willing to describe.
pub fn windows() -> Result<Vec<Window>> {
    windows_where(|_| true)
}

/// Return window metadata for skipped trees so cached items can be reused.
pub fn windows_where(wanted: impl Fn(&Window) -> bool + Sync) -> Result<Vec<Window>> {
    let bus = address().context("no accessibility bus")?;
    let connection = Builder::address(bus.as_str())?.build()?;

    let frames: Vec<_> = children(&connection, REGISTRY, ROOT)
        .into_par_iter()
        .flat_map_iter(|(name, path)| children(&connection, name.as_str(), path.as_str()))
        .filter_map(|(name, path)| {
            let frame = read_frame(&connection, name.as_str(), &path)?;
            Some((name, path, frame))
        })
        .collect();

    Ok(frames
        .into_par_iter()
        .map(|(name, path, window)| {
            if !wanted(&window) {
                return window;
            }
            let items = read_items(&connection, name.as_str(), &path);
            Window { items, ..window }
        })
        .collect())
}

fn address() -> Result<String> {
    let session = Connection::session()?;
    call(
        &session,
        "org.a11y.Bus",
        "/org/a11y/bus",
        "org.a11y.Bus",
        "GetAddress",
        &(),
    )
    .context("the accessibility bus did not answer")
}

/// A window's title and size, without walking what is inside it.
fn read_frame(connection: &Connection, name: &str, path: &OwnedObjectPath) -> Option<Window> {
    let (_, _, width, height) = extents(connection, name, path)?;
    if width == 0 || height == 0 {
        return None;
    }

    Some(Window {
        title: text(connection, name, path).unwrap_or_default(),
        width,
        height,
        items: Vec::new(),
    })
}

/// Walk a window level by level, reading each level in parallel. Subtrees
/// that are not showing (inactive tabs, collapsed menus) are skipped.
fn read_items(connection: &Connection, name: &str, root: &OwnedObjectPath) -> Vec<Item> {
    let started = Instant::now();
    let mut items = Vec::new();
    let mut level = vec![root.clone()];
    let mut seen = 0;

    while !level.is_empty() && seen < MAX_NODES && started.elapsed() < MAX_TIME {
        level.truncate(MAX_NODES - seen);
        seen += level.len();
        let visited: Vec<_> = level
            .par_iter()
            .map(|path| visit(connection, name, path))
            .collect();
        level = Vec::new();
        for (item, children) in visited {
            items.extend(item);
            level.extend(children);
        }
    }

    items.sort_by(|a, b| (a.y, a.x, &a.text).cmp(&(b.y, b.x, &b.text)));
    items.dedup_by(|a, b| a.text == b.text && a.x == b.x && a.y == b.y);
    items
}

/// One node: its item when it is named and showing, and its children.
fn visit(
    connection: &Connection,
    name: &str,
    path: &OwnedObjectPath,
) -> (Option<Item>, Vec<OwnedObjectPath>) {
    let states = call::<Vec<u32>>(connection, name, path.as_str(), ACCESSIBLE, "GetState", &())
        .map(|words| states_of(&words));
    if states.as_ref().is_some_and(|states| !states.showing) {
        return (None, Vec::new());
    }
    let (text, children) = rayon::join(
        || text(connection, name, path),
        || children(connection, name, path.as_str()),
    );
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return (None, children.into_iter().map(|(_, child)| child).collect());
    };
    let (extents, role) = rayon::join(
        || extents(connection, name, path),
        || call::<u32>(connection, name, path.as_str(), ACCESSIBLE, "GetRole", &()),
    );
    let item = extents.map(|(x, y, width, height)| Item {
        text,
        x,
        y,
        width,
        height,
        role: role.and_then(role_name),
        states: states.map(|states| states.notable).unwrap_or_default(),
    });
    (item, children.into_iter().map(|(_, child)| child).collect())
}

struct States {
    showing: bool,
    notable: Vec<&'static str>,
}

/// AT-SPI state bits (AtspiStateType) that matter to someone clicking.
fn states_of(words: &[u32]) -> States {
    let has = |bit: u32| words.first().is_some_and(|word| word & (1 << bit) != 0);
    let mut notable = Vec::new();
    for (bit, name) in [
        (4, "checked"),
        (12, "focused"),
        (23, "selected"),
        (10, "expanded"),
    ] {
        if has(bit) {
            notable.push(name);
        }
    }
    if !has(24) {
        notable.push("disabled");
    }
    States {
        showing: has(25),
        notable,
    }
}

/// Short names for the AT-SPI roles (AtspiRole) worth telling apart.
fn role_name(role: u32) -> Option<&'static str> {
    Some(match role {
        7 | 8 => "checkbox",
        11 => "combobox",
        16 => "dialog",
        26 | 27 => "image",
        29 => "label",
        32 => "listitem",
        33 => "menu",
        35 => "menuitem",
        37 => "tab",
        40 => "password",
        43 => "button",
        44 | 45 => "radio",
        51 => "slider",
        52 => "spinbutton",
        56 => "cell",
        61 | 79 => "entry",
        62 => "toggle",
        83 => "heading",
        88 => "link",
        91 => "treeitem",
        130 => "switch",
        _ => return None,
    })
}

/// One method call, skipping the proxy layer: zbus proxies fetch and watch
/// every property, which costs two extra round trips per node.
fn call<R>(
    connection: &Connection,
    name: &str,
    path: &str,
    interface: &str,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> Option<R>
where
    R: serde::de::DeserializeOwned + zbus::zvariant::Type,
{
    connection
        .call_method(Some(name), path, Some(interface), method, body)
        .ok()?
        .body()
        .deserialize()
        .ok()
}

fn children(connection: &Connection, name: &str, path: &str) -> Vec<(String, OwnedObjectPath)> {
    call(connection, name, path, ACCESSIBLE, "GetChildren", &()).unwrap_or_default()
}

fn text(connection: &Connection, name: &str, path: &OwnedObjectPath) -> Option<String> {
    let value: OwnedValue = call(
        connection,
        name,
        path.as_str(),
        "org.freedesktop.DBus.Properties",
        "Get",
        &(ACCESSIBLE, "Name"),
    )?;
    Some(String::try_from(value).ok()?.trim().to_owned())
}

fn extents(
    connection: &Connection,
    name: &str,
    path: &OwnedObjectPath,
) -> Option<(i32, i32, u32, u32)> {
    let (x, y, width, height): (i32, i32, i32, i32) = call(
        connection,
        name,
        path.as_str(),
        COMPONENT,
        "GetExtents",
        &COORDS_WINDOW,
    )?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some((x, y, width as u32, height as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_bits_become_words() {
        let showing_checked = (1 << 25) | (1 << 24) | (1 << 4);
        let states = states_of(&[showing_checked, 0]);
        assert!(states.showing);
        assert_eq!(states.notable, ["checked"]);
        let hidden_disabled = states_of(&[0, 0]);
        assert!(!hidden_disabled.showing);
        assert_eq!(hidden_disabled.notable, ["disabled"]);
        assert_eq!(role_name(43), Some("button"));
        assert_eq!(role_name(20), None);
    }
}
