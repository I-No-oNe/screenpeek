//! Which extra languages to read, and when a line needs them.

use serde::{Deserialize, Serialize};

/// Extra languages for Tesseract, such as `eng+heb`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Language {
    pub codes: String,
    /// Only re-read lines when the built-in reader produced garbled text,
    /// so screens without foreign text cost nothing extra.
    pub auto: bool,
}

/// The languages chosen at install time, saved by `scripts/fetch-models.sh`.
pub fn configured() -> Option<String> {
    let path = dirs::config_dir()?.join("screenpeek").join("languages");
    let codes: Vec<String> = std::fs::read_to_string(path)
        .ok()?
        .split(|c: char| c.is_whitespace() || c == '+' || c == ',')
        .filter(|code| !code.is_empty() && *code != "eng")
        .map(str::to_owned)
        .collect();
    (!codes.is_empty()).then(|| format!("eng+{}", codes.join("+")))
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
        .filter(|language| !matches!(*language, "osd" | "equ" | "eng"))
        .collect();
    languages.sort_unstable();
    if installed.iter().any(|have| have == "eng") {
        languages.insert(0, "eng");
    }
    languages.join("+")
}

/// Whether the built-in (Latin) reader's output looks like a misread of
/// another script: unknown glyphs come back as `?`, digit-letter mixes such
/// as `7U`, lowercase-uppercase pairs such as `mI`, or vowelless words.
pub fn garbled(text: &str) -> bool {
    text.contains('?') || text.split(|c: char| !c.is_alphanumeric()).any(garbled_word)
}

fn garbled_word(word: &str) -> bool {
    const COMMON: &[&str] = &[
        "ctrl", "http", "https", "www", "std", "npm", "pwd", "ssh", "sftp", "fps",
    ];
    let letters: Vec<char> = word.chars().filter(char::is_ascii_alphabetic).collect();
    let has_digits = word.chars().any(|c| c.is_ascii_digit());
    // 1080p, 3rd, x64, mp3, v2: digits and lowercase letters in one run each.
    let tidy = |word: &str| {
        let rest = word.trim_start_matches(|c: char| c.is_ascii_lowercase());
        let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
        rest.chars().all(|c| c.is_ascii_lowercase())
    };
    let mixed = !letters.is_empty() && has_digits && !tidy(word);
    let short_camel = word.chars().count() <= 4
        && word
            .chars()
            .zip(word.chars().skip(1))
            .any(|(a, b)| a.is_ascii_lowercase() && b.is_ascii_uppercase());
    let vowelless = letters.len() >= 3
        && letters.iter().any(char::is_ascii_lowercase)
        && !letters.iter().any(|c| "aeiouyAEIOUY".contains(*c))
        && !COMMON.contains(&word.to_ascii_lowercase().as_str());
    mixed || short_camel || vowelless
}

/// Whether a page of lines holds enough garbled ones to be worth re-reading.
pub fn worth_rereading(texts: &[&str]) -> bool {
    let garbled = texts.iter().filter(|text| garbled(text)).count();
    garbled >= 2 && garbled * 100 >= texts.len() * 3
}

/// Keep the recognized text unless the other reading is clearly better.
pub fn prefer(recognized: &str, reread: &str) -> bool {
    !reread.is_empty() && (recognized.is_empty() || !reread.is_ascii() || garbled(recognized))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed() -> Vec<String> {
        ["osd", "heb", "eng", "deu"].map(String::from).to_vec()
    }

    #[test]
    fn english_is_read_alongside_whatever_is_asked_for() {
        assert_eq!(with_english("heb", &installed()), "eng+heb");
        assert_eq!(with_english("heb+eng", &installed()), "heb+eng");
        assert_eq!(every(&installed()), "eng+deu+heb");
    }

    #[test]
    fn misreads_of_other_scripts_are_garbled_and_english_is_not() {
        for misread in ["Ha????????", "7U", "YJ777 nio", "mI", "bdz", "PDFZ1R17"] {
            assert!(garbled(misread), "{misread}");
        }
        for clean in [
            "Preferences",
            "Save 3 files",
            "Open file",
            "Font size",
            "Cancel",
            "13",
            "1080p",
            "x64",
            "PDF",
            "Ctrl+S",
            "https://example.com",
            "v2.1",
            "3rd",
        ] {
            assert!(!garbled(clean), "{clean}");
        }
        assert!(!worth_rereading(
            &["Save", "Cancel", "7U", "Open file"][..3]
        ));
        assert!(worth_rereading(&["mI", "7U", "Cancel"]));
    }

    #[test]
    fn rereads_win_only_when_they_add_something() {
        assert!(prefer("7U", "שמור"));
        assert!(prefer("Fenetre", "Fenêtre"));
        assert!(!prefer("Save", "Sаve".replace('а', "a").as_str()));
        assert!(!prefer("mI", ""));
    }
}
