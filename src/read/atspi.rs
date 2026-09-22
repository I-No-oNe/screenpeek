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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
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

    let mut frames = Vec::new();
    for (name, path) in children(&connection, REGISTRY, ROOT) {
        for (window_name, window_path) in children(&connection, name.as_str(), path.as_str()) {
            if let Some(frame) = read_frame(&connection, window_name.as_str(), &window_path) {
                frames.push((window_name, window_path, frame));
            }
        }
    }

    // Walk windows in parallel; each talks to its own application.
    Ok(frames
        .into_par_iter()
        .map(|(name, path, window)| {
            if !wanted(&window) {
                return window;
            }
            match read_items(&connection, name.as_str(), &path) {
                Some(items) => Window { items, ..window },
                None => window,
            }
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

fn read_items(connection: &Connection, name: &str, path: &OwnedObjectPath) -> Option<Vec<Item>> {
    let mut items = Vec::new();
    let mut queue = vec![path.clone()];
    let started = Instant::now();
    let mut seen = 0;

    while let Some(current) = queue.pop() {
        seen += 1;
        if seen > MAX_NODES || started.elapsed() > MAX_TIME {
            break;
        }

        if let Some(item) = read_item(connection, name, &current) {
            items.push(item);
        }
        for (_, child) in children(connection, name, current.as_str()) {
            queue.push(child);
        }
    }

    items.sort_by_key(|item| (item.y, item.x, item.text.clone()));
    items.dedup_by(|a, b| a.text == b.text && a.x == b.x && a.y == b.y);
    Some(items)
}

fn read_item(connection: &Connection, name: &str, path: &OwnedObjectPath) -> Option<Item> {
    let text = text(connection, name, path)?;
    if text.is_empty() {
        return None;
    }

    let (x, y, width, height) = extents(connection, name, path)?;
    if width == 0 || height == 0 {
        return None;
    }

    Some(Item {
        text,
        x,
        y,
        width,
        height,
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
