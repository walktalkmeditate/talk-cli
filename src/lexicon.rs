use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// A loaded personal lexicon: substitution pairs sorted longest-key-first.
pub struct Lexicon {
    pairs: Vec<(String, String)>,
}

#[derive(Deserialize, Default)]
struct LexiconFile {
    #[serde(default)]
    corrections: BTreeMap<String, String>,
}

impl Lexicon {
    /// Build from already-parsed corrections (pure; used by `load` and tests).
    pub fn from_map(map: BTreeMap<String, String>) -> Lexicon {
        let mut pairs: Vec<(String, String)> = map.into_iter().collect();
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        Lexicon { pairs }
    }

    /// Load `[corrections]` from `path`. Missing file → empty (silent). Malformed
    /// TOML → warn once to stderr, empty. Call this BEFORE entering the live screen
    /// so a warning never lands inside the alternate-screen TUI.
    pub fn load(path: &Path) -> Lexicon {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Lexicon::from_map(BTreeMap::new());
        };
        match toml::from_str::<LexiconFile>(&text) {
            Ok(f) => Lexicon::from_map(f.corrections),
            Err(e) => {
                eprintln!("lexicon ignored ({}): {e}", path.display());
                Lexicon::from_map(BTreeMap::new())
            }
        }
    }

    pub fn correct(&self, text: &str) -> String {
        talk_core::lexicon::apply_lexicon(text, &self.pairs)
    }
}

/// The one transform both transcript paths apply to the CLEAN text: user lexicon
/// first (so a corrected word is never mistaken for a sound tag), then sound-tags.
pub fn correct(raw: &str, lexicon: &Lexicon) -> String {
    talk_core::cleanup::strip_sound_tags(&lexicon.correct(raw))
}

/// The fully-commented `lexicon.toml` written by `talk config init`.
pub fn template() -> &'static str {
    "# talk lexicon — teach talk the proper nouns it mishears.\n\
     # Word-bounded, case-insensitive match; the value sets the spelling\n\
     # (sentence-start capitalization still applies on the live path).\n\
     # Uncomment and edit; talk corrects nothing until you do.\n\
     #\n\
     # [corrections]\n\
     # \"TOC\"   = \"talk\"        # the tool's own name\n\
     # \"WOC\"   = \"walk\"\n\
     # \"WAC\"   = \"walk\"\n\
     # \"cloth\" = \"Claude\"\n\
     # \"Obsidian\" = \"Obsidian\" # force exact casing\n\
     # \"Pilgrim\"  = \"Pilgrim\"\n\
     # \"Ellen\"    = \"Ellen\"    # names talk guesses wrong\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_applies_lexicon_then_strips_tags() {
        let lex = Lexicon::from_map(
            [("TOC".to_string(), "talk".to_string())].into_iter().collect(),
        );
        assert_eq!(correct("open TOC (buzzer) now", &lex), "open talk now");
    }

    #[test]
    fn empty_lexicon_still_strips_tags() {
        let lex = Lexicon::from_map(BTreeMap::new());
        assert_eq!(correct("hi (applause) there", &lex), "hi there");
        assert_eq!(correct("plain words", &lex), "plain words");
    }

    #[test]
    fn shipped_template_parses_to_an_empty_map() {
        let f: LexiconFile = toml::from_str(template()).unwrap();
        assert!(f.corrections.is_empty());
    }
}
