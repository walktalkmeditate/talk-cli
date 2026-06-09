/// Cleanup intensity. Plan 3 wires this into the LLM rewrite; deterministic-Light
/// is the instant, always-present layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level { None, Light, Medium, High }

/// Words the guard treats as droppable (disfluencies + conversational filler), so
/// `deterministic_light`'s leading-filler strip stays guard-safe.
///
/// KNOWN LIMIT (Plan 3 code review): the guard drops these from BOTH sides of its
/// comparison, so it permits removing a *content* use of you/know/like/so/well/i/
/// mean anywhere (e.g. `guard_accepts("do you know the way", "do the way")` is
/// true). `deterministic_light` only strips LEADING fillers, so it never triggers
/// this — but an LLM (T7) told to remove mid-sentence filler could drop content
/// and pass the guard. `rewrite_prompt`'s Light rule therefore asks only for
/// LEADING filler removal; the real fix (position-aware / leading-only handling)
/// is deferred to the subsequence-guard work.
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

/// Apply spoken formatting commands deterministically. Padding the input with
/// spaces lets a command at the phrase start or end match too (the replacements
/// are space-delimited). Note: back-to-back identical commands ("new line new
/// line") collapse to one — an accepted Plan-1 edge case.
pub fn apply_spoken_commands(text: &str) -> String {
    format!(" {} ", text)
        .replace(" new paragraph ", "\n\n")
        .replace(" new line ", "\n")
        .replace(" period ", ". ")
        .replace(" comma ", ", ")
        .trim()
        .to_string()
}

/// Find `needle_lower` (lowercase ASCII) in `hay` at word boundaries, returning
/// a byte offset valid in `hay`. Case-insensitive without lowercasing `hay`, so
/// the offset never lands mid-codepoint (a prior bug: offsets from a lowercased
/// copy were sliced against the original and panicked on case-shrinking chars).
fn find_word_bounded(hay: &str, needle_lower: &str) -> Option<usize> {
    let hb = hay.as_bytes();
    let nb = needle_lower.as_bytes();
    let nlen = nb.len();
    if nlen == 0 || hb.len() < nlen { return None; }
    let mut i = 0;
    while i + nlen <= hb.len() {
        // ASCII needle bytes can only match ASCII haystack bytes, so a match
        // always starts/ends on a char boundary.
        if (0..nlen).all(|k| hb[i + k].to_ascii_lowercase() == nb[k]) {
            let before_ok = i == 0 || !hb[i - 1].is_ascii_alphanumeric();
            let after = i + nlen;
            let after_ok = after == hb.len() || !hb[after].is_ascii_alphanumeric();
            if before_ok && after_ok { return Some(i); }
        }
        i += 1;
    }
    None
}

