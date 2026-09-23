//! Key names as commands write them, such as `ctrl`, `enter` or `slash`.

use anyhow::{bail, Result};
use enigo::Key;

pub(super) fn named(key: &str) -> Result<Key> {
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
}
