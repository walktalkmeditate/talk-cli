use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub base_dir: Option<String>,
    pub default_mode: String,        // "reflect" | "journal"
    pub keep_raw: bool,
    pub auto_end_silence_seconds: u32, // 0 = off
    pub default_pack: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base_dir: None,
            default_mode: "reflect".into(),
            keep_raw: true,
            auto_end_silence_seconds: 0,
            default_pack: "spine".into(),
        }
    }
}

impl Config {
    pub fn load(text: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(text)
    }

    /// The fully-commented template `talk config init` writes.
    pub fn commented_template() -> String {
        let d = Config::default();
        format!(
            "# talk config — every line is optional; zero-config still launches.\n\
             # base_dir = \"~/talk\"          # where reflections land\n\
             default_mode = \"{mode}\"          # bare `talk` runs this\n\
             keep_raw = {keep}                 # store verbatim transcript in a hidden comment\n\
             auto_end_silence_seconds = {silence}  # 0 = off; you press space to finish\n\
             default_pack = \"{pack}\"\n",
            mode = d.default_mode, keep = d.keep_raw,
            silence = d.auto_end_silence_seconds, pack = d.default_pack,
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
    }

    #[test]
    fn template_is_loadable() {
        let c = Config::load(&Config::commented_template()).unwrap();
        assert_eq!(c.auto_end_silence_seconds, 0);
    }

    #[test]
    fn pins_override_defaults() {
        let c = Config::load("default_mode = \"journal\"\nkeep_raw = false\n").unwrap();
        assert_eq!(c.default_mode, "journal");
        assert!(!c.keep_raw);
    }
}
