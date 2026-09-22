//! Choose installed OCR languages from locale and accessible text.

use std::env;

use crate::index::Element;

/// Locale code to Tesseract code.
const LOCALES: &[(&str, &str)] = &[
    ("af", "afr"),
    ("am", "amh"),
    ("az", "aze"),
    ("bn", "ben"),
    ("ca", "cat"),
    ("eu", "eus"),
    ("gl", "glg"),
    ("gu", "guj"),
    ("hr", "hrv"),
    ("hy", "hye"),
    ("ka", "kat"),
    ("kk", "kaz"),
    ("km", "khm"),
    ("kn", "kan"),
    ("lo", "lao"),
    ("lt", "lit"),
    ("lv", "lav"),
    ("mk", "mkd"),
    ("ml", "mal"),
    ("ms", "msa"),
    ("my", "mya"),
    ("nb", "nor"),
    ("ne", "nep"),
    ("nn", "nor"),
    ("pa", "pan"),
    ("si", "sin"),
    ("sk", "slk"),
    ("sl", "slv"),
    ("sq", "sqi"),
    ("sr", "srp"),
    ("ta", "tam"),
    ("te", "tel"),
    ("ur", "urd"),
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
    (Script::Armenian, &["hye"]),
    (Script::Georgian, &["kat"]),
    (Script::Bengali, &["ben"]),
    (Script::Gurmukhi, &["pan"]),
    (Script::Gujarati, &["guj"]),
    (Script::Tamil, &["tam"]),
    (Script::Telugu, &["tel"]),
    (Script::Kannada, &["kan"]),
    (Script::Malayalam, &["mal"]),
    (Script::Sinhala, &["sin"]),
    (Script::Ethiopic, &["amh"]),
    (Script::Khmer, &["khm"]),
    (Script::Lao, &["lao"]),
    (Script::Myanmar, &["mya"]),
    (Script::Tibetan, &["bod"]),
    (Script::Hebrew, &["heb"]),
    (Script::Arabic, &["ara", "fas", "urd"]),
    (
        Script::Cyrillic,
        &["rus", "ukr", "bul", "srp", "mkd", "kaz"],
    ),
    (Script::Greek, &["ell"]),
    (Script::Han, &["chi_sim", "chi_tra", "jpn"]),
    (Script::Kana, &["jpn"]),
    (Script::Hangul, &["kor"]),
    (Script::Devanagari, &["hin", "nep"]),
    (Script::Thai, &["tha"]),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Script {
    Armenian,
    Georgian,
    Bengali,
    Gurmukhi,
    Gujarati,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Sinhala,
    Ethiopic,
    Khmer,
    Lao,
    Myanmar,
    Tibetan,
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

/// Return Tesseract languages, or None for built-in English recognition.
pub fn detect(known_text: &[Element], installed: &[String]) -> Option<String> {
    detect_with_locale(known_text, installed, from_locale())
}

fn detect_with_locale(
    known_text: &[Element],
    installed: &[String],
    locale: Option<&str>,
) -> Option<String> {
    let mut wanted: Vec<&str> = Vec::new();

    let scripts = scripts_in(known_text);
    for script in &scripts {
        // Kana/Hangul identify the reader for accompanying Han characters.
        if *script == Script::Han
            && ((scripts.contains(&Script::Kana) && installed.iter().any(|l| l == "jpn"))
                || (scripts.contains(&Script::Hangul) && installed.iter().any(|l| l == "kor")))
        {
            continue;
        }
        if let Some((_, languages)) = SCRIPTS.iter().find(|(kind, _)| kind == script) {
            if let Some(language) = locale.filter(|l| languages.contains(l)) {
                if installed.iter().any(|have| have == language) {
                    wanted.push(language);
                    continue;
                }
            }
            for language in *languages {
                if installed.iter().any(|have| have == language) {
                    wanted.push(language);
                    break;
                }
            }
        }
    }

    if let Some(language) = locale {
        if language != "eng" && installed.iter().any(|have| have == language) {
            wanted.push(language);
        }
    }

    wanted.sort_unstable();
    wanted.dedup();
    if wanted.is_empty() {
        return None;
    }

    if installed.iter().any(|have| have == "eng") {
        wanted.insert(0, "eng");
    }
    Some(wanted.join("+"))
}

/// Add installed English data for mixed-language interfaces.
pub fn with_english(requested: &str, installed: &[String]) -> String {
    let has_english = requested.split('+').any(|part| part == "eng");
    if has_english || !installed.iter().any(|have| have == "eng") {
        return requested.to_owned();
    }
    format!("eng+{requested}")
}

/// Return all installed text languages, English first.
pub fn every(installed: &[String]) -> String {
    let mut languages: Vec<&str> = installed
        .iter()
        .map(String::as_str)
        .filter(|language| *language != "osd" && *language != "equ" && *language != "eng")
        .collect();
    languages.sort_unstable();
    if installed.iter().any(|have| have == "eng") {
        languages.insert(0, "eng");
    }
    languages.join("+")
}

/// The language this session is set to, as a tesseract code.
fn from_locale() -> Option<&'static str> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| env::var(name).ok().filter(|value| !value.is_empty()))?;

    locale_language(&locale)
}

fn locale_language(locale: &str) -> Option<&'static str> {
    let locale = locale.to_ascii_lowercase().replace('-', "_");
    let parts: Vec<_> = locale.split(['_', '.', '@']).collect();
    let code = *parts.first()?;
    if code == "zh"
        && parts
            .iter()
            .any(|p| matches!(*p, "tw" | "hk" | "mo" | "hant"))
    {
        return Some("chi_tra");
    }
    LOCALES
        .iter()
        .find(|(prefix, _)| *prefix == code)
        .map(|(_, language)| *language)
}

