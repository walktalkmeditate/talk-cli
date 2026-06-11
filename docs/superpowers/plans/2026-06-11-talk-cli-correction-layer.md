# Correction Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Strip Whisper non-speech tags and apply a user-owned personal lexicon to transcribed text, on both the live mic path and the `--from-text` path, leaving the verbatim raw untouched.

**Architecture:** Two pure functions in `talk-core` (`strip_sound_tags`, `apply_lexicon`); a binary-side `Lexicon` loader (`src/lexicon.rs`) reading `~/.config/talk/lexicon.toml`; a `correct(raw, &Lexicon)` helper applied to the clean path in `live.rs::apply_event` (primary) and `session.rs::run` (secondary), before the existing cleanup. The original `raw` is always what gets stored.

**Tech Stack:** Rust, `toml` + `serde` (existing deps). No new dependencies, no model, no network.

Spec: `docs/superpowers/specs/2026-06-11-talk-cli-correction-layer-design.md`

---

## File Structure

- `crates/talk-core/src/cleanup.rs` — add `strip_sound_tags` + its `SOUND_WORDS` set (pure).
- `crates/talk-core/src/lexicon.rs` — NEW. `apply_lexicon(text, &[(String,String)])` (pure, single-pass, word-bounded).
- `crates/talk-core/src/lib.rs` — add `pub mod lexicon;`.
- `src/lexicon.rs` — NEW (binary). `Lexicon` (loads `[corrections]`, holds the longest-first slice), `correct(raw, &Lexicon)`, `template()`.
- `src/main.rs` — add `mod lexicon;`; load the lexicon before the live `Screen`; thread into `LiveConfig`/`RunConfig`; `handle_config` init writes `lexicon.toml`.
- `src/paths.rs` — add `lexicon_path()`.
- `src/live.rs` — `LiveConfig.lexicon`; thread through `run_loop`/`drain_until_done`/`apply_event`; apply `correct` in the Commit/Revise arms.
- `src/session.rs` — `RunConfig.lexicon`; apply `correct` in the Commit/Revise arms.

---

### Task 1: `strip_sound_tags` (talk-core, pure)

**Files:**
- Modify: `crates/talk-core/src/cleanup.rs` (add function + `SOUND_WORDS` + tests)

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/talk-core/src/cleanup.rs`:

```rust
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
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p talk-core strip_sound_tags`
Expected: FAIL — `cannot find function strip_sound_tags`.

- [ ] **Step 3: Implement**

Add to `crates/talk-core/src/cleanup.rs` (after the `apply_spoken_commands` group):

```rust
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
        let Some(rel) = rest[open + 1..].find(close_ch) else { break }; // unmatched → stop
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
    // all-caps event shape: only A-Z, '_', or space, with at least one A-Z.
    let mut has_alpha = false;
    for c in inner.chars() {
        if c.is_ascii_uppercase() {
            has_alpha = true;
        } else if c != '_' && c != ' ' {
            return false;
        }
    }
    has_alpha
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p talk-core strip_sound_tags`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/talk-core/src/cleanup.rs
git commit -m "feat(core): strip_sound_tags — remove Whisper non-speech tags"
```

---

### Task 2: `apply_lexicon` (talk-core, pure, NEW module)

**Files:**
- Create: `crates/talk-core/src/lexicon.rs`
- Modify: `crates/talk-core/src/lib.rs` (add `pub mod lexicon;`)

- [ ] **Step 1: Create the module with failing tests**

Create `crates/talk-core/src/lexicon.rs`:

