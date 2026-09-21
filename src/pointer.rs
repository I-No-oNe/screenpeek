//! Mouse and keyboard through enigo. On Wayland this goes over the virtual
//! pointer and keyboard protocols, with no root access and no daemon.

use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use enigo::{Button as EnigoButton, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

/// Time for a clicked widget to take focus before typing into it.
const FOCUS_DELAY: Duration = Duration::from_millis(120);

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
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(single), None) => Key::Unicode(single),
                _ => bail!("unknown key {key:?}"),
            }
        }
    })
}

pub struct Pointer {
    enigo: Enigo,
}

impl Pointer {
    pub fn new() -> Result<Pointer> {
        let enigo = Enigo::new(&Settings::default()).context(
            "cannot reach the input backend; on Wayland the compositor must support the \
             virtual pointer and virtual keyboard protocols",
        )?;
        Ok(Pointer { enigo })
    }

    pub fn click(&mut self, x: i32, y: i32, button: Button, times: u32) -> Result<()> {
        self.enigo
            .move_mouse(x, y, Coordinate::Abs)
            .with_context(|| format!("cannot move the pointer to {x},{y}"))?;
        for _ in 0..times {
            self.enigo
                .button(button.into(), Direction::Click)
                .context("cannot click")?;
        }
        Ok(())
    }

    /// Presses a named key, or a combination such as `ctrl+s`.
    pub fn press(&mut self, combination: &str) -> Result<()> {
        let mut parts: Vec<&str> = combination.split('+').map(str::trim).collect();
        let key = parts.pop().context("no key given")?;
        let modifiers: Vec<Key> = parts
            .iter()
            .map(|part| named(part))
            .collect::<Result<_>>()?;

        for modifier in &modifiers {
            self.enigo.key(*modifier, Direction::Press)?;
        }
        let result = self.enigo.key(named(key)?, Direction::Click);
        for modifier in modifiers.iter().rev() {
            self.enigo.key(*modifier, Direction::Release)?;
        }
        result.with_context(|| format!("cannot press {combination:?}"))
    }

    pub fn type_text(&mut self, text: &str) -> Result<()> {
        self.enigo
            .text(text)
            .with_context(|| format!("cannot type {text:?}"))
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
        assert!(named("nonsense").is_err());
    }

    #[test]
    fn parses_button_names() {
        assert_eq!("left".parse::<Button>().unwrap(), Button::Left);
        assert_eq!("RIGHT".parse::<Button>().unwrap(), Button::Right);
        assert!("scroll".parse::<Button>().is_err());
    }
}
