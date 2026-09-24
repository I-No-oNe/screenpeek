//! Mouse and keyboard through enigo. On Wayland this goes over the virtual
//! pointer and keyboard protocols, with no root access and no daemon.

use std::str::FromStr;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{bail, Context, Result};
#[cfg(not(windows))]
use enigo::Coordinate;
pub use enigo::Direction;
use enigo::{Axis, Button as EnigoButton, Enigo, Key, Keyboard, Mouse, Settings};

/// Time for a clicked widget to take focus before typing into it.
const FOCUS_DELAY: Duration = Duration::from_millis(120);

/// Pause between drag movements so applications see a drag, not a jump.
const DRAG_STEP: Duration = Duration::from_millis(15);

/// Time for keys already sent through the portal to arrive before text is committed.
#[cfg(target_os = "linux")]
const KEY_FLUSH: Duration = Duration::from_millis(60);

/// Time around a KDE layout switch for queued keys and the new keymap to settle.
#[cfg(target_os = "linux")]
const LAYOUT_SWITCH: Duration = Duration::from_millis(150);

/// Time for a new keyboard's keymap to land. Raise if characters go missing.
#[cfg(target_os = "linux")]
const KEYMAP_DELAY: Duration = Duration::from_millis(30);

/// Pause after each typed key on Wayland. A compositor reads input between
/// frames, and a slow one (software-rendered Sway in CI) dropped keys from an
/// unpaced burst; with it, 700 characters take about 0.2 s.
const KEY_PACE: Duration = Duration::from_micros(150);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

mod keys;
use keys::named;

#[cfg(target_os = "linux")]
mod kde;
#[cfg(target_os = "linux")]
mod portal;
#[cfg(target_os = "linux")]
pub use portal::Screen;

/// Input the daemon performs with its portal session, so each command need not open one.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum Input {
    Move(i32, i32),
    Button(Button, bool, bool),
    Scroll(i32, bool),
    Press(String),
    Type(String),
}

/// The portal session the daemon keeps for input and screen frames.
#[cfg(target_os = "linux")]
pub struct Desk(portal::Remote);

#[cfg(target_os = "linux")]
impl Desk {
    pub fn open() -> Result<Desk> {
        crate::portal::retry(portal::Remote::new).map(Desk)
    }

    /// The session's PipeWire connection and the monitors it streams.
    pub fn screens(&self) -> Result<(std::os::fd::OwnedFd, Vec<Screen>)> {
        self.0.screens()
    }

    pub fn perform(self, input: Input) -> (Result<()>, Option<Desk>) {
        let mut pointer = Pointer {
            enigo: None,
            desktop: None,
            portal: Some(self.0),
            daemon: false,
        };
        let done = pointer.perform(input);
        (done, pointer.portal.map(Desk))
    }
}

pub struct Pointer {
    enigo: Option<Enigo>,
    /// The logical desktop as x, y, width and height, when on Wayland.
    #[cfg(target_os = "linux")]
    desktop: Option<(i32, i32, u32, u32)>,
    #[cfg(target_os = "linux")]
    portal: Option<portal::Remote>,
    /// Hand input to the daemon, which keeps a portal session open.
    daemon: bool,
}

fn new_enigo() -> Result<Enigo> {
    #[allow(unused_mut)]
    let mut settings = Settings::default();
    #[cfg(target_os = "linux")]
    if wayland_client::Connection::connect_to_env().is_ok() {
        // Avoid sending every input event through both Wayland and XWayland.
        settings.x11_display = Some("screenpeek-disabled".into());
    }
    let enigo = Enigo::new(&settings).context(
        "cannot reach the input backend; on Wayland the compositor must support the \
         virtual pointer and virtual keyboard protocols",
    )?;
    // Keys sent before a new keyboard's keymap lands are dropped.
    #[cfg(target_os = "linux")]
    sleep(KEYMAP_DELAY);
    Ok(enigo)
}

/// enigo scales absolute motion by the first output's mode in pixels, while
/// wlroots compositors spread it over the logical desktop, so convert.
#[cfg(target_os = "linux")]
fn on_virtual_pointer(
    (x, y): (i32, i32),
    (left, top, width, height): (i32, i32, u32, u32),
    (extent_x, extent_y): (i32, i32),
) -> (i32, i32) {
    let scale = |value: i32, origin: i32, size: u32, extent: i32| {
        (f64::from(value - origin) * f64::from(extent) / f64::from(size)).round() as i32
    };
    (
        scale(x, left, width, extent_x),
        scale(y, top, height, extent_y),
    )
}

