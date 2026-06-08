#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level { None, Light, Medium, High }

/// Words the guard is allowed to see added/removed (disfluencies + filler).
const FILLERS: &[&str] = &["um", "uh", "er", "ah", "like", "you", "know", "so", "well", "i", "mean"];

fn content_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter(|w| !FILLERS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// The moat: accept a rewrite only if it preserves every content word from the
/// input, in order, adding/removing nothing but allowed fillers. Guards *harm*
/// (a substituted/dropped meaning word) rather than edit *volume*.
pub fn guard_accepts(input: &str, output: &str) -> bool {
    content_words(input) == content_words(output)
}

/// Apply spoken formatting commands deterministically.
pub fn apply_spoken_commands(text: &str) -> String {
    text.replace(" new paragraph ", "\n\n")
        .replace(" new line ", "\n")
        .replace(" period ", ". ")
        .replace(" comma ", ", ")
}

/// Remove a self-correction: when a backtrack trigger appears, drop the words
/// immediately preceding it (the spec's >3-word-reduction guard: only fire when
/// at least 3 words precede the trigger, so we don't nuke a short true clause).
pub fn apply_backtrack(text: &str) -> String {
    const TRIGGERS: &[&str] = &["scratch that", "actually no"];
    let mut result = text.to_string();
    for trigger in TRIGGERS {
        while let Some(pos) = result.to_lowercase().find(trigger) {
            let before = result[..pos].trim_end();
            let after = &result[pos + trigger.len()..];
            let kept: Vec<&str> = before.split_whitespace().collect();
            if kept.len() >= 3 {
                // Drop everything back to the previous sentence boundary.
                let cut = before.rfind(['.', '\n']).map(|i| i + 1).unwrap_or(0);
                result = format!("{}{}", &before[..cut], after);
            } else {
                // Too short to be a real correction — just remove the trigger.
                result = format!("{} {}", before, after.trim_start());
            }
        }
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Deterministic "Light": capitalize sentence starts, ensure terminal
/// punctuation, strip leading fillers. Always guard-safe by construction.
pub fn deterministic_light(text: &str) -> String {
    let trimmed = text.trim();
    let without_lead = strip_leading_fillers(trimmed);
    let capped = capitalize_sentences(&without_lead);
    ensure_terminal(&capped)
}

fn strip_leading_fillers(text: &str) -> String {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    while let Some(first) = words.first() {
        let lw = first.to_lowercase();
        if FILLERS.contains(&lw.as_str()) { words.remove(0); } else { break; }
    }
    words.join(" ")
}

fn capitalize_sentences(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_start = true;
    for ch in text.chars() {
        if at_start && ch.is_alphabetic() {
            out.extend(ch.to_uppercase());
            at_start = false;
        } else {
            out.push(ch);
            if ch == '.' || ch == '!' || ch == '?' { at_start = true; }
        }
    }
    out
}

fn ensure_terminal(text: &str) -> String {
    let t = text.trim_end();
    if t.is_empty() || matches!(t.chars().last(), Some('.') | Some('!') | Some('?')) {
        t.to_string()
    } else {
        format!("{}.", t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_pure_punctuation_and_filler_cleanup() {
        assert!(guard_accepts(
            "um so the thing is i keep avoiding it",
            "The thing is, I keep avoiding it.",
        ));
    }

    #[test]
    fn rejects_a_substituted_meaning_word() {
        // "love" -> "loathe": tiny edit distance, catastrophic meaning change.
        assert!(!guard_accepts("i love her", "I loathe her."));
    }

    #[test]
    fn rejects_a_dropped_content_word() {
        assert!(!guard_accepts("i never said that", "I said that."));
    }

    #[test]
    fn rejects_an_added_content_word() {
        assert!(!guard_accepts("i am tired", "I am very tired."));
    }

    #[test]
    fn deterministic_light_caps_and_terminates() {
        assert_eq!(deterministic_light("um the thing is"), "The thing is.");
    }

    #[test]
    fn deterministic_light_is_guard_safe() {
        let raw = "um so i keep avoiding the hard conversation";
        assert!(guard_accepts(raw, &deterministic_light(raw)));
    }

    #[test]
    fn spoken_command_becomes_newline() {
        assert_eq!(apply_spoken_commands("a new line b"), "a\nb");
    }

    #[test]
    fn backtrack_drops_preceding_clause() {
        let out = apply_backtrack("the answer is yes scratch that the answer is no");
        assert!(!out.contains("yes"));
        assert!(out.contains("the answer is no"));
    }
}
