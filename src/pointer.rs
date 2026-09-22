//! Mouse and keyboard through enigo. On Wayland this goes over the virtual
//! pointer and keyboard protocols, with no root access and no daemon.

use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use enigo::{Button as EnigoButton, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

/// Time for a clicked widget to take focus before typing into it.
const FOCUS_DELAY: Duration = Duration::from_millis(120);

/// Time for a new keyboard's keymap to land. Raise if characters go missing.
const KEYMAP_DELAY: Duration = Duration::from_millis(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl FromStr for Button {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "left" => Ok(Button::Left),
            "right" => Ok(Button::Right),
            "middle" => Ok(Button::Middle),
            other => bail!("unknown button {other:?}; use left, right or middle"),
        }
    }
}

impl From<Button> for EnigoButton {
    fn from(button: Button) -> EnigoButton {
        match button {
            Button::Left => EnigoButton::Left,
            Button::Right => EnigoButton::Right,
            Button::Middle => EnigoButton::Middle,
        }
    }
}

fn named(key: &str) -> Result<Key> {
    Ok(match key.to_lowercase().as_str() {
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        "ctrl" | "control" => Key::Control,
        "alt" => Key::Alt,
        "shift" => Key::Shift,
        "super" | "meta" | "win" => Key::Meta,
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "slash" => Key::Unicode('/'),
        "backslash" => Key::Unicode('\\'),
        "minus" | "dash" | "hyphen" => Key::Unicode('-'),
        "plus" => Key::Unicode('+'),
        "equal" | "equals" => Key::Unicode('='),
        "period" | "dot" | "full_stop" => Key::Unicode('.'),
        "comma" => Key::Unicode(','),
        "semicolon" => Key::Unicode(';'),
        "colon" => Key::Unicode(':'),
        "apostrophe" | "quote" => Key::Unicode('\''),
        "grave" | "backtick" => Key::Unicode('`'),
        "question" => Key::Unicode('?'),
        "exclamation" | "bang" => Key::Unicode('!'),
        "at" => Key::Unicode('@'),
        "hash" | "pound" => Key::Unicode('#'),
        "underscore" => Key::Unicode('_'),
        "asterisk" | "star" => Key::Unicode('*'),
        "bracketleft" => Key::Unicode('['),
        "bracketright" => Key::Unicode(']'),
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(single), None) => Key::Unicode(single),
                _ => bail!("unknown key {key:?}"),
            }
        }
    })
}

#[cfg(target_os = "linux")]
mod portal;

pub struct Pointer {
    enigo: Option<Enigo>,
    #[cfg(target_os = "linux")]
    portal: Option<portal::Remote>,
}

fn new_enigo() -> Result<Enigo> {
    #[allow(unused_mut)]
    let mut settings = Settings::default();
    #[cfg(target_os = "linux")]
    if wayland_client::Connection::connect_to_env().is_ok() {
        // Avoid sending every input event through both Wayland and XWayland.
        settings.x11_display = Some("screenpeek-disabled".into());
    }
    Enigo::new(&settings).context(
        "cannot reach the input backend; on Wayland the compositor must support the \
         virtual pointer and virtual keyboard protocols",
    )
}

impl Pointer {
    pub fn new() -> Result<Pointer> {
        #[cfg(target_os = "linux")]
        if std::env::var("SCREENPEEK_INPUT").as_deref() == Ok("portal")
            || (crate::capture::wayland::Screencopy::new().is_err()
                && crate::capture::wayland::logical_desktop().is_some())
        {
            return Ok(Pointer {
                enigo: None,
                portal: Some(portal::Remote::new()?),
            });
        }
        Ok(Pointer {
            enigo: Some(new_enigo()?),
            #[cfg(target_os = "linux")]
            portal: None,
        })
    }

    pub fn click(&mut self, x: i32, y: i32, button: Button, times: u32) -> Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(portal) = &self.portal {
            return portal.click(x, y, button, times);
        }
        let enigo = self.enigo.as_mut().context("input backend unavailable")?;
        enigo
            .move_mouse(x, y, Coordinate::Abs)
            .with_context(|| format!("cannot move the pointer to {x},{y}"))?;
        for _ in 0..times {
            enigo
                .button(button.into(), Direction::Click)
                .context("cannot click")?;
        }
        Ok(())
    }

    pub fn press(&mut self, combination: &str) -> Result<()> {
        let mut parts: Vec<&str> = combination.split('+').map(str::trim).collect();
        let key = parts.pop().context("no key given")?;
        let modifiers: Vec<Key> = parts
            .iter()
            .map(|part| named(part))
            .collect::<Result<_>>()?;

        let key = named(key)?;
        let mut held = Vec::new();
        let result = (|| -> Result<()> {
            for modifier in &modifiers {
                self.key(*modifier, Direction::Press)?;
                held.push(*modifier);
            }
            self.key(key, Direction::Click)
        })();
        let mut release_error = None;
        for modifier in held.into_iter().rev() {
            if let Err(error) = self.key(modifier, Direction::Release) {
                release_error = Some(error);
            }
        }
        result.with_context(|| format!("cannot press {combination:?}"))?;
        if let Some(error) = release_error {
            return Err(error);
        }
        Ok(())
    }

    fn key(&mut self, key: Key, direction: Direction) -> Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(portal) = &self.portal {
            return match direction {
                Direction::Press => portal.key(key, true),
                Direction::Release => portal.key(key, false),
                Direction::Click => {
                    portal.key(key, true)?;
                    portal.key(key, false)
                }
            };
        }
        self.enigo
            .as_mut()
            .context("input backend unavailable")?
            .key(key, direction)?;
        Ok(())
    }

    /// Recreate the virtual keyboard per character to apply Enigo's keymap.
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        #[cfg(target_os = "linux")]
        if self.portal.is_some() {
            for character in text.chars() {
                self.key(Key::Unicode(character), Direction::Click)?;
            }
            return Ok(());
        }
        for character in text.chars() {
            self.enigo = Some(new_enigo()?);
            sleep(KEYMAP_DELAY);
            self.enigo
                .as_mut()
                .context("input backend unavailable")?
                .text(character.encode_utf8(&mut [0; 4]))
                .context("cannot type text")?;
        }
        Ok(())
    }

    pub fn wait_for_focus(&self) {
        sleep(FOCUS_DELAY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_names() {
        assert!(matches!(named("enter").unwrap(), Key::Return));
        assert!(matches!(named("CTRL").unwrap(), Key::Control));
        assert!(matches!(named("s").unwrap(), Key::Unicode('s')));
        assert!(matches!(named("F4").unwrap(), Key::F4));
        assert!(named("nonsense").is_err());
    }

    #[test]
    fn punctuation_has_names_as_well_as_characters() {
        for (name, character) in [
            ("slash", '/'),
            ("minus", '-'),
            ("period", '.'),
            ("question", '?'),
            ("equals", '='),
        ] {
            assert!(
                matches!(named(name).unwrap(), Key::Unicode(got) if got == character),
                "{name} should press {character}"
            );
            assert!(
                matches!(named(&character.to_string()).unwrap(), Key::Unicode(got) if got == character)
            );
        }
    }

    #[test]
    fn parses_button_names() {
        assert_eq!("left".parse::<Button>().unwrap(), Button::Left);
        assert_eq!("RIGHT".parse::<Button>().unwrap(), Button::Right);
        assert!("scroll".parse::<Button>().is_err());
    }
}
