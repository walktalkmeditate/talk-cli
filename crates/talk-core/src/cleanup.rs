/// Cleanup intensity. Plan 3 wires this into the LLM rewrite; deterministic-Light
/// is the instant, always-present layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level { None, Light, Medium, High }

/// Words the GUARD treats as droppable (disfluencies + conversational filler), so a
/// rewrite that removes them still passes `guard_accepts`.
///
/// KNOWN LIMIT (Plan 3 code review): the guard drops these from BOTH sides of its
/// comparison, so it permits removing a *content* use of you/know/like/so/well/i/
/// mean anywhere (e.g. `guard_accepts("do you know the way", "do the way")` is
/// true). This is a moat for an LLM rewriter, not an instruction to remove them.
/// `deterministic_light` deliberately does NOT strip from this set — see
/// `LEADING_DISFLUENCIES`.
const FILLERS: &[&str] = &["um", "uh", "er", "ah", "like", "you", "know", "so", "well", "i", "mean"];

/// The ONLY words `deterministic_light` strips from the phrase start: non-lexical
/// vocalizations that are never content. The broader `FILLERS` set is NOT used
/// here — a leading `i`/`you`/`so`/`well`/`like` is almost always a real sentence
/// opener ("I think…", "You know what…", "So I realized…"), and dropping it
/// silently rewrites the user's meaning. Restraint wins: when a leading word is a
/// real dictionary word, keep it.
const LEADING_DISFLUENCIES: &[&str] = &["um", "uh", "er", "ah", "mm", "hmm", "uhm", "erm", "hm"];

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

/// Words/contractions whose deletion inverts meaning — never droppable by the
/// Medium/High guard.
const PINNED_NEGATIONS: &[&str] = &[
    "not", "never", "no", "none", "nor", "cannot", "neither", "nobody",
    "nothing", "nowhere", "without",
];

/// Count negations in `text`. Scans raw whitespace tokens (NOT `content_words`,
/// which splits on the apostrophe and so can never see "n't"): a token in
/// `PINNED_NEGATIONS` or ending in "n't" (can't, won't, isn't, …) counts.
fn negation_count(text: &str) -> usize {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase())
        .filter(|w| PINNED_NEGATIONS.contains(&w.as_str()) || w.ends_with("n't"))
        .count()
}

fn is_subsequence(needle: &[String], hay: &[String]) -> bool {
    let mut it = hay.iter();
    needle.iter().all(|w| it.any(|h| h == w))
}

/// The Medium/High moat: accept a rewrite that only DELETES content words (filler /
/// false starts) and reflows whitespace. The output's content words must be a
/// subsequence of the input's; no negation may be dropped (incl. contractions); and
/// at least 60% of content words must survive (a wholesale clause collapse fails
/// closed). KNOWN LIMIT: like `guard_accepts`, deleting a non-negation content word
/// such as a concessive "but" is permitted — the 60% budget backstops gross loss.
pub fn guard_accepts_deletions(input: &str, output: &str) -> bool {
    let inp = content_words(input);
    let out = content_words(output);
    if out.len() * 5 < inp.len() * 3 {
        return false; // out/inp < 0.6
    }
    if negation_count(output) < negation_count(input) {
        return false;
    }
    is_subsequence(&out, &inp)
}

