//! Working out which language the screen is in.
//!
//! Two free signals say it without reading a single pixel twice: the session's
//! locale, and the script of the text the accessibility tree already handed
//! over. Whatever they suggest is checked against the language data tesseract
//! actually has, so `--lang auto` never asks for something that is not there.

use std::env;

use crate::index::Element;

/// Locale language to tesseract code, for the languages whose data is
/// commonly installed.
const LOCALES: &[(&str, &str)] = &[
    ("ar", "ara"),
    ("bg", "bul"),
    ("cs", "ces"),
    ("da", "dan"),
    ("de", "deu"),
    ("el", "ell"),
    ("en", "eng"),
    ("es", "spa"),
    ("et", "est"),
    ("fa", "fas"),
    ("fi", "fin"),
    ("fr", "fra"),
    ("he", "heb"),
    ("hi", "hin"),
    ("hu", "hun"),
    ("id", "ind"),
    ("it", "ita"),
    ("iw", "heb"),
    ("ja", "jpn"),
    ("ko", "kor"),
    ("nl", "nld"),
    ("no", "nor"),
    ("pl", "pol"),
    ("pt", "por"),
    ("ro", "ron"),
    ("ru", "rus"),
    ("sv", "swe"),
    ("th", "tha"),
    ("tr", "tur"),
    ("uk", "ukr"),
    ("vi", "vie"),
    ("zh", "chi_sim"),
];

/// A script seen on screen, and the languages that are written in it.
const SCRIPTS: &[(Script, &[&str])] = &[
    (Script::Hebrew, &["heb"]),
    (Script::Arabic, &["ara", "fas"]),
    (Script::Cyrillic, &["rus", "ukr", "bul"]),
    (Script::Greek, &["ell"]),
    (Script::Han, &["chi_sim", "jpn"]),
    (Script::Kana, &["jpn"]),
    (Script::Hangul, &["kor"]),
    (Script::Devanagari, &["hin"]),
    (Script::Thai, &["tha"]),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Script {
    Hebrew,
    Arabic,
    Cyrillic,
    Greek,
    Han,
    Kana,
    Hangul,
    Devanagari,
    Thai,
}

/// The languages worth reading this screen in, as a tesseract argument such as
/// `eng+heb`. `None` means English alone, which the built-in model already
/// reads faster than tesseract can.
pub fn detect(known_text: &[Element], installed: &[String]) -> Option<String> {
    let mut wanted: Vec<&str> = Vec::new();

    for script in scripts_in(known_text) {
        if let Some((_, languages)) = SCRIPTS.iter().find(|(kind, _)| *kind == script) {
            for language in *languages {
                if installed.iter().any(|have| have == language) {
                    wanted.push(language);
                    break;
                }
            }
        }
    }

    if let Some(language) = from_locale() {
        if language != "eng" && installed.iter().any(|have| have == language) {
            wanted.push(language);
        }
    }

    wanted.dedup();
    if wanted.is_empty() {
        return None;
    }

    if installed.iter().any(|have| have == "eng") {
        wanted.insert(0, "eng");
    }
    Some(wanted.join("+"))
}

/// The language this session is set to, as a tesseract code.
fn from_locale() -> Option<&'static str> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| env::var(name).ok())?;

    let code = locale.split(['_', '.', '@']).next()?.to_lowercase();
    LOCALES
        .iter()
        .find(|(prefix, _)| *prefix == code)
        .map(|(_, language)| *language)
}

/// The scripts present in text that is already known, which on Linux and
/// Windows is whatever the accessibility tree reported.
fn scripts_in(elements: &[Element]) -> Vec<Script> {
    let mut seen: Vec<Script> = Vec::new();
    for character in elements.iter().flat_map(|element| element.text.chars()) {
        let Some(script) = script_of(character) else {
            continue;
        };
        if !seen.contains(&script) {
            seen.push(script);
        }
    }
    seen
}

fn script_of(character: char) -> Option<Script> {
    match character as u32 {
        0x0590..=0x05FF => Some(Script::Hebrew),
        0x0600..=0x06FF | 0x0750..=0x077F => Some(Script::Arabic),
        0x0370..=0x03FF => Some(Script::Greek),
        0x0400..=0x04FF => Some(Script::Cyrillic),
        0x0900..=0x097F => Some(Script::Devanagari),
        0x0E00..=0x0E7F => Some(Script::Thai),
        0x3040..=0x30FF => Some(Script::Kana),
        0xAC00..=0xD7AF | 0x1100..=0x11FF => Some(Script::Hangul),
        0x4E00..=0x9FFF => Some(Script::Han),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(text: &str) -> Element {
        Element {
            id: 0,
            text: text.to_owned(),
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        }
    }

    fn installed() -> Vec<String> {
        ["eng", "heb", "rus", "jpn"]
            .iter()
            .map(|code| (*code).to_owned())
            .collect()
    }

    #[test]
    fn hebrew_on_screen_asks_for_hebrew() {
        let detected = detect(&[element("שמור"), element("Save")], &installed());
        assert_eq!(detected.as_deref(), Some("eng+heb"));
    }

    #[test]
    fn several_scripts_are_all_requested() {
        let detected = detect(&[element("Привет"), element("こんにちは")], &installed());
        let detected = detected.expect("two scripts were seen");
        assert!(detected.contains("rus"), "{detected}");
        assert!(detected.contains("jpn"), "{detected}");
    }

    #[test]
    fn a_script_with_no_data_installed_is_not_asked_for() {
        assert_eq!(detect(&[element("สวัสดี")], &installed()), None);
    }

    #[test]
    fn plain_english_needs_no_second_engine() {
        assert_eq!(detect(&[element("Save file")], &installed()), None);
    }

    #[test]
    fn locale_codes_map_to_tesseract_codes() {
        assert_eq!(
            LOCALES.iter().find(|(prefix, _)| *prefix == "he"),
            Some(&("he", "heb"))
        );
        assert_eq!(script_of('ש'), Some(Script::Hebrew));
        assert_eq!(script_of('a'), None);
    }
}