impl Pointer {
    pub fn new() -> Result<Pointer> {
        // One connection answers both: which input route, and the desktop's shape.
        #[cfg(target_os = "linux")]
        let screencopy = crate::capture::wayland::Screencopy::new().ok();
        #[cfg(target_os = "linux")]
        if std::env::var("SCREENPEEK_INPUT").as_deref() == Ok("portal")
            || (screencopy.is_none() && crate::capture::wayland::logical_desktop().is_some())
        {
            if crate::daemon::available() {
                return Ok(Pointer {
                    enigo: None,
                    desktop: None,
                    portal: None,
                    daemon: true,
                });
            }
            return Self::local_portal();
        }
        Ok(Pointer {
            enigo: Some(new_enigo()?),
            #[cfg(target_os = "linux")]
            desktop: screencopy
                .as_ref()
                .and_then(crate::capture::wayland::Screencopy::logical_desktop),
            #[cfg(target_os = "linux")]
            portal: None,
            daemon: false,
        })
    }

    /// A portal session owned by this process.
    #[cfg(target_os = "linux")]
    fn local_portal() -> Result<Pointer> {
        Ok(Pointer {
            enigo: None,
            desktop: None,
            portal: Some(Desk::open()?.0),
            daemon: false,
        })
    }

    /// Perform input sent by a client.
    pub fn perform(&mut self, input: Input) -> Result<()> {
        match input {
            Input::Move(x, y) => self.move_to(x, y),
            Input::Button(button, press, release) => self.button(
                button,
                match (press, release) {
                    (true, true) => Direction::Click,
                    (true, false) => Direction::Press,
                    _ => Direction::Release,
                },
            ),
            Input::Scroll(steps, horizontal) => self.scroll(steps, horizontal),
            Input::Press(keys) => self.press(&keys),
            Input::Type(text) => self.type_text(&text),
        }
    }

