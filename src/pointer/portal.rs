//! Mouse and keyboard through the consent-based RemoteDesktop portal.

use super::{Button, Key};
use crate::portal::{self, Values, PATH, SERVICE};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, Value};

const INTERFACE: &str = "org.freedesktop.portal.RemoteDesktop";
type Options<'a> = HashMap<&'a str, Value<'a>>;

pub struct Remote {
    connection: Connection,
    session: OwnedObjectPath,
    streams: Vec<(u32, Values)>,
}

impl Remote {
    pub fn new() -> Result<Self> {
        let connection = Connection::session()?;
        let created = portal::request(&connection, INTERFACE, "CreateSession", &(Options::new(),))?;
        let session: String = created
            .get("session_handle")
            .context("portal returned no session")?
            .try_clone()?
            .try_into()?;
        let mut remote = Self {
            connection,
            session: session.try_into()?,
            streams: Vec::new(),
        };
        portal::request(
            &remote.connection,
            INTERFACE,
            "SelectDevices",
            &(
                &remote.session,
                Options::from([("types", Value::from(3u32))]),
            ),
        )?;
        portal::request(
            &remote.connection,
            "org.freedesktop.portal.ScreenCast",
            "SelectSources",
            &(
                &remote.session,
                Options::from([
                    ("types", Value::from(1u32)),
                    ("multiple", Value::from(true)),
                ]),
            ),
        )?;
        eprintln!("Allow screenpeek keyboard/pointer access and select the monitor(s) to control.");
        let mut started = portal::request(
            &remote.connection,
            INTERFACE,
            "Start",
            &(&remote.session, "", Options::new()),
        )?;
        let devices = u32::try_from(
            started
                .remove("devices")
                .context("portal returned no devices")?,
        )?;
        if devices & 3 != 3 {
            bail!("keyboard and pointer access are required");
        }
        remote.streams = started
            .remove("streams")
            .context("portal returned no monitors")?
            .try_into()?;
        if remote.streams.is_empty() {
            bail!("no monitor selected");
        }
        Ok(remote)
    }

    pub fn click(&self, x: i32, y: i32, button: Button, times: u32) -> Result<()> {
        let (stream, x, y) = locate(&self.streams, x, y)?;
        self.connection.call_method(
            Some(SERVICE),
            PATH,
            Some(INTERFACE),
            "NotifyPointerMotionAbsolute",
            &(&self.session, Options::new(), stream, x, y),
        )?;
        let button: i32 = match button {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
        };
        for _ in 0..times {
            for state in [1u32, 0] {
                self.connection.call_method(
                    Some(SERVICE),
                    PATH,
                    Some(INTERFACE),
                    "NotifyPointerButton",
                    &(&self.session, Options::new(), button, state),
                )?;
            }
        }
        Ok(())
    }

    pub fn key(&self, key: Key, pressed: bool) -> Result<()> {
        self.connection.call_method(
            Some(SERVICE),
            PATH,
            Some(INTERFACE),
            "NotifyKeyboardKeysym",
            &(
                &self.session,
                Options::new(),
                keysym(key)?,
                u32::from(pressed),
            ),
        )?;
        Ok(())
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        let _ = self.connection.call_method(
            Some(SERVICE),
            self.session.as_str(),
            Some("org.freedesktop.portal.Session"),
            "Close",
            &(),
        );
    }
}

fn locate(streams: &[(u32, Values)], x: i32, y: i32) -> Result<(u32, f64, f64)> {
    for (id, properties) in streams {
        let pair = |key: &str| -> Option<(i32, i32)> {
            properties.get(key)?.try_clone().ok()?.try_into().ok()
        };
        // Refuse ambiguous coordinates when the portal omits monitor geometry.
        let Some((left, top)) = pair("position") else {
            continue;
        };
        let Some((width, height)) = pair("logical_size").or_else(|| pair("size")) else {
            continue;
        };
        let dx = i64::from(x) - i64::from(left);
        let dy = i64::from(y) - i64::from(top);
        if dx >= 0 && dy >= 0 && dx < i64::from(width) && dy < i64::from(height) {
            return Ok((*id, dx as f64, dy as f64));
        }
    }
    bail!("target is outside selected monitors, or the portal omitted their logical geometry")
}

fn keysym(key: Key) -> Result<i32> {
    Ok(match key {
        Key::Unicode('\n') => 0xff0d,
        Key::Unicode('\t') => 0xff09,
        Key::Unicode(c) if (c as u32) <= 0xff => c as i32,
        Key::Unicode(c) => 0x01000000 | c as i32,
        Key::Control => 0xffe3,
        Key::Alt => 0xffe9,
        Key::Shift => 0xffe1,
        Key::Meta => 0xffeb,
        Key::Return => 0xff0d,
        Key::Tab => 0xff09,
        Key::Escape => 0xff1b,
        Key::Space => 0x20,
        Key::Backspace => 0xff08,
        Key::Delete => 0xffff,
        Key::Home => 0xff50,
        Key::End => 0xff57,
        Key::PageUp => 0xff55,
        Key::PageDown => 0xff56,
        Key::LeftArrow => 0xff51,
        Key::UpArrow => 0xff52,
        Key::RightArrow => 0xff53,
        Key::DownArrow => 0xff54,
        Key::F1 => 0xffbe,
        Key::F2 => 0xffbf,
        Key::F3 => 0xffc0,
        Key::F4 => 0xffc1,
        Key::F5 => 0xffc2,
        Key::F6 => 0xffc3,
        Key::F7 => 0xffc4,
        Key::F8 => 0xffc5,
        Key::F9 => 0xffc6,
        Key::F10 => 0xffc7,
        Key::F11 => 0xffc8,
        Key::F12 => 0xffc9,
        _ => bail!("unsupported portal key: {key:?}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::OwnedValue;
    #[test]
    fn input_uses_logical_monitor_offsets_and_unicode_keysyms() {
        let streams = vec![(
            42,
            Values::from([
                (
                    "position".into(),
                    OwnedValue::try_from(Value::from((-1280i32, 0i32))).unwrap(),
                ),
                (
                    "size".into(),
                    OwnedValue::try_from(Value::from((1280i32, 720i32))).unwrap(),
                ),
            ]),
        )];
        assert_eq!(locate(&streams, -100, 20).unwrap(), (42, 1180.0, 20.0));
        assert!(locate(&streams, 0, 20).is_err());
        assert!(locate(&[(42, Values::new())], 0, 0).is_err());
        assert_eq!(keysym(Key::Unicode('א')).unwrap(), 0x010005d0);
        assert_eq!(keysym(Key::Control).unwrap(), 0xffe3);
    }
}
