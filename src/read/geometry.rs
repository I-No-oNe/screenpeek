//! Where each window actually is, asked of the compositor.
//!
//! The accessibility tree knows what a window contains but not where it sits.
//! wlroots compositors keep that in their own IPC: Hyprland answers a line of
//! JSON, Sway speaks the i3 protocol. Either way nothing has to be worked out
//! from pixels.

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

/// i3 and Sway prefix every message with this.
const I3_MAGIC: &[u8; 6] = b"i3-ipc";

/// The i3 message that asks for the window tree.
const I3_GET_TREE: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
}

/// Every mapped window, or an error when no compositor answers.
pub fn windows() -> Result<Vec<Placement>> {
    if let Ok(socket) = hyprland_socket() {
        return hyprland(&socket);
    }
    if let Some(socket) = env::var_os("SWAYSOCK") {
        return sway(Path::new(&socket));
    }
    bail!("no compositor to ask; Hyprland and Sway are supported")
}

/// The window the keyboard is on.
pub fn focused() -> Result<Placement> {
    windows()?
        .into_iter()
        .find(|placement| placement.focused)
        .ok_or_else(|| anyhow!("no window has focus"))
}

fn hyprland(socket: &Path) -> Result<Vec<Placement>> {
    let mut connection = UnixStream::connect(socket).context("cannot reach Hyprland")?;
    connection.write_all(b"j/clients")?;

    let mut reply = String::new();
    connection.read_to_string(&mut reply)?;

    let clients: Vec<HyprlandClient> = serde_json::from_str(&reply)?;
    Ok(clients.into_iter().filter_map(placement_of).collect())
}

fn hyprland_socket() -> Result<PathBuf> {
    let runtime = env::var("XDG_RUNTIME_DIR").context("no XDG_RUNTIME_DIR")?;
    let instance =
        env::var("HYPRLAND_INSTANCE_SIGNATURE").context("this is not a Hyprland session")?;
    Ok(PathBuf::from(runtime)
        .join("hypr")
        .join(instance)
        .join(".socket.sock"))
}

#[derive(Deserialize)]
struct HyprlandClient {
    title: String,
    at: [i32; 2],
    size: [i32; 2],
    mapped: bool,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: i32,
}

fn placement_of(client: HyprlandClient) -> Option<Placement> {
    if !client.mapped || client.size[0] <= 0 || client.size[1] <= 0 {
        return None;
    }
    Some(Placement {
        title: client.title,
        x: client.at[0],
        y: client.at[1],
        width: client.size[0] as u32,
        height: client.size[1] as u32,
        focused: client.focus_history_id == 0,
    })
}

fn sway(socket: &Path) -> Result<Vec<Placement>> {
    let tree: SwayNode = serde_json::from_str(&i3_request(socket, I3_GET_TREE)?)?;
    let mut placements = Vec::new();
    collect_sway(&tree, &mut placements);
    Ok(placements)
}

/// One request and one reply over the i3 protocol: the magic string, the
/// payload length and the message type, all in native byte order.
fn i3_request(socket: &Path, message: u32) -> Result<String> {
    let mut connection = UnixStream::connect(socket).context("cannot reach Sway")?;

    let mut request = Vec::with_capacity(14);
    request.extend_from_slice(I3_MAGIC);
    request.extend_from_slice(&0u32.to_ne_bytes());
    request.extend_from_slice(&message.to_ne_bytes());
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

/// Sway reports a container rectangle and, inside it, where the client's own
/// surface sits. The surface is what the accessibility tree describes, so that
/// is what gets used.
#[derive(Deserialize)]
struct SwayNode {
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hyprland_clients_become_placements() {
        let json = r#"[
            {"title":"Home","at":[775,12],"size":[749,840],"mapped":true,"focusHistoryID":0},
            {"title":"Terminal","at":[12,12],"size":[749,840],"mapped":true,"focusHistoryID":1},
            {"title":"Hidden","at":[0,0],"size":[0,0],"mapped":false,"focusHistoryID":2}
        ]"#;
        let clients: Vec<HyprlandClient> = serde_json::from_str(json).unwrap();
        let placements: Vec<Placement> = clients.into_iter().filter_map(placement_of).collect();

        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].title, "Home");
        assert_eq!((placements[0].x, placements[0].y), (775, 12));
        assert!(placements[0].focused);
        assert!(!placements[1].focused);
    }

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