```rust
//! Personal-lexicon substitution (pure). The user authorizes these meaning
//! changes, so they run in the pre-layer BEFORE the content-word guard. Single
//! left-to-right pass over the original input: substituted output is never
//! re-scanned, so cyclic and value-contains-key maps terminate.

/// Apply word-bounded, case-insensitive substitutions. `corrections` MUST be
/// sorted by descending key length (longest-first), so the longest matching key
/// wins at each position. The value is emitted as written (later sentence-start
/// capitalization by `deterministic_light` may still re-case it — that is accepted).
pub fn apply_lexicon(text: &str, corrections: &[(String, String)]) -> String {
    if corrections.is_empty() {
        return text.to_string();
    }
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let at_word_start = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        let hit = if at_word_start {
            corrections.iter().find_map(|(key, val)| {
                let kb = key.as_bytes();
                let end = i + kb.len();
                let bounded_after = end == b.len() || !b[end].is_ascii_alphanumeric();
                if !kb.is_empty() && end <= b.len()
                    && b[i..end].eq_ignore_ascii_case(kb) && bounded_after
                {
                    Some((kb.len(), val.as_str()))
                } else {
                    None
                }
            })
        } else {
            None
        };
        match hit {
            Some((klen, val)) => {
                out.push_str(val);
                i += klen; // advance past the matched span — never re-scan output
            }
            None => {
                let ch = text[i..].chars().next().expect("i on a char boundary");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        let mut p: Vec<(String, String)> =
            v.iter().map(|(k, val)| (k.to_string(), val.to_string())).collect();
        p.sort_by(|a, b| b.0.len().cmp(&a.0.len())); // longest-first
        p
    }

    #[test]
    fn substitutes_whole_words_only() {
        let c = pairs(&[("TOC", "talk")]);
        assert_eq!(apply_lexicon("open TOC now", &c), "open talk now");
        assert_eq!(apply_lexicon("buy STOCK today", &c), "buy STOCK today"); // not inside a word
    }

    #[test]
    fn matches_case_insensitively_value_as_written() {
        let c = pairs(&[("toc", "talk")]);
        assert_eq!(apply_lexicon("TOC Toc toc", &c), "talk talk talk");
    }

    #[test]
    fn longest_key_wins() {
        let c = pairs(&[("talk", "X"), ("talk CLI", "talk")]);
        assert_eq!(apply_lexicon("the talk CLI rocks", &c), "the talk rocks");
    }

    #[test]
    fn single_pass_terminates_on_cyclic_and_value_contains_key() {
        let cyclic = pairs(&[("a", "b"), ("b", "a")]);
        assert_eq!(apply_lexicon("a b", &cyclic), "b a"); // each swapped once, no loop
        let contains = pairs(&[("cloth", "Claude")]);
        assert_eq!(apply_lexicon("cloth", &contains), "Claude"); // "Cl..." not re-scanned
    }

    #[test]
    fn empty_corrections_is_identity() {
        assert_eq!(apply_lexicon("nothing changes", &[]), "nothing changes");
    }
}
```

- [ ] **Step 2: Register the module and run to verify failure**

Add `pub mod lexicon;` to `crates/talk-core/src/lib.rs` (alongside the other `pub mod` lines).

Run: `cargo test -p talk-core lexicon::`
Expected: PASS once the module compiles — but first confirm it FAILS before the impl by temporarily emptying the function body if desired. (The impl is included above; if practicing strict TDD, write the tests first, see the compile error, then paste the function.)

- [ ] **Step 3: (impl already shown in Step 1)**

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p talk-core lexicon::`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/talk-core/src/lexicon.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): apply_lexicon — single-pass word-bounded substitution"
```

---

### Task 3: binary `Lexicon` loader + `correct` helper + template

**Files:**
- Create: `src/lexicon.rs`
- Modify: `src/main.rs` (add `mod lexicon;` near the other `mod` declarations)

- [ ] **Step 1: Write the failing tests**

Create `src/lexicon.rs`:

```rust
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
```

- [ ] **Step 2: Register the module and run to verify**

Add `mod lexicon;` to `src/main.rs` near the other top-level `mod` declarations (e.g. beside `mod config;`).

Run: `cargo test lexicon::tests`
Expected: PASS (3 tests).

- [ ] **Step 3: (impl shown in Step 1)** — n/a

- [ ] **Step 4: Confirm the whole crate still builds**

Run: `cargo build`
Expected: success.

- [ ] **Step 5: Commit**

```bash
git add src/lexicon.rs src/main.rs
git commit -m "feat: binary Lexicon loader + correct() helper + commented template"
```

---

### Task 4: `lexicon_path()` in paths.rs

**Files:**
- Modify: `src/paths.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/paths.rs`:

```rust
#[test]
fn lexicon_path_is_config_dir_lexicon_toml() {
    assert!(lexicon_path().ends_with("talk/lexicon.toml"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p talk-cli lexicon_path` (or `cargo test lexicon_path`)
Expected: FAIL — `cannot find function lexicon_path`.

- [ ] **Step 3: Implement**

Add to `src/paths.rs` next to `config_dir`:

```rust
/// Where the personal lexicon lives: `<config_dir>/lexicon.toml`.
pub fn lexicon_path() -> PathBuf {
    config_dir().join("lexicon.toml")
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test lexicon_path`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/paths.rs
git commit -m "feat: lexicon_path() under the XDG config dir"
```

---

### Task 5: wire `correct` into the live mic path (`live.rs`)

**Files:**
- Modify: `src/live.rs` (`LiveConfig`, `apply_event`, `run_loop`, `drain_until_done`, and the existing `apply_event` tests)

- [ ] **Step 1: Add the failing integration test**

Add to `#[cfg(test)] mod tests` in `src/live.rs` (the `guards` helper already exists there):

