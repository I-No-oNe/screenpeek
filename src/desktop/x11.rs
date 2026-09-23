//! X11: EWMH window lists and activation requests, which every window manager honours.

use anyhow::{anyhow, Context, Result};

use super::Placement;

/// Raise and focus the window with this X11 id.
pub(super) fn focus(handle: &str) -> Result<()> {
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
pub(super) fn windows() -> Result<Vec<Placement>> {
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
