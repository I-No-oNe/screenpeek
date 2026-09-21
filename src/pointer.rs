//! Mouse and keyboard through enigo. On Wayland this goes over the virtual
//! pointer and keyboard protocols, with no root access and no daemon.

use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use enigo::{Button as EnigoButton, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};

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
    fn parses_button_names() {
        assert_eq!("left".parse::<Button>().unwrap(), Button::Left);
        assert_eq!("RIGHT".parse::<Button>().unwrap(), Button::Right);
        assert!("scroll".parse::<Button>().is_err());
    }
}