    pub fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        if self.daemon {
            return crate::daemon::act(Input::Move(x, y));
        }
        #[cfg(target_os = "linux")]
        if let Some(portal) = &self.portal {
            return portal.move_to(x, y);
        }
        // enigo's absolute move spans only the primary monitor on Windows.
        #[cfg(windows)]
        let moved = unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y) }
            .map_err(anyhow::Error::from);
        #[cfg(not(windows))]
        let moved = {
            #[allow(unused_mut)]
            let (mut x_sent, mut y_sent) = (x, y);
            #[cfg(target_os = "linux")]
            if let Some(desktop) = self.desktop {
                let extent = self.enigo()?.main_display()?;
                (x_sent, y_sent) = on_virtual_pointer((x, y), desktop, extent);
            }
            self.enigo()?
                .move_mouse(x_sent, y_sent, Coordinate::Abs)
                .map_err(anyhow::Error::from)
        };
        moved.with_context(|| format!("cannot move the pointer to {x},{y}"))
    }

    pub fn button(&mut self, button: Button, direction: Direction) -> Result<()> {
        if self.daemon {
            let press = direction != Direction::Release;
            let release = direction != Direction::Press;
            return crate::daemon::act(Input::Button(button, press, release));
        }
        #[cfg(target_os = "linux")]
        if let Some(portal) = &self.portal {
            if direction != Direction::Release {
                portal.button(button, true)?;
            }
            if direction != Direction::Press {
                portal.button(button, false)?;
            }
            return Ok(());
        }
        self.enigo()?
            .button(button.into(), direction)
            .context("cannot click")
    }

    pub fn click(&mut self, x: i32, y: i32, button: Button, times: u32) -> Result<()> {
        self.move_to(x, y)?;
        for _ in 0..times {
            self.button(button, Direction::Click)?;
        }
        Ok(())
    }

    /// Scroll by whole steps at the pointer; positive is down or right.
    pub fn scroll(&mut self, steps: i32, horizontal: bool) -> Result<()> {
        if self.daemon {
            return crate::daemon::act(Input::Scroll(steps, horizontal));
        }
        #[cfg(target_os = "linux")]
        if let Some(portal) = &self.portal {
            return portal.scroll(steps, horizontal);
        }
        let axis = if horizontal {
            Axis::Horizontal
        } else {
            Axis::Vertical
        };
        self.enigo()?.scroll(steps, axis).context("cannot scroll")
    }

    /// Press at one point, move in small steps, release at the other.
    pub fn drag(&mut self, from: (i32, i32), to: (i32, i32)) -> Result<()> {
        const STEPS: i32 = 12;
        self.move_to(from.0, from.1)?;
        self.button(Button::Left, Direction::Press)?;
        let moved = (1..=STEPS).try_for_each(|step| {
            sleep(DRAG_STEP);
            self.move_to(
                from.0 + (to.0 - from.0) * step / STEPS,
                from.1 + (to.1 - from.1) * step / STEPS,
            )
        });
        sleep(DRAG_STEP);
        let released = self.button(Button::Left, Direction::Release);
        moved.and(released)
    }

    fn enigo(&mut self) -> Result<&mut Enigo> {
        self.enigo.as_mut().context("input backend unavailable")
    }

    pub fn press(&mut self, combination: &str) -> Result<()> {
        if self.daemon {
            return crate::daemon::act(Input::Press(combination.to_owned()));
        }
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

    /// A reused virtual keyboard drops characters that need Shift or a keymap
    /// change, so those get a fresh keyboard each; plain keys reuse one.
    pub fn type_text(&mut self, text: &str) -> Result<()> {
        if self.daemon {
            return crate::daemon::act(Input::Type(text.to_owned()));
        }
        #[cfg(target_os = "linux")]
        if self.portal.is_some() {
            // Text the layout may lack goes whole through GNOME's input method; mixing it
            // with keys reorders them, since keys take the slower portal route.
            if !text.is_ascii() {
                sleep(KEY_FLUSH);
                if crate::desktop::helper::commit(text) {
                    return Ok(());
                }
                if let Some(layouts) = kde::Layouts::new() {
                    return self.type_in_layouts(text, &layouts);
                }
            }
            return self.type_keys(text);
        }
        // Windows takes Unicode text directly.
        if !cfg!(target_os = "linux") {
            return self.enigo()?.text(text).context("cannot type text");
        }
        // Reuse is measured on Wayland only; X11 keeps a keyboard per key.
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        for character in text.chars() {
            let reuse = wayland && plain(character);
            if self.enigo.is_none() || !reuse {
                self.enigo = Some(new_enigo()?);
            }
            // A key press, not enigo's text(): on wlroots that commits through the
            // input method, which apps without text input never see and which drops
            // what arrives before the compositor activates it.
            self.enigo()?
                .key(Key::Unicode(character), Direction::Click)
                .context("cannot type text")?;
            if !reuse {
                self.enigo = None;
            }
            if wayland {
                sleep(KEY_PACE);
            }
        }
        Ok(())
    }

    /// Type each run of text in a KDE layout that has its letters, then switch back.
    #[cfg(target_os = "linux")]
    fn type_in_layouts(&mut self, text: &str, layouts: &kde::Layouts) -> Result<()> {
        let original = layouts.active;
        let mut active = original;
        let mut start = 0;
        let mut typed = Ok(());
        for (index, character) in text.char_indices().chain([(text.len(), ' ')]) {
            let wanted = match index < text.len() {
                true => layouts.for_char(character, original).unwrap_or(active),
                false => original,
            };
            if wanted == active {
                continue;
            }
            typed = typed.and_then(|()| self.type_keys(&text[start..index]));
            // Keys still on their way would use the new layout, and the switch needs a moment.
            sleep(LAYOUT_SWITCH);
            layouts.set(wanted);
            sleep(LAYOUT_SWITCH);
            (active, start) = (wanted, index);
        }
        // The end of the text switched back to the original layout.
        typed.and_then(|()| self.type_keys(&text[start..]))
    }

    /// Type through the portal one key at a time.
    #[cfg(target_os = "linux")]
    fn type_keys(&mut self, text: &str) -> Result<()> {
        for character in text.chars() {
            // The KDE portal applies a keysym's Shift one key late, so hold it here.
            let shifted = character.is_ascii_graphic() && !plain(character);
            if shifted {
                self.key(Key::Shift, Direction::Press)?;
            }
            let typed = self.key(Key::Unicode(character), Direction::Click);
            if shifted {
                self.key(Key::Shift, Direction::Release)?;
            }
            typed?;
        }
        Ok(())
    }

    pub fn wait_for_focus(&self) {
        sleep(FOCUS_DELAY);
    }
}

/// Characters on an unshifted US key, which a reused keyboard types reliably.
fn plain(character: char) -> bool {
    character.is_ascii_lowercase()
        || character.is_ascii_digit()
        || " ,.-=;'/[]`\\".contains(character)
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

    #[cfg(target_os = "linux")]
    #[test]
    fn virtual_pointer_spreads_over_the_logical_desktop() {
        // One 1920x1080 output at scale 1.25, as reported in issue #1.
        let desktop = (0, 0, 1536, 864);
        assert_eq!(
            on_virtual_pointer((283, 649), desktop, (1920, 1080)),
            (354, 811)
        );
        // Unscaled outputs pass through unchanged.
        assert_eq!(
            on_virtual_pointer((283, 649), (0, 0, 1920, 1080), (1920, 1080)),
            (283, 649)
        );
        // A second monitor to the left moves the origin.
        assert_eq!(
            on_virtual_pointer((0, 0), (-1280, 0, 3200, 1080), (1920, 1080)),
            (768, 0)
        );
    }

    /// Clicks the real desktop; scripts/sway-ci.sh runs it and checks where it landed.
    #[test]
    #[ignore = "moves the real pointer"]
    fn click_reaches_the_desktop() {
        if std::env::var("SCREENPEEK_DESKTOP_TEST").as_deref() != Ok("1") {
            return;
        }
        Pointer::new()
            .unwrap()
            .click(283, 649, Button::Left, 1)
            .unwrap();
    }
}
