//! Query visible window geometry from the compositor or X11.

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

use super::Placement;

const I3_MAGIC: &[u8; 6] = b"i3-ipc";

const I3_GET_TREE: u32 = 4;
const I3_RUN_COMMAND: u32 = 0;

/// Every mapped window, or an error when nothing can be asked.
pub fn windows() -> Result<Vec<Placement>> {
    if let Ok(socket) = hyprland_socket() {
        return hyprland(&socket);
    }
    if let Some(socket) = env::var_os("SWAYSOCK") {
        return sway(Path::new(&socket));
    }
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        return super::geometry_helper::windows();
    }
    x11()
}

/// Raise and focus a window listed by `windows`.
pub fn focus(window: &Placement) -> Result<()> {
    let handle = window
        .handle
        .as_deref()
        .context("the compositor gave this window no handle")?;
    if let Ok(socket) = hyprland_socket() {
        // Classic configs take the dispatcher by name; Lua configs by call.
        let classic = hyprland_ask(&socket, &format!("dispatch focuswindow address:{handle}"))?;
        if classic.trim() == "ok" {
            return Ok(());
        }
        let lua = format!("dispatch hl.dsp.focus({{ window = \"address:{handle}\" }})");
        let reply = hyprland_ask(&socket, &lua)?;
        return match reply.trim() {
            "ok" => Ok(()),
            _ => bail!("Hyprland refused to focus the window: {}", classic.trim()),
        };
    }
    if let Some(socket) = env::var_os("SWAYSOCK") {
        let reply = i3_request(
            Path::new(&socket),
            I3_RUN_COMMAND,
            &format!("[con_id={handle}] focus"),
        )?;
        return match reply.contains("\"success\":true") {
            true => Ok(()),
            false => bail!("Sway refused to focus the window: {reply}"),
        };
    }
    if env::var_os("WAYLAND_DISPLAY").is_some() {
        return super::geometry_helper::focus(handle);
    }
    x11_focus(handle.parse().context("bad X11 window id")?)
}

/// Ask the window manager to activate a window, as a taskbar would.
fn x11_focus(window: u32) -> Result<()> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};

    let (connection, preferred) = x11rb::connect(None).context("no X display")?;
    let root = connection
        .setup()
        .roots
        .get(preferred)
        .ok_or_else(|| anyhow!("no screen {preferred}"))?
        .root;
    let active = connection
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
        .reply()?
        .atom;
    // Source 2 is a pager, which window managers always obey.
    let event = ClientMessageEvent::new(32, window, active, [2, 0, 0, 0, 0]);
    connection.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
        event,
    )?;
    connection.flush()?;
    Ok(())
}

/// EWMH root properties, set by every X11 window manager.
fn x11() -> Result<Vec<Placement>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, MapState};

    let (connection, preferred) = x11rb::connect(None).context("no X display")?;
    let root = connection
        .setup()
        .roots
        .get(preferred)
        .ok_or_else(|| anyhow!("no screen {preferred}"))?
        .root;

    let atom = |name: &str| -> Result<u32> {
        Ok(connection
            .intern_atom(false, name.as_bytes())?
            .reply()?
            .atom)
    };
    let client_list = atom("_NET_CLIENT_LIST")?;
    let active_window = atom("_NET_ACTIVE_WINDOW")?;
    let net_name = atom("_NET_WM_NAME")?;
    let utf8 = atom("UTF8_STRING")?;
    let process_id = atom("_NET_WM_PID")?;
    let window_desktop = atom("_NET_WM_DESKTOP")?;
    let current_desktop = atom("_NET_CURRENT_DESKTOP")?;

    let listed = connection
        .get_property(false, root, client_list, AtomEnum::WINDOW, 0, u32::MAX)?
        .reply()?;
    let windows: Vec<u32> = listed.value32().map(Iterator::collect).unwrap_or_default();

    let focused = connection
        .get_property(false, root, active_window, AtomEnum::WINDOW, 0, 1)?
        .reply()?
        .value32()
        .and_then(|mut values| values.next())
        .unwrap_or(0);

    // Skip windows on other desktops to avoid masking visible content.
    let showing = connection
        .get_property(false, root, current_desktop, AtomEnum::CARDINAL, 0, 1)?
        .reply()?
        .value32()
        .and_then(|mut values| values.next());

    let mut placements = Vec::new();
    for window in windows {
        // A window may close while it is listed; skip it rather than fail them all.
        let Ok(attributes) = connection.get_window_attributes(window)?.reply() else {
            continue;
        };
        if attributes.map_state != MapState::VIEWABLE {
            continue;
        }

        if let Some(showing) = showing {
            let desktop = connection
                .get_property(false, window, window_desktop, AtomEnum::CARDINAL, 0, 1)?
                .reply()?
                .value32()
                .and_then(|mut values| values.next());
            // 0xFFFFFFFF means the window is on every desktop.
            if matches!(desktop, Some(desktop) if desktop != showing && desktop != u32::MAX) {
                continue;
            }
        }

        let (Ok(geometry), Ok(position)) = (
            connection.get_geometry(window)?.reply(),
            connection
                .translate_coordinates(window, root, 0, 0)?
                .reply(),
        ) else {
            continue;
        };

        let title = text_property(&connection, window, net_name, utf8)?
            .or(text_property(
                &connection,
                window,
                AtomEnum::WM_NAME.into(),
                AtomEnum::STRING.into(),
            )?)
            .unwrap_or_default();

        placements.push(Placement {
            title,
            x: position.dst_x as i32,
            y: position.dst_y as i32,
            width: geometry.width as u32,
            height: geometry.height as u32,
            focused: window == focused,
            handle: Some(window.to_string()),
            stack: None,
            pid: connection
                .get_property(false, window, process_id, AtomEnum::CARDINAL, 0, 1)?
                .reply()?
                .value32()
                .and_then(|mut values| values.next()),
        });
    }

    Ok(placements)
}