/// Scripts present in accessible text.
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
        0x0530..=0x058F => Some(Script::Armenian),
        0x10A0..=0x10FF | 0x1C90..=0x1CBF | 0x2D00..=0x2D2F => Some(Script::Georgian),
        0x0980..=0x09FF => Some(Script::Bengali),
        0x0A00..=0x0A7F => Some(Script::Gurmukhi),
        0x0A80..=0x0AFF => Some(Script::Gujarati),
        0x0B80..=0x0BFF => Some(Script::Tamil),
        0x0C00..=0x0C7F => Some(Script::Telugu),
        0x0C80..=0x0CFF => Some(Script::Kannada),
        0x0D00..=0x0D7F => Some(Script::Malayalam),
        0x0D80..=0x0DFF => Some(Script::Sinhala),
        0x1200..=0x137F => Some(Script::Ethiopic),
        0x1780..=0x17FF => Some(Script::Khmer),
        0x0E80..=0x0EFF => Some(Script::Lao),
        0x1000..=0x109F => Some(Script::Myanmar),
        0x0F00..=0x0FFF => Some(Script::Tibetan),
        0x0590..=0x05FF => Some(Script::Hebrew),
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => {
            Some(Script::Arabic)
        }
        0x0370..=0x03FF => Some(Script::Greek),
        0x0400..=0x04FF => Some(Script::Cyrillic),
        0x0900..=0x097F => Some(Script::Devanagari),
        0x0E00..=0x0E7F => Some(Script::Thai),
        0x3040..=0x30FF => Some(Script::Kana),
        0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xD7B0..=0xD7FF => {
            Some(Script::Hangul)
        }
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0x20000..=0x2EBEF => Some(Script::Han),
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
            source: crate::index::Source::Ocr,
            ..Default::default()
        }
    }

    fn detect(known: &[Element], installed: &[String]) -> Option<String> {
        detect_with_locale(known, installed, None)
    }

    fn installed() -> Vec<String> {
        ["eng", "heb", "rus", "jpn"]
            .iter()
            .map(|code| (*code).to_owned())
            .collect()
    }

    #[test]
    fn english_is_read_alongside_whatever_is_asked_for() {
        assert_eq!(with_english("heb", &installed()), "eng+heb");
        assert_eq!(with_english("eng+heb", &installed()), "eng+heb");
        assert_eq!(with_english("heb+eng", &installed()), "heb+eng");
        // Nothing to add when English is not installed.
        assert_eq!(with_english("heb", &["heb".to_owned()]), "heb");
    }

    #[test]
    fn every_installed_language_puts_english_first() {
        assert_eq!(every(&installed()), "eng+heb+jpn+rus");
        assert_eq!(every(&["osd".to_owned(), "deu".to_owned()]), "deu");
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
    fn locale_and_script_languages_are_deduplicated() {
        assert_eq!(
            detect_with_locale(&[element("שמור Привет")], &installed(), Some("heb")),
            Some("eng+heb+rus".into())
        );
        assert_eq!(detect_with_locale(&[], &installed(), Some("deu")), None);
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    #[test]
    fn locale_respects_region_and_script() {
        for locale in ["zh_TW.UTF-8", "zh-HK", "zh_MO", "zh-Hant"] {
            assert_eq!(locale_language(locale), Some("chi_tra"));
        }
        assert_eq!(locale_language("zh_CN.UTF-8"), Some("chi_sim"));
        assert_eq!(locale_language("he_IL.UTF-8"), Some("heb"));
    }
    #[test]
    fn specific_script_and_locale_select_the_right_installed_reader() {
        let installed =
            ["eng", "chi_sim", "chi_tra", "jpn", "rus", "ukr", "heb"].map(str::to_owned);
        for (text, locale, expected) in [
            ("保存する", None, "eng+jpn"),
            ("保存", Some("chi_tra"), "eng+chi_tra"),
            ("Привіт", Some("ukr"), "eng+ukr"),
            ("", Some("heb"), "eng+heb"),
        ] {
            let known = [Element {
                id: 0,
                text: text.into(),
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                source: crate::index::Source::Ocr,
                ..Default::default()
            }];
            assert_eq!(
                detect_with_locale(&known, &installed, locale).as_deref(),
                Some(expected)
            );
        }
        for (text, language) in [
            ("বাংলা", "ben"),
            ("தமிழ்", "tam"),
            ("ქართული", "kat"),
            ("㐀", "chi_sim"),
            ("ꥠ", "kor"),
        ] {
            let known = [Element {
                id: 0,
                text: text.into(),
                x: 0,
                y: 0,
                width: 1,
                height: 1,
                source: crate::index::Source::Ocr,
                ..Default::default()
            }];
            assert_eq!(
                detect_with_locale(&known, &[language.into()], None).as_deref(),
                Some(language)
            );
        }
    }
}