/// Remove a self-correction: when a backtrack trigger appears AS A WHOLE PHRASE,
/// drop the words immediately preceding it (the spec's >3-word-reduction guard:
/// only fire when at least 3 words precede the trigger, so we don't nuke a short
/// true clause). Word-bounded so it never deletes content words it matched inside.
pub fn apply_backtrack(text: &str) -> String {
    const TRIGGERS: &[&str] = &["scratch that", "actually no"];
    let mut result = text.to_string();
    for trigger in TRIGGERS {
        while let Some(pos) = find_word_bounded(&result, trigger) {
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
    ensure_terminal(&capitalize_standalone_i(&capped))
}

/// Capitalize a standalone `i` (and its contractions — the `'` after it is a
/// non-alphanumeric boundary, so `i'm`/`i'll` qualify) anywhere in the phrase.
/// The Plan-3 T1 spike showed this is the LLM's main visible improvement over the
/// deterministic layer — and it's free, and invisible to the case-insensitive
/// content-word guard.
fn capitalize_standalone_i(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (idx, &ch) in chars.iter().enumerate() {
        let alone_before = idx == 0 || !chars[idx - 1].is_alphanumeric();
        let alone_after = idx + 1 == chars.len() || !chars[idx + 1].is_alphanumeric();
        out.push(if ch == 'i' && alone_before && alone_after { 'I' } else { ch });
    }
    out
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

/// Parse a config string into a `Level` (defaults to Light — the safe, restrained
/// default — on anything unrecognized).
pub fn parse_level(s: &str) -> Level {
    match s.trim().to_lowercase().as_str() {
        "none" => Level::None,
        "medium" => Level::Medium,
        "high" => Level::High,
        _ => Level::Light,
    }
}

/// The constrained-rewrite prompt for the LLM formatter (consumed by the Candle
/// façade in T7). `system` is hard restraint that holds at every level; the
/// per-level rule only *widens* which edits are permitted. Restraint is the
/// wording, so it lives here in the pure core, not in the inference façade.
pub struct RewritePrompt {
    pub system: String,
    pub user: String,
}

/// Build the per-level rewrite prompt for T7's Candle façade. The Light rule keeps
/// filler removal to LEADING disfluencies only — mid-sentence `you know`/`i mean`
/// removal is deliberately NOT requested, because the content-word guard would
/// accept such drops (see the `FILLERS` note). T7 must preserve this restriction.
pub fn rewrite_prompt(level: Level, text: &str) -> RewritePrompt {
    let restraint = "You clean up raw voice transcripts. Return ONLY the cleaned text, nothing else — no preamble, no quotes. NEVER change meaning: never swap a word for a different one, never add words that change meaning, never drop a negation, never reorder clauses. When unsure, leave it as it is.";
    let rule = match level {
        Level::None => "Return the text exactly as given.",
        Level::Light => "Fix only capitalization and punctuation, and drop leading filler (um, uh, like). Remove no other words.",
        Level::Medium => "Also remove disfluencies and false starts and join fragments into sentences. Keep every meaning-bearing word.",
        Level::High => "Also break into paragraphs at topic shifts and turn spoken lists into bullets. Keep every meaning-bearing word.",
    };
    RewritePrompt {
        system: format!("{restraint} {rule}"),
        user: format!("Clean this transcript:\n{text}"),
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
    fn guard_permits_dropping_filler_homographs_known_limit() {
        // Documents the Plan-3 review limit: filler-set words can be dropped even
        // as content. deterministic_light never does this (leading-only); the T7
        // LLM prompt must not request mid-sentence filler removal.
        assert!(guard_accepts("do you know the way", "do the way"));
        assert!(guard_accepts("i like it a lot", "it a lot"));
    }

    #[test]
    fn deterministic_light_caps_and_terminates() {
        assert_eq!(deterministic_light("um the thing is"), "The thing is.");
    }

    #[test]
    fn standalone_i_is_capitalized_mid_sentence() {
        assert_eq!(
            deterministic_light("the thing is i keep avoiding it"),
            "The thing is I keep avoiding it."
        );
        assert_eq!(
            deterministic_light("i'm sure i'll try what i've found"),
            "I'm sure I'll try what I've found."
        );
        // never inside words
        assert_eq!(deterministic_light("it is in the bin"), "It is in the bin.");
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

    #[test]
    fn backtrack_does_not_fire_inside_a_word() {
        // "actually no" must NOT match inside "actually nobody" (word-bounded).
        let out = apply_backtrack("well actually nobody knows the truth");
        assert!(out.contains("nobody"));
        assert!(out.contains("the truth"));
    }

    #[test]
    fn spoken_command_at_phrase_start_and_end() {
        assert_eq!(apply_spoken_commands("new line b"), "b");
        assert_eq!(apply_spoken_commands("a new line"), "a");
    }

    #[test]
    fn backtrack_handles_non_ascii_without_panicking() {
        // 'ẞ' lowercases to fewer bytes; offsets must stay on char boundaries.
        let out = apply_backtrack("aa bb ẞ scratch that ẞ tail");
        assert!(out.contains("tail"));
        assert!(!out.contains("scratch that"));
    }

    #[test]
    fn parse_level_maps_known_and_defaults_to_light() {
        assert_eq!(parse_level("none"), Level::None);
        assert_eq!(parse_level("Medium"), Level::Medium);
        assert_eq!(parse_level("HIGH"), Level::High);
        assert_eq!(parse_level("light"), Level::Light);
        assert_eq!(parse_level("nonsense"), Level::Light);
    }

    #[test]
    fn rewrite_prompt_widens_by_level_and_carries_the_text() {
        assert!(rewrite_prompt(Level::Light, "x").system.to_lowercase().contains("capitalization"));
        assert!(rewrite_prompt(Level::Medium, "x").system.to_lowercase().contains("disfluencies"));
        assert!(rewrite_prompt(Level::High, "x").system.to_lowercase().contains("paragraph"));
        assert!(rewrite_prompt(Level::Light, "the raw phrase").user.contains("the raw phrase"));
    }

    #[test]
    fn rewrite_prompt_always_states_the_restraint() {
        for lvl in [Level::Light, Level::Medium, Level::High] {
            assert!(rewrite_prompt(lvl, "x").system.to_lowercase().contains("never change meaning"));
        }
    }
}
