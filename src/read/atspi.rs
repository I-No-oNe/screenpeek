//! Read AT-SPI labels and window-relative rectangles.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use rayon::prelude::*;
use zbus::blocking::connection::Builder;
use zbus::blocking::{Connection, Proxy};
use zbus::names::BusName;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

const REGISTRY: &str = "org.a11y.atspi.Registry";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";

/// Window-relative coordinates, as the toolkit reports them.
const COORDS_WINDOW: u32 = 1;

/// Walking is breadth-first and bounded: a deep tree is not worth a slow scan.
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
    for (name, path) in children(&connection, REGISTRY, ROOT).unwrap_or_default() {
        for (window_name, window_path) in
            children(&connection, name.as_str(), path.as_str()).unwrap_or_default()
        {
            if let Some(frame) = read_frame(&connection, window_name.as_str(), &window_path) {
                frames.push((window_name, window_path, frame));
            }
        }
    }

    // Each window is a long conversation with its own application, so they are
    // walked at the same time rather than one after another.
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
    let proxy = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")?;
    Ok(proxy.call("GetAddress", &())?)
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
        for (_, child) in children(connection, name, current.as_str()).unwrap_or_default() {
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

fn children(
    connection: &Connection,
    name: &str,
    path: &str,
) -> Result<Vec<(String, OwnedObjectPath)>> {
    let proxy = proxy(connection, name, path, ACCESSIBLE)?;
    Ok(proxy.call("GetChildren", &())?)
}

fn text(connection: &Connection, name: &str, path: &OwnedObjectPath) -> Option<String> {
    let proxy = proxy(connection, name, path.as_str(), ACCESSIBLE).ok()?;
    let name: String = proxy.get_property("Name").ok()?;
    Some(name.trim().to_owned())
}

fn extents(
    connection: &Connection,
    name: &str,
    path: &OwnedObjectPath,
) -> Option<(i32, i32, u32, u32)> {
    let proxy = proxy(connection, name, path.as_str(), COMPONENT).ok()?;
    let (x, y, width, height): (i32, i32, i32, i32) =
        proxy.call("GetExtents", &COORDS_WINDOW).ok()?;
    if width <= 0 || height <= 0 {
        return None;
    }
    Some((x, y, width as u32, height as u32))
}

fn proxy<'a>(
    connection: &'a Connection,
    name: &str,
    path: &str,
    interface: &str,
) -> Result<Proxy<'a>> {
    let name = BusName::try_from(name.to_owned())?;
    let path = ObjectPath::try_from(path.to_owned())?;
    Ok(Proxy::new(connection, name, path, interface.to_owned())?)
}
