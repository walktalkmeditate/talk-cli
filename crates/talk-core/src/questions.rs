use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Question {
    /// Immutable identity — binds the thread, never derived from `text`.
    pub id: String,
    pub text: String,
    /// Optional authored filename slug; if absent, the binary derives one from
    /// `id`. Stored in the spec's TOML schema, so the struct must accept it.
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default = "default_addressee")]
    pub addressee: String,
    #[serde(default = "default_cadence")]
    pub cadence: String, // "daily" or "held:N"
    #[serde(default)]
    pub slot: Option<String>, // "morning" / "evening" / None
}

fn default_addressee() -> String { "self".into() }
fn default_cadence() -> String { "daily".into() }

#[derive(Clone, Debug, Deserialize)]
pub struct Pack {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "questions", default)]
    pub questions: Vec<Question>,
}

impl Pack {
    pub fn from_toml(s: &str) -> Result<Pack, toml::de::Error> {
        toml::from_str(s)
    }

    /// Parse "held:N" → Some(N); anything else → None.
    pub fn held_len(cadence: &str) -> Option<u32> {
        cadence.strip_prefix("held:").and_then(|n| n.parse().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_pack_with_defaults() {
        let toml = r#"
            name = "future-self"
            description = "Talk to the you that's coming."
            [[questions]]
            id = "scared-of"
            text = "Tell next-December you what you're scared of."
            addressee = "future-self"
            [[questions]]
            id = "held-avoid"
            text = "What am I avoiding?"
            cadence = "held:7"
        "#;
        let pack = Pack::from_toml(toml).unwrap();
        assert_eq!(pack.name, "future-self");
        assert_eq!(pack.questions.len(), 2);
        assert_eq!(pack.questions[0].addressee, "future-self");
        assert_eq!(pack.questions[1].cadence, "held:7");
        assert_eq!(pack.questions[0].cadence, "daily"); // default
        assert_eq!(Pack::held_len("held:7"), Some(7));
        assert_eq!(Pack::held_len("daily"), None);
    }
}
