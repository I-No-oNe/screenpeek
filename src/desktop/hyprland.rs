//! Hyprland: window list and focus over its IPC socket.

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use super::Placement;

/// Raise and focus a window by its Hyprland address.
pub(super) fn focus(socket: &Path, handle: &str) -> Result<()> {
    // Classic configs take the dispatcher by name; Lua configs by call.
    let classic = ask(socket, &format!("dispatch focuswindow address:{handle}"))?;
    if classic.trim() == "ok" {
        return Ok(());
    }
    let lua = format!("dispatch hl.dsp.focus({{ window = \"address:{handle}\" }})");
    let reply = ask(socket, &lua)?;
    match reply.trim() {
        "ok" => Ok(()),
        _ => bail!("Hyprland refused to focus the window: {}", classic.trim()),
    }
}

pub(super) fn windows(socket: &Path) -> Result<Vec<Placement>> {
    let clients: Vec<HyprlandClient> = serde_json::from_str(&ask(socket, "j/clients")?)?;
    // Skip hidden workspaces before placing trees or excluding the caller.
    let monitors: Vec<HyprlandMonitor> =
        serde_json::from_str(&ask(socket, "j/monitors")?).unwrap_or_default();
    let shown = shown_workspaces(&monitors);

    Ok(clients
        .into_iter()
        .filter(|client| shown.is_empty() || shown.contains(&client.workspace.id))
        .filter_map(placement_of)
        .collect())
}

/// Each monitor's workspace, plus the special one (scratchpad) it shows on top.
fn shown_workspaces(monitors: &[HyprlandMonitor]) -> Vec<i32> {
    monitors
        .iter()
        .flat_map(|monitor| [monitor.active_workspace.id, monitor.special_workspace.id])
        .filter(|id| *id != 0)
        .collect()
}

fn ask(socket: &Path, request: &str) -> Result<String> {
    let mut connection = UnixStream::connect(socket).context("cannot reach Hyprland")?;
    connection.write_all(request.as_bytes())?;
    let mut reply = String::new();
    connection.read_to_string(&mut reply)?;
    Ok(reply)
}

pub(super) fn socket() -> Result<PathBuf> {
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
    /// Id 0 when no special workspace is open.
    #[serde(rename = "specialWorkspace", default)]
    special_workspace: HyprlandWorkspace,
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

    #[test]
    fn an_open_scratchpad_counts_as_shown() {
        let monitors: Vec<HyprlandMonitor> = serde_json::from_str(
            r#"[
                {"activeWorkspace":{"id":1},"specialWorkspace":{"id":-98}},
                {"activeWorkspace":{"id":2},"specialWorkspace":{"id":0}}
            ]"#,
        )
        .unwrap();
        assert_eq!(shown_workspaces(&monitors), [1, -98, 2]);
    }
}
