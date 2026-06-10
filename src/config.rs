use serde::{Deserialize, Serialize};
use talk_core::cleanup::{parse_level, Level};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub base_dir: Option<String>,
    pub default_mode: String,        // "reflect" | "journal"
    pub keep_raw: bool,
    pub raw_sidecar: bool,           // true: raw goes to ~/talk/.raw/, not inline
    pub auto_end_silence_seconds: u32, // 0 = off
    pub default_pack: String,
    pub reflect_cleanup: String,     // none | light | medium | high
    pub journal_cleanup: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base_dir: None,
            default_mode: "reflect".into(),
            keep_raw: true,
            raw_sidecar: false,
            auto_end_silence_seconds: 0,
            default_pack: "spine".into(),
            reflect_cleanup: "light".into(),
            journal_cleanup: "medium".into(),
        }
    }
}

impl Config {
    pub fn load(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    /// The cleanup `Level` for a mode ("journal" → journal_cleanup, else reflect).
    pub fn cleanup_for(&self, mode: &str) -> Level {
        let s = if mode == "journal" { &self.journal_cleanup } else { &self.reflect_cleanup };
        parse_level(s)
    }

    /// The fully-commented template `talk config init` writes.
    pub fn commented_template() -> String {
        let d = Config::default();
        format!(
            "# talk config — every line is optional; zero-config still launches.\n\
             # base_dir = \"~/talk\"          # where reflections land\n\
             default_mode = \"{mode}\"          # bare `talk` runs this\n\
             keep_raw = {keep}                 # store verbatim transcript in a hidden comment\n\
             raw_sidecar = {sidecar}           # true: verbatim raw goes to ~/talk/.raw/ (skipped by vault sync) instead of inline comments\n\
             auto_end_silence_seconds = {silence}  # 0 = off; you press space to finish\n\
             default_pack = \"{pack}\"\n\
             # cleanup levels: none · light · medium · high\n\
             reflect_cleanup = \"{rc}\"        # light: caps + punctuation + leading filler. \"um so i guess\" → \"I guess.\"\n\
             journal_cleanup = \"{jc}\"       # medium/high: deterministic-only in v1 (LLM enhances light); full LLM rewrite is future work\n",
            mode = d.default_mode, keep = d.keep_raw, sidecar = d.raw_sidecar,
            silence = d.auto_end_silence_seconds, pack = d.default_pack,
            rc = d.reflect_cleanup, jc = d.journal_cleanup,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_config_uses_defaults() {
        let c = Config::load("").unwrap();
        assert_eq!(c.default_mode, "reflect");
        assert!(c.keep_raw);
        assert_eq!(c.cleanup_for("reflect"), Level::Light);
        assert_eq!(c.cleanup_for("journal"), Level::Medium);
    }

    #[test]
    fn template_is_loadable() {
        let c = Config::load(&Config::commented_template()).unwrap();
        assert_eq!(c.auto_end_silence_seconds, 0);
        assert_eq!(c.cleanup_for("reflect"), Level::Light);
    }

    #[test]
    fn pins_override_defaults() {
        let c = Config::load("default_mode = \"journal\"\nkeep_raw = false\njournal_cleanup = \"high\"\n").unwrap();
        assert_eq!(c.default_mode, "journal");
        assert!(!c.keep_raw);
        assert_eq!(c.cleanup_for("journal"), Level::High);
    }
}