fn text_property(
    connection: &impl x11rb::connection::Connection,
    window: u32,
    property: u32,
    kind: u32,
) -> Result<Option<String>> {
    use x11rb::protocol::xproto::ConnectionExt;

    let reply = connection
        .get_property(false, window, property, kind, 0, u32::MAX)?
        .reply()?;
    if reply.value.is_empty() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&reply.value).into_owned()))
}

fn hyprland(socket: &Path) -> Result<Vec<Placement>> {
    let clients: Vec<HyprlandClient> = serde_json::from_str(&hyprland_ask(socket, "j/clients")?)?;
    // Skip hidden workspaces before placing trees or excluding the caller.
    let monitors: Vec<HyprlandMonitor> =
        serde_json::from_str(&hyprland_ask(socket, "j/monitors")?).unwrap_or_default();
    let shown: Vec<i32> = monitors
        .iter()
        .map(|monitor| monitor.active_workspace.id)
        .collect();

    Ok(clients
        .into_iter()
        .filter(|client| shown.is_empty() || shown.contains(&client.workspace.id))
        .filter_map(placement_of)
        .collect())
}

fn hyprland_ask(socket: &Path, request: &str) -> Result<String> {
    let mut connection = UnixStream::connect(socket).context("cannot reach Hyprland")?;
    connection.write_all(request.as_bytes())?;
    let mut reply = String::new();
    connection.read_to_string(&mut reply)?;
    Ok(reply)
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

#[derive(Default, Deserialize)]
struct HyprlandMonitor {
    #[serde(rename = "activeWorkspace", default)]
    active_workspace: HyprlandWorkspace,
}

#[derive(Default, Deserialize)]
struct HyprlandWorkspace {
    #[serde(default)]
    id: i32,
}

#[derive(Deserialize)]
pub(super) struct HyprlandClient {
    title: String,
    at: [i32; 2],
    size: [i32; 2],
    mapped: bool,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: i32,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    workspace: HyprlandWorkspace,
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    stack: Option<u32>,
}

pub(super) fn placement_of(client: HyprlandClient) -> Option<Placement> {
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
        pid: client.pid,
        handle: client.address,
        stack: client.stack,
    })
}

fn sway(socket: &Path) -> Result<Vec<Placement>> {
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

    #[test]
    fn hyprland_clients_become_placements() {
        let json = r#"[
            {"title":"Home","at":[775,12],"size":[749,840],"mapped":true,"focusHistoryID":0,"pid":1234},
            {"title":"Terminal","at":[12,12],"size":[749,840],"mapped":true,"focusHistoryID":1},
            {"title":"Hidden","at":[0,0],"size":[0,0],"mapped":false,"focusHistoryID":2}
        ]"#;
        let clients: Vec<HyprlandClient> = serde_json::from_str(json).unwrap();
        let placements: Vec<Placement> = clients.into_iter().filter_map(placement_of).collect();

        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].title, "Home");
        assert_eq!((placements[0].x, placements[0].y), (775, 12));
        assert!(placements[0].focused);
        assert_eq!(placements[0].pid, Some(1234));
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