```rust
#[test]
fn commit_applies_lexicon_and_strips_tags_to_clean_only() {
    let lex = crate::lexicon::Lexicon::from_map(
        [("TOC".to_string(), "talk".to_string())].into_iter().collect(),
    );
    let mut settle = Settle::new();
    let mut g = guards(false, false);
    apply_event(Event::Commit("open TOC (buzzer)".into()), &mut g, &mut settle, &lex);
    settle.finalize();
    let block = settle.settled().last().unwrap();
    assert!(block.clean.to_lowercase().contains("open talk"));
    assert!(!block.clean.contains("buzzer"));
    assert_eq!(block.raw, "open TOC (buzzer)"); // raw is verbatim
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test commit_applies_lexicon`
Expected: FAIL — `apply_event` takes 3 args, not 4 (and `crate::lexicon::Lexicon` path mismatch until wired).

- [ ] **Step 3: Thread the lexicon and apply `correct`**

In `src/live.rs`:

(a) Add to `LiveConfig`:

```rust
pub struct LiveConfig<'a> {
    pub mode: RMode,
    pub question: Option<&'a str>,
    pub held_label: Option<&'a str>,
    pub cleanup: &'a str,
    pub ephemeral: bool,
    pub palette: talk_core::palette::Palette,
    pub lexicon: &'a crate::lexicon::Lexicon,
}
```

(b) Change `apply_event`'s signature and its Commit/Revise arms:

```rust
fn apply_event(ev: Event, g: &mut EventGuards, settle: &mut Settle, lexicon: &crate::lexicon::Lexicon) -> bool {
    match ev {
        Event::Done => return true,
        Event::Revise(raw2) if !g.commit_dropped => {
            let prev = settle.settled().last().map(|b| b.clean.clone());
            let clean = talk_core::cleanup::format_revise(
                &crate::lexicon::correct(&raw2, lexicon), prev.as_deref());
            settle.revise_committing(&raw2, &clean);
        }
        Event::Revise(_) => {}
        Event::Commit(_) if g.paused => { g.commit_dropped = true; }
        _ if g.paused => {}
        Event::Commit(raw) => {
            let corrected = crate::lexicon::correct(&raw, lexicon);
            let pre = talk_core::cleanup::apply_backtrack(
                &talk_core::cleanup::apply_spoken_commands(&corrected));
            settle.commit(&raw, &talk_core::cleanup::deterministic_light(&pre));
            g.commit_dropped = false;
        }
        Event::Partial(p) => settle.on_partial(&p),
    }
    false
}
```

(c) Update the two `apply_event(ev, &mut guards, &mut settle)` call sites — in `run_loop`'s drain loop and in `drain_until_done` — to pass the lexicon. Thread it via `cfg.lexicon` in `run_loop`, and add a `lexicon: &crate::lexicon::Lexicon` parameter to `drain_until_done`, passing `cfg.lexicon` at its call site in `run_loop`:

```rust
// run_loop drain loop:
if apply_event(ev, &mut guards, &mut settle, cfg.lexicon) { finished = true; break; }
// run_loop [space] handler:
drain_until_done(source, &mut settle, &mut guards, cfg.lexicon)?;
```

```rust
fn drain_until_done(
    source: &mut dyn TranscriptSource,
    settle: &mut Settle,
    guards: &mut EventGuards,
    lexicon: &crate::lexicon::Lexicon,
) -> std::io::Result<()> {
    // ...unchanged body, except the apply_event call:
    if apply_event(ev, guards, settle, lexicon) { break; }
    // ...
}
```

(d) Update the EXISTING `apply_event(...)` calls in the test module to pass an empty lexicon (identity, so behavior is unchanged). Add a local helper at the top of the test module and use it:

```rust
fn empty_lex() -> crate::lexicon::Lexicon {
    crate::lexicon::Lexicon::from_map(std::collections::BTreeMap::new())
}
```