/// Strip a chat model's conversational wrapper from a cleanup reply — a leading
/// "Sure, here…:"/"Here is…:" preface line, surrounding ``` fences, and one pair of
/// wrapping quotes — so a well-formed-but-prefixed reply isn't needlessly rejected
/// by the guard. Best-effort; the guard + Light fallback are the real safety net.
pub fn strip_model_preamble(text: &str) -> String {
    let mut s = text.trim();
    if let Some(rest) = s.strip_prefix("```") {
        s = rest.split_once('\n').map(|(_, r)| r).unwrap_or("").trim();
    }
    if let Some(rest) = s.strip_suffix("```") {
        s = rest.trim();
    }
    if let Some((first, rest)) = s.split_once('\n') {
        let lower = first.trim().to_lowercase();
        let prefaced = ["sure", "here is", "here's", "here are", "certainly", "okay", "ok"]
            .iter().any(|p| lower.starts_with(p));
        if prefaced && lower.ends_with(':') {
            s = rest.trim();
        }
    }
    for (open, close) in [('"', '"'), ('\'', '\''), ('\u{201c}', '\u{201d}')] {
        if s.starts_with(open) && s.ends_with(close) && s.chars().count() >= 2 {
            s = s[open.len_utf8()..s.len() - close.len_utf8()].trim();
            break;
        }
    }
    s.to_string()
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

/// Continuation function-words: common enough as sentence-internal openers that
/// lowercasing them when a sentence spans a pause is safe. Deliberately excludes
/// anything proper-noun-shaped — we never lowercase an arbitrary capitalized token.
const CONTINUATIONS: &[&str] = &[
    "and", "but", "so", "or", "the", "a", "an", "it", "that", "this", "these",
    "those", "all", "then", "because", "which", "who",
];

/// Lowercase the first letter of `text` when it CONTINUES the previous block —
/// the previous block didn't end a sentence (no terminal `.!?`) AND the first word
/// is an allow-listed continuation word. Whisper cases each segment as a fresh
/// sentence; this undoes the spurious mid-sentence capital when a sentence spans a
/// pause. Conservative by construction (only the allow-list; never a proper noun).
pub fn decapitalize_continuation(text: &str, prev_clean: Option<&str>) -> String {
    let continues = prev_clean.is_some_and(|p| {
        // Look past trailing closing quotes/brackets so `."` reads as terminated;
        // a Unicode ellipsis is a deliberate trail-off, also terminal.
        let tail = p.trim_end().trim_end_matches(['"', '\'', ')', ']', '”', '’']);
        !matches!(tail.chars().last(), Some('.' | '!' | '?' | '…') | None)
    });
    if !continues {
        return text.to_string();
    }
    let first = text.split_whitespace().next().unwrap_or("");
    let bare = first.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    if !CONTINUATIONS.contains(&bare.as_str()) {
        return text.to_string();
    }
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c.is_uppercase() => c.to_lowercase().collect::<String>() + chars.as_str(),
        _ => text.to_string(),
    }
}

