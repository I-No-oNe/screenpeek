//! Input for a seat without a keymap, such as a headless Sway. enigo 0.6
//! starts each keyboard from the seat's current keymap, and its own fallback
//! is empty, so it gets one to start from; text bypasses enigo altogether.

use std::io::Write;
use std::os::fd::AsFd;
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use wayland_client::protocol::wl_keyboard::{self, KeymapFormat, WlKeyboard};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, Capability, WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use xkbcommon::xkb;

/// A keyboard with a US keymap, held until the next keyboard has read it.
pub(super) struct Seed {
    _connection: Connection,
}

#[derive(Default)]
struct State {
    seat: Option<WlSeat>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
    has_keyboard: bool,
    has_keymap: bool,
}

/// The seat with its globals bound.
fn connect() -> Option<(Connection, EventQueue<State>, State)> {
    let connection = Connection::connect_to_env().ok()?;
    let mut queue = connection.new_event_queue();
    connection.display().get_registry(&queue.handle(), ());
    let mut state = State::default();
    queue.roundtrip(&mut state).ok()?;
    // The seat announces its capabilities once bound.
    queue.roundtrip(&mut state).ok()?;
    Some((connection, queue, state))
}

/// Whether the seat has no keymap to give. Ask before making keyboards of our
/// own: they would answer for the seat, and only while they last.
pub(super) fn needed() -> bool {
    let Some((_connection, mut queue, mut state)) = connect() else {
        return false;
    };
    if let (true, Some(seat)) = (state.has_keyboard, &state.seat) {
        let _keyboard = seat.get_keyboard(&queue.handle(), ());
        let _ = queue.roundtrip(&mut state);
    }
    !state.has_keymap
}

/// A new keyboard with a US keymap. Sway makes a new keyboard current when
/// none is, so the next keyboard made starts from this keymap.
pub(super) fn plant() -> Option<Seed> {
    static KEYMAP: OnceLock<Option<String>> = OnceLock::new();
    let keymap = KEYMAP
        .get_or_init(|| {
            let keymap = xkb::Keymap::new_from_names(
                &xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
                "",
                "",
                "us",
                "",
                None,
                xkb::COMPILE_NO_FLAGS,
            )?;
            Some(keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1))
        })
        .as_ref()?;
    let (connection, mut queue, mut state) = connect()?;
    keyboard_with(keymap, &mut queue, &mut state).ok()?;
    Some(Seed {
        _connection: connection,
    })
}

/// A new virtual keyboard carrying `keymap`.
fn keyboard_with(
    keymap: &str,
    queue: &mut EventQueue<State>,
    state: &mut State,
) -> Result<ZwpVirtualKeyboardV1> {
    let mut file = tempfile::tempfile()?;
    file.write_all(keymap.as_bytes())?;
    file.write_all(&[0])?;
    let seat = state.seat.as_ref().context("no seat")?;
    let keyboard = state
        .manager
        .as_ref()
        .context("no virtual keyboard support")?
        .create_virtual_keyboard(seat, &queue.handle(), ());
    keyboard.keymap(
        xkb::KEYMAP_FORMAT_TEXT_V1,
        file.as_fd(),
        u32::try_from(keymap.len() + 1)?,
    );
    queue.roundtrip(state)?;
    Ok(keyboard)
}

/// Type through one keyboard whose keymap holds just the text's characters,
/// one key each, so no Shift and no keymap change can lose a key.
pub(super) fn type_text(text: &str, pace: Duration, keymap_delay: Duration) -> Result<()> {
    let mut distinct: Vec<char> = Vec::new();
    for character in text.chars() {
        if !distinct.contains(&character) {
            distinct.push(character);
        }
    }
    let keymap = text_keymap(&distinct)?;
    let (_connection, mut queue, mut state) = connect().context("no Wayland seat")?;
    let keyboard = keyboard_with(&keymap, &mut queue, &mut state)?;
    // Keys sent before the keymap lands are dropped.
    sleep(keymap_delay);
    let started = Instant::now();
    for character in text.chars() {
        let index = distinct
            .iter()
            .position(|c| *c == character)
            .expect("listed");
        let key = u32::try_from(index)? + 1;
        let time = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
        keyboard.key(time, key, 1);
        keyboard.key(time, key, 0);
        queue.roundtrip(&mut state)?;
        sleep(pace);
    }
    Ok(())
}

/// An xkb keymap where keycode 9 + i types `characters[i]`, evdev code i + 1.
fn text_keymap(characters: &[char]) -> Result<String> {
    let mut codes = String::new();
    let mut symbols = String::new();
    for (index, character) in characters.iter().enumerate() {
        let keysym = match character {
            '\n' => xkb::Keysym::Return,
            '\t' => xkb::Keysym::Tab,
            other => xkb::utf32_to_keysym(u32::from(*other)),
        };
        if keysym.raw() == 0 {
            bail!("cannot type {character:?}");
        }
        codes.push_str(&format!("<K{index}> = {}; ", index + 9));
        symbols.push_str(&format!(
            "key <K{index}> {{ [ {} ] }}; ",
            xkb::keysym_get_name(keysym)
        ));
    }
    Ok(format!(
        "xkb_keymap {{\n\
         xkb_keycodes \"screenpeek\" {{ minimum = 8; maximum = {}; {codes}}};\n\
         xkb_types \"screenpeek\" {{ include \"complete\" }};\n\
         xkb_compatibility \"screenpeek\" {{ include \"complete\" }};\n\
         xkb_symbols \"screenpeek\" {{ {symbols}}};\n\
         }};\n",
        characters.len() + 9
    ))
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut State,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<State>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_seat" if state.seat.is_none() => {
                state.seat = Some(registry.bind(name, version.min(7), handle, ()));
            }
            "zwp_virtual_keyboard_manager_v1" => {
                state.manager = Some(registry.bind(name, 1, handle, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut State,
        _: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            state.has_keyboard = capabilities.contains(Capability::Keyboard);
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut State,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_keyboard::Event::Keymap { format, .. } = event {
            state.has_keymap = format == WEnum::Value(KeymapFormat::XkbV1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_keymap_compiles_with_one_key_per_character() {
        let keymap = text_keymap(&['H', '!', '~', '\n', '\u{5d0}']).unwrap();
        let compiled = xkb::Keymap::new_from_string(
            &xkb::Context::new(xkb::CONTEXT_NO_FLAGS),
            keymap,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::COMPILE_NO_FLAGS,
        )
        .expect("keymap compiles");
        let state = xkb::State::new(&compiled);
        // Keycode 9 + i types character i, with no modifier.
        assert_eq!(state.key_get_utf8(9u32.into()), "H");
        assert_eq!(state.key_get_utf8(10u32.into()), "!");
        assert_eq!(state.key_get_utf8(11u32.into()), "~");
        assert_eq!(state.key_get_utf8(13u32.into()), "\u{5d0}");
    }
}

macro_rules! ignore_events {
    ($($interface:ty),* $(,)?) => {$(
        impl Dispatch<$interface, ()> for State {
            fn event(
                _: &mut State,
                _: &$interface,
                _: <$interface as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<State>,
            ) {
            }
        }
    )*};
}

ignore_events!(ZwpVirtualKeyboardManagerV1, ZwpVirtualKeyboardV1);