Then change **every** existing `apply_event(<ev>, &mut g, &mut settle)` call in the test module to `apply_event(<ev>, &mut g, &mut settle, &empty_lex())` — there are ~10 of them across the pairing/pause/drain tests; a missed one is an arity-mismatch compile error. (Each `&empty_lex()` is a statement-scoped temporary, which compiles fine; the value-returning helper is intentionally simpler than session.rs's `cfg`, which must `Box::leak` because it *returns* an owned `RunConfig` that outlives the call.)

- [ ] **Step 4: Run to verify pass**

Run: `cargo test live::tests`
Expected: PASS — the new test plus all existing pairing/pause tests (now passing an empty lexicon).

- [ ] **Step 5: Commit**

```bash
git add src/live.rs
git commit -m "feat(live): apply lexicon + sound-tag strip on the mic path (raw stays verbatim)"
```

---

### Task 6: wire `correct` into the `--from-text` path (`session.rs`)

**Files:**
- Modify: `src/session.rs` (`RunConfig`, the Commit/Revise arms, and the test `cfg` helper)

- [ ] **Step 1: Add the failing test**

Add to `#[cfg(test)] mod tests` in `src/session.rs`:

```rust
#[test]
fn run_applies_lexicon_to_clean_keeps_raw_verbatim() {
    let lex = crate::lexicon::Lexicon::from_map(
        [("TOC".to_string(), "talk".to_string())].into_iter().collect(),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut src = FakeTranscript::new(vec![
        Event::Commit("open TOC (buzzer)".into()),
        Event::Done,
    ]);
    let cfg = RunConfig {
        base: dir.path(), date: "2026-06-08", time: "08:14", keep_raw: true,
        raw_sidecar: false, ephemeral: false,
        formatter: &talk_core::format::DeterministicFormatter, level: Level::Light,
        lexicon: &lex,
    };
    let p = run(&mut src, Target::Journal, &cfg).unwrap().unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("Open talk."));
    assert!(!text.contains("buzzer"));
    assert!(text.contains("<!-- raw: open TOC (buzzer) -->"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test run_applies_lexicon`
Expected: FAIL — `RunConfig` has no `lexicon` field.

- [ ] **Step 3: Implement**

In `src/session.rs`:

(a) Add the field to `RunConfig`:

```rust
pub struct RunConfig<'a> {
    pub base: &'a Path,
    pub date: &'a str,
    pub time: &'a str,
    pub keep_raw: bool,
    pub raw_sidecar: bool,
    pub ephemeral: bool,
    pub formatter: &'a dyn Formatter,
    pub level: Level,
    pub lexicon: &'a crate::lexicon::Lexicon,
}
```

(b) Apply `correct` in the Commit and Revise arms of `run`:

```rust
Event::Commit(raw) => {
    let clean = guarded_format(cfg.formatter, cfg.level, &crate::lexicon::correct(&raw, cfg.lexicon));
    settle.commit(&raw, &clean);
}
Event::Revise(raw2) => {
    let prev = settle.settled().last().map(|b| b.clean.clone());
    let clean2 = talk_core::cleanup::format_revise(
        &crate::lexicon::correct(&raw2, cfg.lexicon), prev.as_deref());
    settle.revise_committing(&raw2, &clean2);
}
```

(c) Update the existing test `cfg` helper (and the two inline `RunConfig { ... }` literals in the test module) to include `lexicon`. Add a shared empty lexicon to the test `cfg` helper:

```rust
fn cfg(base: &Path, ephemeral: bool) -> RunConfig<'_> {
    // NOTE: leak an empty lexicon so the returned RunConfig can borrow it for 'static-ish test use.
    let lex: &'static crate::lexicon::Lexicon = Box::leak(Box::new(
        crate::lexicon::Lexicon::from_map(std::collections::BTreeMap::new())));
    RunConfig {
        base, date: "2026-06-08", time: "08:14", keep_raw: true, raw_sidecar: false, ephemeral,
        formatter: &talk_core::format::DeterministicFormatter, level: Level::Light, lexicon: lex,
    }
}
```

There are two inline `RunConfig { ... }` literals in the test module — they differ in lifetime and must be edited differently:

- **`spoken_command_words_do_not_survive`** builds the `RunConfig` *inline as an argument* to `run(...)` in a single statement, so a borrowed temporary lives to the end of the statement. Add the field inline: `lexicon: &crate::lexicon::Lexicon::from_map(std::collections::BTreeMap::new()),`.
- **`an_over_editing_formatter_cannot_corrupt_the_file`** does `let cfg = RunConfig { ... };` and uses `cfg` on a *later* line. An inline `&...from_map(...)` temporary would be dropped at the `let`'s semicolon → `error[E0716]: temporary value dropped while borrowed`. Bind it first:

```rust
let lex = crate::lexicon::Lexicon::from_map(std::collections::BTreeMap::new());
let cfg = RunConfig {
    base: dir.path(), date: "2026-06-08", time: "08:14", keep_raw: true, raw_sidecar: false, ephemeral: false,
    formatter: &Flip, level: Level::Light, lexicon: &lex,
};
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test session::tests`
Expected: PASS — the new test plus all existing `run` tests.

- [ ] **Step 5: Commit**

```bash
git add src/session.rs
git commit -m "feat(session): apply lexicon + sound-tag strip on the --from-text path"
```

---

### Task 7: load the lexicon in main.rs + `config init` writes `lexicon.toml`

**Files:**
- Modify: `src/main.rs` (`run_live_session`, `run_and_report`, `handle_config`)

- [ ] **Step 1: (coverage note)**

The template-parses-empty behavior is already covered by Task 3's `shipped_template_parses_to_an_empty_map`, and the missing-file → empty behavior by `Lexicon::load`. No new test is needed here; this task is pure wiring, verified end-to-end by the smoke test in Step 4.

- [ ] **Step 2: Update `handle_config` init arm**

In `src/main.rs::handle_config`, replace the `Some("init")` arm:

```rust
Some("init") => {
    if let Some(dir) = p.parent() { paths::ensure_base_dir(dir)?; }
    paths::write_private(&p, &config::Config::commented_template())?;
    let lp = paths::lexicon_path();
    paths::write_private(&lp, lexicon::template())?;
    println!("wrote {} and {}", p.display(), lp.display());
}
```

- [ ] **Step 3: Load + thread the lexicon into both session entry points**

(a) In `run_and_report` (the `--from-text` path), load the lexicon and pass it into `RunConfig`:

```rust
fn run_and_report(r: Report) -> std::io::Result<()> {
    let lexicon = lexicon::Lexicon::load(&paths::lexicon_path());
    let path = run(&mut FakeTranscript::from_text(r.text), r.target,
        &RunConfig {
            base: r.base, date: r.date, time: r.time, keep_raw: r.keep_raw,
            raw_sidecar: r.raw_sidecar, ephemeral: r.ephemeral,
            formatter: &talk_core::format::DeterministicFormatter, level: r.level,
            lexicon: &lexicon,
        })?;
    // ...rest unchanged...
}
```

(b) In `run_live_session`, load the lexicon BEFORE constructing `live_cfg` (which is before `run_loop`/`Screen`), and add it to `LiveConfig`:

```rust
let lexicon = lexicon::Lexicon::load(&paths::lexicon_path());
let live_cfg = live::LiveConfig {
    mode: rmode, question, held_label: held_label.as_deref(), cleanup, ephemeral, palette,
    lexicon: &lexicon,
};
```

- [ ] **Step 4: Run the full suite + a manual smoke**

Run: `cargo test`
Expected: PASS (all tests, default features).

Run: `cargo build && env HOME=/tmp/lex-smoke ./target/debug/talk config init`
Expected: prints `wrote /tmp/lex-smoke/.config/talk/config.toml and /tmp/lex-smoke/.config/talk/lexicon.toml`.

Run: `printf '[corrections]\n"TOC" = "talk"\n' > /tmp/lex-smoke/.config/talk/lexicon.toml && env HOME=/tmp/lex-smoke ./target/debug/talk journal --from-text "open TOC (buzzer) now"`
Expected: the written entry contains `Open talk now.` (journal → DeterministicFormatter → `deterministic_light` caps the sentence start; no `TOC`, no `(buzzer)`); the `<!-- raw: -->` comment keeps `open TOC (buzzer) now`. (Inspect `/tmp/lex-smoke/talk/`.)

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: load lexicon before the live screen; config init writes lexicon.toml"
```

---

## Notes for the implementer

- **`--from-text` flag:** the smoke test in Task 7 assumes `talk journal --from-text <text>` exists (it routes through `run_and_report`). Confirm the exact flag name in `src/cli.rs` and adjust the command if it differs.
- **Raw verbatim is sacred:** every `settle.commit`/`settle.revise_committing` call passes the ORIGINAL `raw`/`raw2`, never the corrected text. The `correct(...)` result feeds only the `clean` argument. Do not "simplify" by correcting the stored raw.
- **No new dependencies:** `toml` and `serde` are already in both `Cargo.toml` files. If `cargo build` complains about `serde::Deserialize` in `src/lexicon.rs`, confirm `serde`'s `derive` feature is enabled in the binary `Cargo.toml` (it is — used by `config.rs`).
