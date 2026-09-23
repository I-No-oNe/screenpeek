//! Sway: window tree and focus over the i3 IPC socket.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::Placement;

const I3_MAGIC: &[u8; 6] = b"i3-ipc";

const I3_GET_TREE: u32 = 4;
const I3_RUN_COMMAND: u32 = 0;

/// Raise and focus a window by its Sway container id.
pub(super) fn focus(socket: &Path, handle: &str) -> Result<()> {
    let reply = i3_request(socket, I3_RUN_COMMAND, &format!("[con_id={handle}] focus"))?;
    match reply.contains("\"success\":true") {
        true => Ok(()),
        false => bail!("Sway refused to focus the window: {reply}"),
    }
}

pub(super) fn windows(socket: &Path) -> Result<Vec<Placement>> {
    let tree: SwayNode = serde_json::from_str(&i3_request(socket, I3_GET_TREE, "")?)?;
    let mut placements = Vec::new();
    collect_sway(&tree, &mut placements);
    Ok(placements)
}

/// One i3 IPC request/reply.
fn i3_request(socket: &Path, message: u32, payload: &str) -> Result<String> {
    let mut connection = UnixStream::connect(socket).context("cannot reach Sway")?;

    let mut request = Vec::with_capacity(14);
    request.extend_from_slice(I3_MAGIC);
    request.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    request.extend_from_slice(&message.to_ne_bytes());
    request.extend_from_slice(payload.as_bytes());
    connection.write_all(&request)?;

    let mut header = [0u8; 14];
    connection.read_exact(&mut header)?;
    if &header[..6] != I3_MAGIC {
        bail!("the reply is not i3 protocol");
    }

    let length = u32::from_ne_bytes(header[6..10].try_into()?) as usize;
    let mut body = vec![0u8; length];
    connection.read_exact(&mut body)?;
    Ok(String::from_utf8(body)?)
}

/// Use Sway’s client-surface rectangle as the AT-SPI origin.
#[derive(Deserialize)]
struct SwayNode {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    visible: Option<bool>,
    #[serde(default)]
    focused: bool,
    rect: SwayRect,
    #[serde(default)]
    window_rect: Option<SwayRect>,
    #[serde(default)]
    nodes: Vec<SwayNode>,
    #[serde(default)]
    floating_nodes: Vec<SwayNode>,
}

#[derive(Deserialize, Clone, Copy, Default)]
struct SwayRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn collect_sway(node: &SwayNode, placements: &mut Vec<Placement>) {
    if let Some(placement) = sway_placement(node) {
        placements.push(placement);
    }
    for child in node.nodes.iter().chain(&node.floating_nodes) {
        collect_sway(child, placements);
    }
}

fn sway_placement(node: &SwayNode) -> Option<Placement> {
    node.pid?;
    if node.visible == Some(false) {
        return None;
    }

    let surface = node.window_rect.unwrap_or_default();
    let (width, height) = if surface.width > 0 && surface.height > 0 {
        (surface.width, surface.height)
    } else {
        (node.rect.width, node.rect.height)
    };
    if width <= 0 || height <= 0 {
        return None;
    }

    Some(Placement {
        title: node.name.clone().unwrap_or_default(),
        x: node.rect.x + surface.x,
        y: node.rect.y + surface.y,
        width: width as u32,
        height: height as u32,
        focused: node.focused,
        pid: node.pid,
        handle: node.id.map(|id| id.to_string()),
        stack: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `swaymsg -t get_tree`: a workspace holding one
    /// tiled window and one floating one, plus a container with no surface.
    const SWAY_TREE: &str = r#"{
        "name": "root",
        "rect": {"x":0,"y":0,"width":1920,"height":1080},
        "nodes": [{
            "name": "HDMI-A-1",
            "rect": {"x":0,"y":0,"width":1920,"height":1080},
            "nodes": [{
                "name": "1",
                "rect": {"x":0,"y":0,"width":1920,"height":1080},
                "nodes": [{
                    "name": "Home",
                    "pid": 4242,
                    "visible": true,
                    "focused": true,
                    "rect": {"x":8,"y":8,"width":952,"height":1064},
                    "window_rect": {"x":2,"y":24,"width":948,"height":1038},
                    "nodes": []
                }],
                "floating_nodes": [{
                    "name": "Preferences",
                    "pid": 4343,
                    "visible": true,
                    "focused": false,
                    "rect": {"x":500,"y":300,"width":600,"height":400},
                    "window_rect": {"x":0,"y":0,"width":600,"height":400},
                    "nodes": []
                }]
            }]
        }]
    }"#;

    #[test]
    fn sway_views_become_placements_at_their_surface() {
        let tree: SwayNode = serde_json::from_str(SWAY_TREE).unwrap();
        let mut placements = Vec::new();
        collect_sway(&tree, &mut placements);

        assert_eq!(placements.len(), 2, "{placements:?}");

        let home = &placements[0];
        assert_eq!(home.title, "Home");
        assert_eq!(home.pid, Some(4242));
        assert_eq!((home.x, home.y), (10, 32));
        assert_eq!((home.width, home.height), (948, 1038));
        assert!(home.focused);

        let floating = &placements[1];
        assert_eq!(floating.title, "Preferences");
        assert_eq!((floating.x, floating.y), (500, 300));
        assert!(!floating.focused);
    }

    #[test]
    fn sway_containers_without_a_surface_are_skipped() {
        let tree: SwayNode = serde_json::from_str(SWAY_TREE).unwrap();
        assert!(sway_placement(&tree).is_none(), "the root is not a window");
        assert!(sway_placement(&tree.nodes[0]).is_none(), "nor an output");
    }

    #[test]
    fn a_hidden_sway_view_is_skipped() {
        let node: SwayNode = serde_json::from_str(
            r#"{"name":"Background","pid":1,"visible":false,
                "rect":{"x":0,"y":0,"width":800,"height":600},
                "window_rect":{"x":0,"y":0,"width":800,"height":600}}"#,
        )
        .unwrap();
        assert!(sway_placement(&node).is_none());
    }
}
