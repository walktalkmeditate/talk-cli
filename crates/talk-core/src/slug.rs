/// Deterministic kebab slug for a bring-your-own question.
/// Lowercase, alphanumeric words only, first 6 words, capped at 60 chars.
pub fn derive_slug(text: &str) -> String {
    let mut words = Vec::new();
    for raw in text.to_lowercase().split_whitespace() {
        let word: String = raw.chars().filter(|c| c.is_alphanumeric()).collect();
        if word.is_empty() { continue; }
        words.push(word);
        if words.len() == 6 { break; }
    }
    let joined = words.join("-");
    let slug: String = joined.chars().take(60).collect();
    if slug.is_empty() { short_hash(text) } else { slug }
}

/// A stable FNV-1a short hash (base36), used to disambiguate slug collisions
/// without pulling a hashing crate. Deterministic across runs and platforms.
pub fn short_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let mut out = String::new();
    let mut n = hash;
    for _ in 0..6 {
        let d = (n % 36) as u32;
        let c = char::from_digit(d, 36).unwrap();
        out.push(c);
        n /= 36;
    }
    out
}

/// Collision-aware slug. `taken(slug)` answers "does a file for this slug
/// already exist for a DIFFERENT question?" (the binary supplies it from disk).
/// On a real collision, append `-{short_hash(text)}` so two distinct questions
/// never share a file — wiring the spec's promised collision suffixing.
pub fn derive_slug_unique(text: &str, taken: impl Fn(&str) -> bool) -> String {
    let base = derive_slug(text);
    if taken(&base) {
        format!("{}-{}", base, short_hash(text))
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_kebab_and_stripped() {
        assert_eq!(derive_slug("What am I avoiding?"), "what-am-i-avoiding");
    }

    #[test]
    fn unique_slug_suffixes_on_collision() {
        // "what am i avoiding in life today" → first 6 words → "what-am-i-avoiding-in-life".
        let plain = derive_slug_unique("what am i avoiding in life today", |_| false);
        assert_eq!(plain, "what-am-i-avoiding-in-life");
        let collided = derive_slug_unique(
            "what am i avoiding in life today",
            |s| s == "what-am-i-avoiding-in-life",
        );
        assert!(collided.starts_with("what-am-i-avoiding-in-life-"));
        assert_ne!(plain, collided);
    }

    #[test]
    fn slug_is_deterministic_same_text_same_slug() {
        assert_eq!(derive_slug("What am I avoiding?"), derive_slug("what am i avoiding"));
    }

    #[test]
    fn slug_truncates_to_six_words() {
        let s = derive_slug("one two three four five six seven eight");
        assert_eq!(s, "one-two-three-four-five-six");
    }

    #[test]
    fn empty_derivable_slug_falls_back_to_hash() {
        let s = derive_slug("???");
        assert!(!s.is_empty());
        assert_eq!(s, derive_slug("???")); // deterministic
    }

    #[test]
    fn short_hash_is_stable_and_short() {
        let a = short_hash("what am i avoiding");
        assert_eq!(a, short_hash("what am i avoiding"));
        assert_eq!(a.len(), 6);
        assert_ne!(a, short_hash("what am i grateful for"));
    }
}