/// Format a pass-2 Whisper revise. Whisper already cased + punctuated, so this does
/// NOT re-capitalize sentence starts or force terminal punctuation (that re-creates
/// the per-segment mid-sentence capital). It applies only the spoken-command and
/// `scratch that` backtrack features and the continuation de-capitalizer.
///
/// Order is DELIBERATELY the reverse of the commit path's
/// `apply_backtrack(apply_spoken_commands(raw))` (live.rs/session.rs/format.rs):
/// `apply_backtrack` ends with `split_whitespace().join(" ")`, which collapses the
/// `\n` that spoken commands insert. Running backtrack FIRST (on whitespace-only
/// text) and spoken commands LAST lets a spoken `new line` survive into the output.
/// Do not "tidy" this back to the commit order — it silently drops newlines.
pub fn format_revise(whisper: &str, prev_clean: Option<&str>) -> String {
    let pre = apply_spoken_commands(&apply_backtrack(whisper));
    decapitalize_continuation(&pre, prev_clean)
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
        // Strip trailing punctuation so "um," / "uh." still match the bare token.
        let lw = first.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        if LEADING_DISFLUENCIES.contains(&lw.as_str()) { words.remove(0); } else { break; }
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

/// Non-speech event words Whisper emits inside `(...)`. A parenthesized span is
/// removed only when EVERY inner word is in this set, so multi-word events
/// (`(wind blowing)`) strip while a real aside (`(I think)`) survives.
const SOUND_WORDS: &[&str] = &[
    "buzzer", "buzzing", "music", "applause", "applauding", "laughter", "laughs",
    "laughing", "coughs", "coughing", "cough", "sighs", "sigh", "beep", "beeping",
    "breathing", "breath", "breathes", "static", "noise", "silence", "blank_audio",
    "wind", "blowing", "clears", "throat", "typing", "footsteps", "door", "closes",
    "knock", "knocking", "indistinct", "inaudible", "sniffles", "chuckles",
];

/// Remove Whisper's non-speech tags. `[...]` spans go only when a known tag or an
/// all-caps event shape (`[BLANK_AUDIO]`); `(...)` spans go only when every inner
/// word is a sound word. Runs in the pre-layer (before the content-word guard), so
/// nothing it removes ever reaches the guard as a content-word change.
pub fn strip_sound_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(['(', '[']) {
        let open_ch = rest.as_bytes()[open];
        let close_ch = if open_ch == b'(' { ')' } else { ']' };
        let Some(rel) = rest[open + 1..].find(close_ch) else {
            out.push_str(&rest[..=open]);
            rest = &rest[open + 1..];
            continue;
        };
        let close = open + 1 + rel;
        let inner = rest[open + 1..close].trim();
        let remove = if open_ch == b'(' {
            is_all_sound_words(inner)
        } else {
            is_event_bracket(inner)
        };
        out.push_str(&rest[..open]);
        if !remove {
            out.push_str(&rest[open..=close]);
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_all_sound_words(inner: &str) -> bool {
    let mut any = false;
    for w in inner.split_whitespace() {
        any = true;
        let bare = w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        if !SOUND_WORDS.contains(&bare.as_str()) {
            return false;
        }
    }
    any
}

fn is_event_bracket(inner: &str) -> bool {
    if SOUND_WORDS.contains(&inner.to_lowercase().as_str()) {
        return true;
    }
    inner.contains('_')
        && inner.chars().any(|c| c.is_ascii_uppercase())
        && inner.chars().all(|c| c.is_ascii_uppercase() || c == '_' || c == ' ')
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
        Level::Light => "Fix only capitalization and punctuation, and drop leading non-lexical filler (um, uh, er, ah). Remove no other words.",
        Level::Medium => "Also remove disfluencies and false starts and join fragments into sentences. Keep every meaning-bearing word.",
        Level::High => "Also break into paragraphs at topic shifts. Keep every meaning-bearing word, in its original order, adding nothing.",
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
    fn does_not_strip_a_leading_content_word() {
        // The reported "cleaning up too much" bug: a leading subject pronoun or
        // discourse opener is CONTENT, not a disfluency — it must survive.
        assert_eq!(deterministic_light("i sometimes forget the small things"),
            "I sometimes forget the small things.");
        assert_eq!(deterministic_light("you should go now"), "You should go now.");
        assert_eq!(deterministic_light("so i realized the answer"), "So I realized the answer.");
        assert_eq!(deterministic_light("well that is the thing"), "Well that is the thing.");
    }

    #[test]
    fn still_strips_leading_nonlexical_disfluencies() {
        assert_eq!(deterministic_light("um uh the thing is"), "The thing is.");
        assert_eq!(deterministic_light("ah i see it now"), "I see it now.");
        // Trailing punctuation on the disfluency token must not shield it.
        assert_eq!(deterministic_light("um, the thing is"), "The thing is.");
    }

    #[test]
    fn a_leading_pure_punctuation_token_survives() {
        // It trims to "" which is not a disfluency, so the loop stops and the token
        // is kept — no panic, no over-strip. (Capitalization still lands on the
        // first real word.)
        assert_eq!(deterministic_light("-- the thing is"), "-- The thing is.");
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

    #[test]
    fn decapitalize_lowercases_an_allowlist_continuation_after_unterminated_prior() {
        assert_eq!(
            decapitalize_continuation("All these edge cases get sorted out.", Some("with their product")),
            "all these edge cases get sorted out."
        );
    }

    #[test]
    fn decapitalize_keeps_capital_after_a_terminated_prior() {
        assert_eq!(
            decapitalize_continuation("All these edge cases.", Some("That worked.")),
            "All these edge cases."
        );
    }

    #[test]
    fn decapitalize_never_lowercases_a_non_allowlist_word_protecting_proper_nouns() {
        assert_eq!(
            decapitalize_continuation("Whisper does the rest", Some("the tool i use is")),
            "Whisper does the rest"
        );
    }

    #[test]
    fn format_revise_trusts_whisper_casing_and_applies_features() {
        assert_eq!(format_revise("hello there", None), "hello there");
        assert_eq!(format_revise("first line new line second", None), "first line\nsecond");
    }

    #[test]
    fn strip_sound_tags_removes_known_parenthesized_and_collapses_space() {
        assert_eq!(strip_sound_tags("woke up (buzzer) early"), "woke up early");
        assert_eq!(strip_sound_tags("(wind blowing) i sat down"), "i sat down");
        assert_eq!(strip_sound_tags("then (clears throat) i spoke"), "then i spoke");
    }

    #[test]
    fn strip_sound_tags_removes_bracketed_events_only() {
        assert_eq!(strip_sound_tags("a [BLANK_AUDIO] b"), "a b");
        assert_eq!(strip_sound_tags("a [MUSIC] b"), "a b");
        // not an event shape → kept
        assert_eq!(strip_sound_tags("see note [7] here"), "see note [7] here");
        assert_eq!(strip_sound_tags("from [Smith] today"), "from [Smith] today");
    }

    #[test]
    fn strip_sound_tags_keeps_real_words_and_asides() {
        assert_eq!(strip_sound_tags("the buzzer rang"), "the buzzer rang"); // bare word kept
        assert_eq!(strip_sound_tags("it works (I think) well"), "it works (I think) well");
    }

    #[test]
    fn strip_sound_tags_keeps_user_acronyms_but_strips_whisper_events() {
        assert_eq!(strip_sound_tags("the [FBI] case"), "the [FBI] case");
        assert_eq!(strip_sound_tags("sign the [NDA] today"), "sign the [NDA] today");
        assert_eq!(strip_sound_tags("a [TODO] item"), "a [TODO] item");
        assert_eq!(strip_sound_tags("a [BLANK_AUDIO] b"), "a b");
        assert_eq!(strip_sound_tags("a [MUSIC] b"), "a b");
    }

    #[test]
    fn strip_sound_tags_keeps_an_unmatched_bracket() {
        assert_eq!(strip_sound_tags("hello (world"), "hello (world"); // unmatched open → kept
    }

    #[test]
    fn strip_sound_tags_skips_a_lone_opener_and_keeps_stripping() {
        assert_eq!(strip_sound_tags("a [ b (buzzer) c"), "a [ b c");
    }

    #[test]
    fn strip_sound_tags_removes_consecutive_tags() {
        assert_eq!(strip_sound_tags("(cough) (laughs) okay"), "okay");
    }

    #[test]
    fn strip_model_preamble_removes_preface_fences_quotes() {
        assert_eq!(strip_model_preamble("Sure, here is the cleaned text:\n\nThe real thing."), "The real thing.");
        assert_eq!(strip_model_preamble("```\nThe real thing.\n```"), "The real thing.");
        assert_eq!(strip_model_preamble("\"The real thing.\""), "The real thing.");
        assert_eq!(strip_model_preamble("Already clean."), "Already clean.");
    }

    #[test]
    fn deletions_guard_accepts_filler_removal_and_reflow() {
        assert!(guard_accepts_deletions("um so i mean the thing", "the thing"));
        assert!(guard_accepts_deletions("a b c", "a b\n\nc"));
    }

    #[test]
    fn deletions_guard_rejects_substitution_addition_reorder() {
        assert!(!guard_accepts_deletions("i love her", "i hate her"));
        assert!(!guard_accepts_deletions("i am tired", "i am very tired"));
        assert!(!guard_accepts_deletions("the cat sat", "sat the cat"));
    }

    #[test]
    fn deletions_guard_rejects_dropped_negations_including_contractions() {
        assert!(!guard_accepts_deletions("i am not sure", "i am sure"));
        assert!(!guard_accepts_deletions("i can't go", "i can go"));
        assert!(!guard_accepts_deletions("i won't do it", "i will do it"));
        assert!(!guard_accepts_deletions("there is no way", "there is way"));
    }

    #[test]
    fn deletions_guard_rejects_wholesale_clause_collapse() {
        assert!(!guard_accepts_deletions("the quick brown fox jumps over", "fox"));
    }
}
