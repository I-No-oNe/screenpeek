//! Where each window actually is, asked of the compositor.
//!
//! The accessibility tree knows what a window contains but not where it sits.
//! wlroots compositors keep that in their own IPC, so on Hyprland the position
//! comes from there and nothing has to be worked out from pixels.

use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
}

/// Every mapped window, or an error when the compositor does not answer.
pub fn windows() -> Result<Vec<Placement>> {
    let clients: Vec<Client> = serde_json::from_str(&ask("j/clients")?)?;
    Ok(clients
        .into_iter()
        .filter(|client| client.mapped && client.size[0] > 0 && client.size[1] > 0)
        .map(|client| Placement {
            title: client.title,
            x: client.at[0],
            y: client.at[1],
            width: client.size[0] as u32,
            height: client.size[1] as u32,
            focused: client.focus_history_id == 0,
        })
        .collect())
}

/// The window the keyboard is on.
pub fn focused() -> Result<Placement> {
    windows()?
        .into_iter()
        .find(|placement| placement.focused)
        .ok_or_else(|| anyhow!("no window has focus"))
}

#[derive(Deserialize)]
struct Client {
    title: String,
    at: [i32; 2],
    size: [i32; 2],
    mapped: bool,
    #[serde(rename = "focusHistoryID")]
    focus_history_id: i32,
}

fn ask(command: &str) -> Result<String> {
    let mut socket = UnixStream::connect(socket_path()?).context("cannot reach the compositor")?;
    socket.write_all(command.as_bytes())?;

    let mut reply = String::new();
    socket.read_to_string(&mut reply)?;
    Ok(reply)
}

fn socket_path() -> Result<PathBuf> {
    let runtime = env::var("XDG_RUNTIME_DIR").context("no XDG_RUNTIME_DIR")?;
    let instance =
        env::var("HYPRLAND_INSTANCE_SIGNATURE").context("this is not a Hyprland session")?;
    Ok(PathBuf::from(runtime)
        .join("hypr")
        .join(instance)
        .join(".socket.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clients_parse_into_placements() {
        let json = r#"[
            {"title":"Home","at":[775,12],"size":[749,840],"mapped":true,"focusHistoryID":0},
            {"title":"Terminal","at":[12,12],"size":[749,840],"mapped":true,"focusHistoryID":1},
            {"title":"Hidden","at":[0,0],"size":[0,0],"mapped":false,"focusHistoryID":2}
        ]"#;
        let clients: Vec<Client> = serde_json::from_str(json).unwrap();
        let placements: Vec<Placement> = clients
            .into_iter()
            .filter(|client| client.mapped && client.size[0] > 0)
            .map(|client| Placement {
                title: client.title,
                x: client.at[0],
                y: client.at[1],
                width: client.size[0] as u32,
                height: client.size[1] as u32,
                focused: client.focus_history_id == 0,
            })
            .collect();

        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].title, "Home");
        assert_eq!((placements[0].x, placements[0].y), (775, 12));
        assert!(placements[0].focused);
        assert!(!placements[1].focused);
    }
}
