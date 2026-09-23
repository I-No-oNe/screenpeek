//! Windows on the desktop: where each one is, which has focus, and raising
//! one. Every desktop answers differently, so each has its own file:
//!
//! | Desktop | File |
//! | --- | --- |
//! | Hyprland | `hyprland.rs`, over its IPC socket |
//! | Sway | `sway.rs`, over the i3 IPC socket |
//! | GNOME, KDE Plasma | `helper.rs`, through the GNOME extension or a KWin script |
//! | X11 | `x11.rs`, through EWMH properties |
//! | Windows | `win32.rs`, through UI Automation |

#[cfg(target_os = "linux")]
use anyhow::Context;
use anyhow::Result;

use crate::capture::Region;

#[cfg(target_os = "linux")]
pub mod helper;
#[cfg(target_os = "linux")]
mod hyprland;
#[cfg(target_os = "linux")]
mod sway;
#[cfg(windows)]
mod win32;
#[cfg(target_os = "linux")]
mod x11;

/// A visible window as the compositor reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
    pub pid: Option<u32>,
    /// The compositor's own name for the window, used to focus it.
    pub handle: Option<String>,
    /// Position from the back, when the desktop reports stacking order.
    pub stack: Option<u32>,
}

impl Placement {
    pub fn rect(&self) -> Region {
        Region {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

/// Visible windows, or none when the desktop cannot be asked.
pub fn placements() -> Vec<Placement> {
    list().unwrap_or_default()
}

/// Every visible window, or an error when nothing can be asked.
#[cfg(target_os = "linux")]
pub fn list() -> Result<Vec<Placement>> {
    if let Ok(socket) = hyprland::socket() {
        return hyprland::windows(&socket);
    }
    if let Some(socket) = std::env::var_os("SWAYSOCK") {
        return sway::windows(std::path::Path::new(&socket));
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return helper::windows();
    }
    x11::windows()
}

#[cfg(windows)]
pub fn list() -> Result<Vec<Placement>> {
    win32::windows()
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn list() -> Result<Vec<Placement>> {
    anyhow::bail!("this desktop does not report its windows yet")
}

/// Raise and focus a window listed by `list`.
#[cfg(target_os = "linux")]
pub fn focus(window: &Placement) -> Result<()> {
    let handle = window
        .handle
        .as_deref()
        .context("the compositor gave this window no handle")?;
    if let Ok(socket) = hyprland::socket() {
        return hyprland::focus(&socket, handle);
    }
    if let Some(socket) = std::env::var_os("SWAYSOCK") {
        return sway::focus(std::path::Path::new(&socket), handle);
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return helper::focus(handle);
    }
    x11::focus(handle)
}

#[cfg(windows)]
pub fn focus(window: &Placement) -> Result<()> {
    use anyhow::Context;
    win32::focus(
        window
            .handle
            .as_deref()
            .context("the window has no handle")?,
    )
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn focus(_window: &Placement) -> Result<()> {
    anyhow::bail!("focusing windows is not supported on this platform yet")
}
