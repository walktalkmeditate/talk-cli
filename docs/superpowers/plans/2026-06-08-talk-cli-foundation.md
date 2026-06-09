# talk-cli Foundation Implementation Plan (Plan 1 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `talk-core` pure engine and `talk` binary's file/persistence layer so that `talk reflect`/`talk journal`/`talk unburden` produce correct markdown files end-to-end — driven by a *fake transcript source*, with no microphone, STT, or LLM yet.

**Architecture:** A Cargo workspace mirroring `meditate-cli`: a pure, dependency-light `talk-core` crate (id/slug, frontmatter, entry append, settle state machine, deterministic cleanup + diff-guard, selection, palette) and a `talk` binary that owns I/O (config, paths, file writing, CLI). All transcript input goes through a `TranscriptSource` trait; this plan ships a `FakeTranscript` impl so the entire pipeline is unit- and integration-testable. The real `listen`/`format`/`render` façades are Plans 2–3.

**Tech Stack:** Rust 1.82 (MSRV, matching meditate), `clap` 4 (derive), `serde`, `toml`, `directories`, `fs2`; `tempfile` for tests. No ML/audio deps in this plan.

**Spec:** `docs/superpowers/specs/2026-06-08-talk-cli-design.md`

---

## Plan decomposition note

This is **Plan 1 of 4** (see the roadmap at the bottom). It deliberately stops before audio and ML so the foundation is provable with fast deterministic tests. Definition of done for Plan 1: `talk reflect`/`journal`/`unburden`, fed a scripted transcript via `--from-text`, write byte-correct files (frontmatter + chronological dated sections + raw comments), honor `keep_raw`/permissions/ephemeral-no-write; **config is loaded and applied, selection + state are wired into the CLI** (bare `talk` selects from the spine pack by time-of-day/rotation/`held:7` and persists `last-served`/held progress), and the deterministic cleanup layer (spoken commands + backtrack) runs in the session — all under `cargo test`.

> **Ephemeral scope (honest):** Plan 1's ephemeral mode guarantees only the *happy-path file-skip* — it writes no file and the integration test asserts zero bytes land in the base dir. The spec §7 hardening (`zeroize` on drop, best-effort `mlock`, disabling the crash-recovery buffer) is **deferred to Plan 4** and is NOT provided here. Plan 1 must not ship as a user-facing feature without Plan 4.

## File structure

```
talk-cli/
  Cargo.toml                       # workspace root + the `talk` package
  crates/
    talk-core/
      Cargo.toml
      src/
        lib.rs                     # re-exports; the pure API surface
        palette.rs                 # RUST base tone (ported from meditate-core)
        slug.rs                    # id passthrough + BYO slug derivation + stable hash
        frontmatter.rs             # Frontmatter struct: to_yaml / parse (hand-rolled, dep-light)
        entry.rs                   # append a dated section to a file body (pure string op)
        settle.rs                  # Live → Committing → Settled state machine (pure)
        cleanup.rs                 # Level enum, deterministic layer, content-word diff-guard
        questions.rs               # Question/Pack data model + TOML load
        selection.rs               # pick a question: pack + time-of-day + rotation + cadence
        clock.rs                   # Clock trait (injectable) so selection/tests are deterministic
  src/
    main.rs                        # entrypoint; dispatch clap → command handlers
    cli.rs                         # clap command definitions
    config.rs                      # config.toml load/init (mirror meditate)
    paths.rs                       # base dir resolution + 0700/0600 creation
    state.rs                       # on-disk selection/streak state (serde_json)
    writer.rs                      # resolve target file + read-modify-write via entry.rs
    session.rs                     # orchestrates TranscriptSource → cleanup → settle → writer
    source.rs                      # TranscriptSource trait + FakeTranscript impl
  questions/
    spine.toml                     # placeholder spine (real 65 vendored in Plan 4)
  tests/
    integration.rs                 # end-to-end via FakeTranscript
```

---

## Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml` (workspace root + `talk` package)
- Create: `crates/talk-core/Cargo.toml`
- Create: `crates/talk-core/src/lib.rs`
- Create: `src/main.rs`

- [ ] **Step 1: Create the workspace + binary manifest**

Create `Cargo.toml`:

```toml
[workspace]
members = ["crates/talk-core"]

[package]
name = "talk"
version = "0.1.0"
edition = "2021"
rust-version = "1.82"
license = "MIT"
description = "A terminal listening companion — speak a reflection, and it settles into a quiet file."
repository = "https://github.com/walktalkmeditate/talk-cli"
keywords = ["journaling", "voice", "reflection", "cli", "wellness"]
categories = ["command-line-utilities"]
exclude = ["/crates", "/web", "/demo", "/docs", "/tests", "/.github"]

[[bin]]
name = "talk"
path = "src/main.rs"

[dependencies]
talk-core = { path = "crates/talk-core", version = "0.1.0" }
clap = { version = "=4.5.20", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
directories = "5"
fs2 = "0.4"

[target.'cfg(unix)'.dependencies]
libc = "0.2"

[dev-dependencies]
tempfile = "3"

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
strip = true
```

- [ ] **Step 2: Create the core crate manifest**

Create `crates/talk-core/Cargo.toml`:

```toml
[package]
name = "talk-core"
version = "0.1.0"
edition = "2021"
rust-version = "1.82"
license = "MIT"
description = "The pure talk-cli engine: selection, slugs, frontmatter, settle, cleanup."
repository = "https://github.com/walktalkmeditate/talk-cli"

[dependencies]
serde = { version = "1", features = ["derive"] }
toml = "0.8"
```

- [ ] **Step 3: Stub the core lib and binary so the workspace compiles**

Create `crates/talk-core/src/lib.rs`:

```rust
//! The pure talk-cli engine. No I/O, no audio, no ML.
```

Create `src/main.rs`:

```rust
fn main() {
    println!("talk: scaffold");
}
```

- [ ] **Step 4: Verify the workspace builds**

Run: `cargo build`
Expected: compiles clean; `talk: scaffold` runs via `cargo run`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/talk-core/Cargo.toml crates/talk-core/src/lib.rs src/main.rs
git commit -m "chore: scaffold talk workspace (talk + talk-core)"
```

---

## Task 2: Palette (RUST base tone)

**Files:**
- Create: `crates/talk-core/src/palette.rs`
- Modify: `crates/talk-core/src/lib.rs`

This ports meditate-core's palette synthesis but swaps the base tone to the talk pillar's `rust` (`rgb(160,99,75)`, from `pilgrim-ios/rust.colorset`). Keep the season/time-of-day shaping minimal here; Plan 2's renderer consumes it.

- [ ] **Step 1: Write the failing test**

Add to `crates/talk-core/src/palette.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb { pub r: u8, pub g: u8, pub b: u8 }

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self { Rgb { r, g, b } }
}

/// The talk pillar base tone — `rust`, from pilgrim-ios rust.colorset (light).
/// Plan 1 needs only the base constant; the `palette()` synthesis (edge/dim
/// variants + season/time tinting) is deferred to Plan 2, where the renderer
/// that consumes it is built (YAGNI — no Plan-1 consumer exists).
pub const RUST: Rgb = Rgb::new(160, 99, 75);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_is_the_talk_base_tone() {
        assert_eq!(RUST, Rgb::new(160, 99, 75));
    }
}
```

Add to `crates/talk-core/src/lib.rs`:

```rust
pub mod palette;
```

- [ ] **Step 2: Run the tests to verify they pass**

Run: `cargo test -p talk-core palette`
Expected: PASS (1 test).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/palette.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): rust-based palette (talk pillar tone)"
```

---

## Task 3: Slug + question identity

**Files:**
- Create: `crates/talk-core/src/slug.rs`
- Modify: `crates/talk-core/src/lib.rs`

Per spec §8: pack/spine questions carry an immutable `id` (the binding); BYO derives a deterministic kebab slug. Same text → same slug (so a repeated BYO question reuses its file); collisions get a stable short hash suffix.

- [ ] **Step 1: Write the failing test**

Create `crates/talk-core/src/slug.rs`:

```rust
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
    joined.chars().take(60).collect()
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
    fn short_hash_is_stable_and_short() {
        let a = short_hash("what am i avoiding");
        assert_eq!(a, short_hash("what am i avoiding"));
        assert_eq!(a.len(), 6);
        assert_ne!(a, short_hash("what am i grateful for"));
    }
}
```

Add to `lib.rs`:

```rust
pub mod slug;
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core slug`
Expected: PASS (5 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/slug.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): slug derivation + stable short hash"
```

---

## Task 4: Frontmatter (hand-rolled, dependency-light)

**Files:**
- Create: `crates/talk-core/src/frontmatter.rs`
- Modify: `crates/talk-core/src/lib.rs`

Per spec §8 frontmatter shape: `id` (immutable binding), `question`, `slug`, `pack`, `addressee`, `created`, `entries`, `last`. Hand-rolled to keep `talk-core` dependency-light; fields are controlled scalars, `question` is always quoted/escaped.

- [ ] **Step 1: Write the failing test**

Create `crates/talk-core/src/frontmatter.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frontmatter {
    pub id: String,
    pub question: String,
    pub slug: String,
    pub pack: String,
    pub addressee: String,
    pub created: String, // YYYY-MM-DD
    pub entries: u32,
    pub last: String,    // YYYY-MM-DD
}

impl Frontmatter {
    /// Render as a `---`-delimited YAML block (trailing newline included).
    pub fn to_yaml(&self) -> String {
        format!(
            "---\nid: {id}\nquestion: {q}\nslug: {slug}\npack: {pack}\naddressee: {addr}\ncreated: {created}\nentries: {entries}\nlast: {last}\n---\n",
            id = self.id,
            q = quote(&self.question),
            slug = self.slug,
            pack = self.pack,
            addr = self.addressee,
            created = self.created,
            entries = self.entries,
            last = self.last,
        )
    }

    /// Parse the leading `---`-delimited block. Returns (frontmatter, rest_of_body).
    pub fn parse(input: &str) -> Option<(Frontmatter, &str)> {
        let rest = input.strip_prefix("---\n")?;
        let end = rest.find("\n---\n")?;
        let block = &rest[..end];
        let body = &rest[end + 5..];

        let mut map = std::collections::HashMap::new();
        for line in block.lines() {
            if let Some((k, v)) = line.split_once(": ") {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        Some((
            Frontmatter {
                id: map.get("id")?.clone(),
                question: unquote(map.get("question")?),
                slug: map.get("slug")?.clone(),
                pack: map.get("pack")?.clone(),
                addressee: map.get("addressee")?.clone(),
                created: map.get("created")?.clone(),
                entries: map.get("entries")?.parse().ok()?,
                last: map.get("last")?.clone(),
            },
            body,
        ))
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(s: &str) -> String {
    let trimmed = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s);
    trimmed.replace("\\\"", "\"").replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frontmatter {
        Frontmatter {
            id: "avoidance-core".into(),
            question: "What am I avoiding?".into(),
            slug: "what-am-i-avoiding".into(),
            pack: "examen".into(),
            addressee: "self".into(),
            created: "2026-06-06".into(),
            entries: 3,
            last: "2026-06-08".into(),
        }
    }

    #[test]
    fn round_trips() {
        let fm = sample();
        let rendered = fm.to_yaml() + "\n## 2026-06-06\nbody text\n";
        let (parsed, body) = Frontmatter::parse(&rendered).unwrap();
        assert_eq!(parsed, fm);
        assert_eq!(body, "\n## 2026-06-06\nbody text\n");
    }

    #[test]
    fn quotes_questions_with_special_chars() {
        let mut fm = sample();
        fm.question = "Why \"this\": really?".into();
        let (parsed, _) = Frontmatter::parse(&(fm.to_yaml() + "\nx\n")).unwrap();
        assert_eq!(parsed.question, "Why \"this\": really?");
    }
}
```

Add to `lib.rs`:

```rust
pub mod frontmatter;
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core frontmatter`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/frontmatter.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): hand-rolled frontmatter round-trip"
```

---

## Task 5: Entry append (pure string op)

**Files:**
- Create: `crates/talk-core/src/entry.rs`
- Modify: `crates/talk-core/src/lib.rs`

Per spec §8: dated sections append chronologically; each section carries an optional raw HTML comment then the cleaned text. A second entry on the same date nests under a `### HH:MM` subsection. This is a pure function over the existing file body + the new entry — no I/O.

- [ ] **Step 1: Write the failing test**

Create `crates/talk-core/src/entry.rs`:

```rust
/// One recorded turn, already cleaned by `cleanup`.
pub struct Entry<'a> {
    pub date: &'a str, // YYYY-MM-DD
    pub time: &'a str, // HH:MM
    pub raw: Option<&'a str>, // None when keep_raw = false or ephemeral
    pub clean: &'a str,
}

/// Reflect files hold many dates in one question-file, so their sections are
/// dates (`## 2026-06-06`); journal files are one-per-day, so their sections are
/// times (`## 08:14`) per spec §8.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode { Reflect, Journal }

/// Append `entry` to `body` (the part after frontmatter), returning the new body.
/// - Reflect: a new date appends `## DATE`; a repeat date nests `### HH:MM`.
/// - Journal: every turn appends `## HH:MM` (the file is already one date).
pub fn append(body: &str, entry: &Entry, mode: Mode) -> String {
    let block = render_turn(entry);
    match mode {
        Mode::Journal => join(body, &format!("\n## {}\n{}", entry.time, block)),
        Mode::Reflect => {
            let date_header = format!("## {}", entry.date);
            if body.contains(&date_header) {
                join(body, &format!("\n### {}\n{}", entry.time, block))
            } else {
                join(body, &format!("\n{}\n{}", date_header, block))
            }
        }
    }
}

fn render_turn(entry: &Entry) -> String {
    match entry.raw {
        Some(raw) => format!("<!-- raw: {} -->\n{}\n", sanitize_comment(raw), entry.clean),
        None => format!("{}\n", entry.clean),
    }
}

/// Neutralize anything that could break out of the HTML comment: the `--`
/// digraph (which terminates/malforms comments) and `<` (a nested `<!--`).
fn sanitize_comment(raw: &str) -> String {
    raw.replace('<', "&lt;")
        .replace("--", "&#45;&#45;")
        .replace('\n', " ")
}

fn join(body: &str, section: &str) -> String {
    format!("{}\n{}", body.trim_end_matches('\n'), section.trim_start_matches('\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn<'a>(date: &'a str, time: &'a str, raw: Option<&'a str>, clean: &'a str) -> Entry<'a> {
        Entry { date, time, raw, clean }
    }

    #[test]
    fn reflect_new_date_appends_a_dated_section() {
        let out = append("", &turn("2026-06-06", "08:14", Some("um the thing is"), "The thing is."), Mode::Reflect);
        assert!(out.contains("## 2026-06-06"));
        assert!(out.contains("<!-- raw: um the thing is -->"));
        assert!(out.contains("The thing is."));
    }

    #[test]
    fn reflect_repeat_date_nests_a_timestamped_subsection() {
        let body = "\n## 2026-06-06\nFirst.\n";
        let out = append(body, &turn("2026-06-06", "20:15", None, "Second."), Mode::Reflect);
        assert!(out.contains("### 20:15"));
        assert_eq!(out.matches("## 2026-06-06").count(), 1);
    }

    #[test]
    fn journal_sections_are_time_keyed() {
        let out = append("", &turn("2026-06-08", "08:14", None, "Morning."), Mode::Journal);
        assert!(out.contains("## 08:14") && !out.contains("## 2026-06-08"));
        let out2 = append(&out, &turn("2026-06-08", "21:30", None, "Night."), Mode::Journal);
        assert!(out2.contains("## 08:14") && out2.contains("## 21:30"));
    }

    #[test]
    fn raw_none_omits_the_comment() {
        let out = append("", &turn("2026-06-06", "08:14", None, "Clean only."), Mode::Reflect);
        assert!(!out.contains("<!-- raw"));
    }

    #[test]
    fn comment_breakout_chars_are_neutralized() {
        let out = append("", &turn("2026-06-06", "08:14", Some("end --> <!-- x"), "y"), Mode::Reflect);
        assert_eq!(out.matches("-->").count(), 1); // only the comment's own terminator
        assert!(!out.contains("<!-- x"));
    }
}
```

Add to `lib.rs`:

```rust
pub mod entry;
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core entry`
Expected: PASS (5 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/entry.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): chronological dated-section append"
```

---

## Task 6: Cleanup — deterministic layer + content-word diff-guard

**Files:**
- Create: `crates/talk-core/src/cleanup.rs`
- Modify: `crates/talk-core/src/lib.rs`

Per spec §10. This plan ships the **deterministic** layer (spoken commands, backtrack, deterministic-Light transform) and the **diff-guard** (content-word preservation). The LLM rewrite is Plan 3 — but the guard predicate must exist now because the settle's instant layer (Plan 2) uses deterministic-Light, and Plan 3's LLM output will be gated by this exact predicate.

- [ ] **Step 1: Write the failing test for the diff-guard**

Create `crates/talk-core/src/cleanup.rs`:

```rust
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
}
```

Add to `lib.rs`:

```rust
pub mod cleanup;
```

- [ ] **Step 2: Run the guard tests**

Run: `cargo test -p talk-core cleanup`
Expected: PASS (4 tests).

- [ ] **Step 3: Write the failing test for the deterministic layer**

Append to `crates/talk-core/src/cleanup.rs` (above the `#[cfg(test)]` block):

```rust
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

/// Find `needle` in `hay` only at word boundaries (so the trigger "actually no"
/// does NOT match inside "actually nobody"). ASCII-boundary check; English
/// triggers only.
fn find_word_bounded(hay: &str, needle: &str) -> Option<usize> {
    let bytes = hay.as_bytes();
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let pos = from + rel;
        let before_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let after = pos + needle.len();
        let after_ok = after == bytes.len() || !bytes[after].is_ascii_alphanumeric();
        if before_ok && after_ok { return Some(pos); }
        from = pos + needle.len();
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
        while let Some(pos) = find_word_bounded(&result.to_lowercase(), trigger) {
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
```

Add these tests inside the existing `mod tests`:

```rust
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

    #[test]
    fn backtrack_does_not_fire_inside_a_word() {
        // "actually no" must NOT match inside "actually nobody" (word-bounded).
        let out = apply_backtrack("well actually nobody knows the truth");
        assert!(out.contains("nobody"));
        assert!(out.contains("the truth"));
    }

    #[test]
    fn spoken_command_at_phrase_start_and_end() {
        // Boundary commands are consumed (no stray "new"/"line" words survive);
        // a boundary newline is trimmed, an interior one is kept (see test above).
        assert_eq!(apply_spoken_commands("new line b"), "b");
        assert_eq!(apply_spoken_commands("a new line"), "a");
    }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p talk-core cleanup`
Expected: PASS (10 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/talk-core/src/cleanup.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): deterministic cleanup + content-word diff-guard"
```

---

## Task 7: Settle state machine (pure)

**Files:**
- Create: `crates/talk-core/src/settle.rs`
- Modify: `crates/talk-core/src/lib.rs`

Per spec §7. The state machine is pure: it consumes transcript events and emits the current render model. It has three phases — `live` (jittering partial), `committing` (the latest phrase, bright but still inside the decode-lag / async-swap window), and `settled` (immutable). Plan 2's crossterm painter renders this model; Plan 3's async LLM swap replaces the *committing* block via `upgrade_committing` (it can't touch settled blocks). Modeling `committing` now means Plans 2–3 extend this type rather than rewrite it. The invariant under test: a settled block is never mutated once finalized.

- [ ] **Step 1: Write the failing test**

Create `crates/talk-core/src/settle.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub clean: String,
    /// Raw is retained so `u` (raw⇄clean toggle) and recovery work.
    pub raw: String,
}

/// Three-phase model per spec §7, so Plan 2's renderer and Plan 3's async swap
/// EXTEND this type instead of rewriting it:
/// - `live`       — the jittering partial hypothesis (not committed)
/// - `committing` — the most recent phrase, shown bright with deterministic-Light,
///   still inside the decode-lag / async-swap window (may be upgraded or take a
///   late STT revision)
/// - `settled`    — finalized, immutable blocks (never move again)
#[derive(Default)]
pub struct Settle {
    settled: Vec<Block>,
    committing: Option<Block>,
    live: String,
}

impl Settle {
    pub fn new() -> Self { Self::default() }

    /// A new/revised partial hypothesis for the live edge.
    pub fn on_partial(&mut self, partial: &str) {
        self.live = partial.to_string();
    }

    /// VAD boundary: the live edge becomes the committing block (`clean` is the
    /// deterministic-Light result). Any prior committing block's window has
    /// closed, so it is finalized into `settled` first.
    pub fn commit(&mut self, raw: &str, clean: &str) {
        self.finalize();
        self.committing = Some(Block { clean: clean.to_string(), raw: raw.to_string() });
        self.live.clear();
    }

    /// Promote the committing block to settled (its lag/swap window elapsed).
    pub fn finalize(&mut self) {
        if let Some(b) = self.committing.take() {
            self.settled.push(b);
        }
    }

    /// Plan 3 async LLM swap: replace the committing block's clean text while
    /// still inside its window. No-op (returns false) once finalized — the
    /// settled rule wins.
    pub fn upgrade_committing(&mut self, clean: &str) -> bool {
        match self.committing.as_mut() {
            Some(b) => { b.clean = clean.to_string(); true }
            None => false,
        }
    }

    /// A late STT revision targeting already-settled text is DROPPED (returns
    /// false so the caller can log it to the raw layer).
    pub fn try_late_revision_settled(&mut self, _index: usize, _new_text: &str) -> bool {
        false
    }

    pub fn settled(&self) -> &[Block] { &self.settled }
    pub fn committing(&self) -> Option<&Block> { self.committing.as_ref() }
    pub fn live(&self) -> &str { &self.live }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_updates_only_the_live_edge() {
        let mut s = Settle::new();
        s.on_partial("um so the thing");
        assert_eq!(s.live(), "um so the thing");
        assert!(s.settled().is_empty() && s.committing().is_none());
    }

    #[test]
    fn commit_moves_live_into_committing_not_settled() {
        let mut s = Settle::new();
        s.on_partial("um so the thing is");
        s.commit("um so the thing is", "The thing is.");
        assert_eq!(s.committing().unwrap().clean, "The thing is.");
        assert_eq!(s.live(), "");
        assert!(s.settled().is_empty());
    }

    #[test]
    fn second_commit_finalizes_the_first() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        s.commit("b", "B.");
        assert_eq!(s.settled().len(), 1);
        assert_eq!(s.settled()[0].clean, "A.");
        assert_eq!(s.committing().unwrap().clean, "B.");
    }

    #[test]
    fn upgrade_changes_only_the_committing_block() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        s.commit("b raw", "B.");
        let settled0 = s.settled()[0].clone();
        assert!(s.upgrade_committing("B, refined."));
        assert_eq!(s.settled()[0], settled0); // settled is immutable
        assert_eq!(s.committing().unwrap().clean, "B, refined.");
    }

    #[test]
    fn upgrade_is_noop_after_finalize() {
        let mut s = Settle::new();
        s.commit("a", "A.");
        s.finalize();
        assert!(!s.upgrade_committing("A!"));
        assert_eq!(s.settled()[0].clean, "A.");
    }

    #[test]
    fn late_revision_never_mutates_settled() {
        let mut s = Settle::new();
        s.commit("first", "First.");
        s.finalize();
        let before = s.settled().to_vec();
        assert!(!s.try_late_revision_settled(0, "FIRST!!"));
        assert_eq!(s.settled(), &before[..]);
    }
}
```

Add to `lib.rs`:

```rust
pub mod settle;
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core settle`
Expected: PASS (6 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/settle.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): settle state machine (immutable settled blocks)"
```

---

## Task 8: Questions + Clock

**Files:**
- Create: `crates/talk-core/src/clock.rs`
- Create: `crates/talk-core/src/questions.rs`
- Modify: `crates/talk-core/src/lib.rs`

Per spec §8/§9. A `Clock` trait makes time-of-day selection deterministic in tests. `questions.rs` models packs and loads them from TOML.

- [ ] **Step 1: Write the Clock and its test**

Create `crates/talk-core/src/clock.rs`:

```rust
/// Injectable time source so selection is deterministic in tests.
pub trait Clock {
    fn hour(&self) -> u32; // 0..=23
}

pub struct FixedClock(pub u32);
impl Clock for FixedClock {
    fn hour(&self) -> u32 { self.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeOfDay { Morning, Day, Evening, Night }

pub fn time_of_day(hour: u32) -> TimeOfDay {
    match hour {
        5..=8 => TimeOfDay::Morning,
        9..=16 => TimeOfDay::Day,
        17..=20 => TimeOfDay::Evening,
        _ => TimeOfDay::Night,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_the_day() {
        assert_eq!(time_of_day(7), TimeOfDay::Morning);
        assert_eq!(time_of_day(23), TimeOfDay::Night);
    }
}
```

- [ ] **Step 2: Write the questions model + TOML load test**

Create `crates/talk-core/src/questions.rs`:

```rust
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
```

Add to `lib.rs`:

```rust
pub mod clock;
pub mod questions;
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p talk-core clock questions`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/talk-core/src/clock.rs crates/talk-core/src/questions.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): question/pack model + injectable clock"
```

---

## Task 9: Selection

**Files:**
- Create: `crates/talk-core/src/selection.rs`
- Modify: `crates/talk-core/src/lib.rs`

Per spec §8. Pure selection over a pack + caller-supplied state. Order: (1) a mid-`held:N` run keeps serving its question until the run completes; (2) else prefer questions whose `slot` matches time-of-day; (3) else rotate to the least-recently-served, breaking ties by lowest served-count then declaration order. Streak-gated depth is deferred to Plan 4 (per the spec's resolved decision), so it's out of scope here.

- [ ] **Step 1: Write the failing test**

Create `crates/talk-core/src/selection.rs`:

```rust
use crate::clock::{time_of_day, TimeOfDay};
use crate::questions::{Pack, Question};

/// Caller-supplied selection state (persisted on disk by the binary).
#[derive(Default)]
pub struct SelectionState {
    /// id -> times served
    pub served_count: std::collections::HashMap<String, u32>,
    /// id -> last-served ordinal (monotonic counter; higher = more recent)
    pub last_served: std::collections::HashMap<String, u64>,
    /// An in-progress held run: (question id, turns completed so far).
    pub held_run: Option<(String, u32)>,
}

pub fn select<'a>(pack: &'a Pack, state: &SelectionState, hour: u32) -> Option<&'a Question> {
    // 1. A held run in progress wins until complete.
    if let Some((id, done)) = &state.held_run {
        if let Some(q) = pack.questions.iter().find(|q| &q.id == id) {
            if let Some(len) = Pack::held_len(&q.cadence) {
                if *done < len {
                    return Some(q);
                }
            }
        }
    }

    let slot = match time_of_day(hour) {
        TimeOfDay::Morning => Some("morning"),
        TimeOfDay::Evening => Some("evening"),
        _ => None,
    };

    let candidates: Vec<&Question> = match slot {
        Some(s) if pack.questions.iter().any(|q| q.slot.as_deref() == Some(s)) => {
            pack.questions.iter().filter(|q| q.slot.as_deref() == Some(s)).collect()
        }
        _ => pack.questions.iter().collect(),
    };

    // 2/3. Least-recently-served, then lowest count, then declaration order.
    candidates.into_iter().enumerate().min_by(|(ia, a), (ib, b)| {
        let la = state.last_served.get(&a.id).copied().unwrap_or(0);
        let lb = state.last_served.get(&b.id).copied().unwrap_or(0);
        let ca = state.served_count.get(&a.id).copied().unwrap_or(0);
        let cb = state.served_count.get(&b.id).copied().unwrap_or(0);
        la.cmp(&lb).then(ca.cmp(&cb)).then(ia.cmp(ib))
    }).map(|(_, q)| q)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> Pack {
        Pack::from_toml(r#"
            name = "t"
            [[questions]]
            id = "a"
            text = "A?"
            slot = "morning"
            [[questions]]
            id = "b"
            text = "B?"
            slot = "evening"
            [[questions]]
            id = "h"
            text = "Held?"
            cadence = "held:7"
        "#).unwrap()
    }

    #[test]
    fn held_run_keeps_serving_until_complete() {
        let p = pack();
        let st = SelectionState { held_run: Some(("h".into(), 3)), ..Default::default() };
        assert_eq!(select(&p, &st, 10).unwrap().id, "h");
    }

    #[test]
    fn held_run_releases_when_complete() {
        let p = pack();
        let st = SelectionState { held_run: Some(("h".into(), 7)), ..Default::default() };
        assert_ne!(select(&p, &st, 7).unwrap().id, "h");
    }

    #[test]
    fn morning_prefers_morning_slot() {
        let p = pack();
        let st = SelectionState::default();
        assert_eq!(select(&p, &st, 7).unwrap().id, "a");
    }

    #[test]
    fn rotation_avoids_the_most_recent() {
        let p = pack();
        let mut st = SelectionState::default();
        st.last_served.insert("a".into(), 5);
        // At midday no slot filter; "a" is most recent so it should be skipped.
        assert_ne!(select(&p, &st, 12).unwrap().id, "a");
    }
}
```

Add to `lib.rs`:

```rust
pub mod selection;
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core selection`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/selection.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): question selection (held/slot/rotation)"
```

---

## Task 10: Paths + permissions

**Files:**
- Create: `src/paths.rs`
- Modify: `src/main.rs`

Per spec §8 (`0700`/`0600`) and §12 (configurable base dir). The base dir defaults to `~/talk`; created `0700`, files written `0600`.

- [ ] **Step 1: Write the failing test**

Create `src/paths.rs`:

```rust
use std::io;
use std::path::{Path, PathBuf};

/// Resolve the base dir: explicit override, else `~/talk`.
pub fn base_dir(override_path: Option<PathBuf>) -> PathBuf {
    override_path.unwrap_or_else(|| {
        directories::UserDirs::new()
            .map(|d| d.home_dir().join("talk"))
            .unwrap_or_else(|| PathBuf::from("talk"))
    })
}

/// Resolve a config-supplied base dir, validating it. A configured path must be
/// absolute and under the user's home — rejecting `../..` traversal or an
/// arbitrary/cloud-synced location, per spec §13. `None` → the default `~/talk`.
pub fn resolve_base(configured: Option<&str>) -> io::Result<PathBuf> {
    let Some(raw) = configured else { return Ok(base_dir(None)) };
    let p = PathBuf::from(expand_tilde(raw));
    if !p.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "base_dir must be absolute"));
    }
    if let Some(dirs) = directories::UserDirs::new() {
        if !p.starts_with(dirs.home_dir()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "base_dir must be under your home directory",
            ));
        }
    }
    Ok(p)
}

fn expand_tilde(s: &str) -> String {
    match s.strip_prefix("~/") {
        Some(rest) => directories::UserDirs::new()
            .map(|d| d.home_dir().join(rest).to_string_lossy().into_owned())
            .unwrap_or_else(|| s.to_string()),
        None => s.to_string(),
    }
}

/// Create the base dir if missing, with 0700 perms set AT creation (no window
/// where it exists at the umask default).
pub fn ensure_base_dir(dir: &Path) -> io::Result<()> {
    if dir.exists() {
        return set_perms(dir, 0o700);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

/// Write `contents` to `path`, creating the file with mode 0600 AT open time —
/// no world-readable TOCTOU window between create and a later chmod.
pub fn write_private(path: &Path, contents: &str) -> io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    let mut f = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true).create(true).truncate(true).mode(0o600).open(path)?
    };
    #[cfg(not(unix))]
    let mut f = std::fs::File::create(path)?;
    f.write_all(contents.as_bytes())
}

#[cfg(unix)]
fn set_perms(path: &Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_perms(_path: &Path, _mode: u32) -> io::Result<()> { Ok(()) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_dir_respects_override() {
        let p = base_dir(Some(PathBuf::from("/tmp/x")));
        assert_eq!(p, PathBuf::from("/tmp/x"));
    }

    #[test]
    fn resolve_base_rejects_relative_and_outside_home() {
        assert!(resolve_base(Some("../../etc")).is_err());      // not absolute
        assert!(resolve_base(Some("/etc")).is_err());           // outside home
        assert!(resolve_base(None).is_ok());                    // default ok
    }

    #[cfg(unix)]
    #[test]
    fn written_files_are_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        ensure_base_dir(dir.path()).unwrap();
        let f = dir.path().join("a.md");
        write_private(&f, "hi").unwrap();
        let mode = std::fs::metadata(&f).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
    }
}
```

Wire the module in `src/main.rs`:

```rust
mod paths;

fn main() {
    println!("talk: scaffold");
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test paths`
Expected: PASS (3 tests).

- [ ] **Step 3: Commit**

```bash
git add src/paths.rs src/main.rs
git commit -m "feat: base-dir resolution + 0700/0600 perms"
```

---

## Task 11: Writer (resolve target file + read-modify-write)

**Files:**
- Create: `src/writer.rs`
- Modify: `src/main.rs`

Combines `frontmatter` + `entry` into the on-disk operation. Reflect files are bound by `id` (filename uses the slug); journal files are date-keyed. Ephemeral skips the write entirely. Honors `keep_raw` and the optional `~/talk/.raw/` sidecar.

- [ ] **Step 1: Write the failing test**

Create `src/writer.rs`:

```rust
use std::path::{Path, PathBuf};
use talk_core::entry::{append, Entry, Mode};
use talk_core::frontmatter::Frontmatter;
use crate::paths::write_private;

pub enum Target<'a> {
    /// Reflect: one file per question, bound by id.
    Reflect { id: &'a str, question: &'a str, slug: &'a str, pack: &'a str, addressee: &'a str },
    /// Journal: date-keyed.
    Journal,
}

pub struct WriteRequest<'a> {
    pub base: &'a Path,
    pub target: Target<'a>,
    pub date: &'a str,
    pub time: &'a str,
    pub raw: Option<&'a str>,
    pub clean: &'a str,
    pub keep_raw: bool,
    pub ephemeral: bool,
}

/// Returns the written path, or None when ephemeral (nothing persisted).
pub fn write_entry(req: &WriteRequest) -> std::io::Result<Option<PathBuf>> {
    if req.ephemeral {
        return Ok(None);
    }
    let path = target_path(req.base, &req.target, req.date);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let raw = if req.keep_raw { req.raw } else { None };
    let entry = Entry { date: req.date, time: req.time, raw, clean: req.clean };

    let new_contents = match &req.target {
        Target::Reflect { id, question, slug, pack, addressee } => {
            let (mut fm, body) = match Frontmatter::parse(&existing) {
                Some((fm, body)) => (fm, body.to_string()),
                None => (
                    Frontmatter {
                        id: id.to_string(), question: question.to_string(),
                        slug: slug.to_string(), pack: pack.to_string(),
                        addressee: addressee.to_string(), created: req.date.to_string(),
                        entries: 0, last: req.date.to_string(),
                    },
                    String::new(),
                ),
            };
            fm.entries += 1;
            fm.last = req.date.to_string();
            let new_body = append(&body, &entry, Mode::Reflect);
            format!("{}{}", fm.to_yaml(), new_body)
        }
        Target::Journal => append(&existing, &entry, Mode::Journal),
    };

    write_private(&path, &new_contents)?;
    Ok(Some(path))
}

fn target_path(base: &Path, target: &Target, date: &str) -> PathBuf {
    match target {
        Target::Reflect { slug, .. } => base.join(format!("{}.md", slug)),
        Target::Journal => base.join(format!("{}.md", date)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let req = WriteRequest {
            base: dir.path(),
            target: Target::Journal,
            date: "2026-06-08", time: "08:14",
            raw: Some("secret"), clean: "Secret.",
            keep_raw: true, ephemeral: true,
        };
        assert!(write_entry(&req).unwrap().is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn reflect_creates_then_appends_same_file() {
        let dir = tempfile::tempdir().unwrap();
        // A generic fn (not a closure) so the returned WriteRequest's borrows
        // tie to the inputs — closures can't express late-bound lifetimes.
        fn mk<'a>(base: &'a Path, date: &'a str, clean: &'a str) -> WriteRequest<'a> {
            WriteRequest {
                base,
                target: Target::Reflect {
                    id: "avoidance-core", question: "What am I avoiding?",
                    slug: "what-am-i-avoiding", pack: "examen", addressee: "self",
                },
                date, time: "08:14", raw: Some("um"), clean, keep_raw: true, ephemeral: false,
            }
        }
        write_entry(&mk(dir.path(), "2026-06-06", "First.")).unwrap();
        let p = write_entry(&mk(dir.path(), "2026-06-07", "Second.")).unwrap().unwrap();

        let text = std::fs::read_to_string(&p).unwrap();
        let (fm, _) = Frontmatter::parse(&text).unwrap();
        assert_eq!(fm.entries, 2);
        assert_eq!(fm.id, "avoidance-core");
        assert!(text.contains("## 2026-06-06") && text.contains("## 2026-06-07"));
    }

    #[test]
    fn keep_raw_false_omits_comment() {
        let dir = tempfile::tempdir().unwrap();
        let req = WriteRequest {
            base: dir.path(), target: Target::Journal,
            date: "2026-06-08", time: "08:14",
            raw: Some("secret"), clean: "Clean.", keep_raw: false, ephemeral: false,
        };
        let p = write_entry(&req).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("<!-- raw"));
    }
}
```

Wire into `src/main.rs`:

```rust
mod paths;
mod writer;

fn main() {
    println!("talk: scaffold");
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test writer`
Expected: PASS (3 tests).

- [ ] **Step 3: Commit**

```bash
git add src/writer.rs src/main.rs
git commit -m "feat: file writer (reflect id-binding, journal, ephemeral, keep_raw)"
```

---

## Task 12: TranscriptSource + FakeTranscript

**Files:**
- Create: `src/source.rs`
- Modify: `src/main.rs`

The seam that lets Plan 1 be end-to-end testable without a mic. Plan 2's sherpa-onnx façade implements the same trait.

- [ ] **Step 1: Write the failing test**

Create `src/source.rs`:

```rust
/// One emitted transcript event from a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A revised partial hypothesis for the live edge.
    Partial(String),
    /// A phrase boundary: the final raw text of the committed phrase.
    Commit(String),
    /// The user finished the whole turn.
    Done,
}

/// A source of transcript events. Plan 2 implements this over Moonshine+VAD.
pub trait TranscriptSource {
    fn next(&mut self) -> Option<Event>;
}

/// A scripted source for tests and `--from-text`.
pub struct FakeTranscript {
    events: std::collections::VecDeque<Event>,
}

impl FakeTranscript {
    pub fn new(events: Vec<Event>) -> Self {
        Self { events: events.into() }
    }

    /// Build a one-commit source from a plain string (used by `talk --from-text`).
    pub fn from_text(text: &str) -> Self {
        Self::new(vec![Event::Partial(text.into()), Event::Commit(text.into()), Event::Done])
    }
}

impl TranscriptSource for FakeTranscript {
    fn next(&mut self) -> Option<Event> {
        self.events.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_yields_scripted_events_in_order() {
        let mut s = FakeTranscript::from_text("hello world");
        assert_eq!(s.next(), Some(Event::Partial("hello world".into())));
        assert_eq!(s.next(), Some(Event::Commit("hello world".into())));
        assert_eq!(s.next(), Some(Event::Done));
        assert_eq!(s.next(), None);
    }
}
```

Wire into `src/main.rs`:

```rust
mod paths;
mod source;
mod writer;

fn main() {
    println!("talk: scaffold");
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test source`
Expected: PASS (1 test).

- [ ] **Step 3: Commit**

```bash
git add src/source.rs src/main.rs
git commit -m "feat: TranscriptSource trait + FakeTranscript"
```

---

## Task 13: Session orchestrator

**Files:**
- Create: `src/session.rs`
- Modify: `src/main.rs`

Drives a `TranscriptSource` through `settle` + `cleanup`, accumulating committed turns, then writes one entry whose `clean` is the joined deterministic-Light text and `raw` is the joined verbatim. (Plan 2 adds live rendering; Plan 3 swaps in the LLM.) Returns the written path (None if ephemeral).

- [ ] **Step 1: Write the failing test**

Create `src/session.rs`:

```rust
use crate::source::{Event, TranscriptSource};
use crate::writer::{write_entry, Target, WriteRequest};
use std::path::{Path, PathBuf};
use talk_core::cleanup::{apply_backtrack, apply_spoken_commands, deterministic_light};
use talk_core::settle::Settle;

pub struct RunConfig<'a> {
    pub base: &'a Path,
    pub date: &'a str,
    pub time: &'a str,
    pub keep_raw: bool,
    pub ephemeral: bool,
}

/// Consume the whole source, running the deterministic cleanup layer
/// (spoken-commands → backtrack → Light) on each committed phrase, settling it,
/// then persisting. `settle` is the single source of truth for the output (raw
/// verbatim + Light clean), so the file matches what Plan 2 will render.
///
/// Note: the Light layer normalizes whitespace, so a spoken `new line` removes
/// the command words but does not yet preserve structural newlines — structured
/// formatting is a Plan 3 (High cleanup / LLM) concern. Plan 1 only guarantees
/// the literal command words don't survive.
pub fn run(
    source: &mut dyn TranscriptSource,
    target: Target,
    cfg: &RunConfig,
) -> std::io::Result<Option<PathBuf>> {
    let mut settle = Settle::new();

    while let Some(ev) = source.next() {
        match ev {
            Event::Partial(p) => settle.on_partial(&p),
            Event::Commit(raw) => {
                let pre = apply_backtrack(&apply_spoken_commands(&raw));
                let clean = deterministic_light(&pre);
                settle.commit(&raw, &clean); // raw stored verbatim for recovery
            }
            Event::Done => break,
        }
    }
    settle.finalize(); // promote the last committing block

    let raw_joined = settle.settled().iter().map(|b| b.raw.as_str()).collect::<Vec<_>>().join(" ");
    let clean_joined = settle.settled().iter().map(|b| b.clean.as_str()).collect::<Vec<_>>().join(" ");

    write_entry(&WriteRequest {
        base: cfg.base,
        target,
        date: cfg.date,
        time: cfg.time,
        raw: Some(&raw_joined),
        clean: &clean_joined,
        keep_raw: cfg.keep_raw,
        ephemeral: cfg.ephemeral,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::FakeTranscript;

    fn cfg(base: &Path, ephemeral: bool) -> RunConfig<'_> {
        RunConfig { base, date: "2026-06-08", time: "08:14", keep_raw: true, ephemeral }
    }

    #[test]
    fn run_settles_and_writes_clean_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Partial("um so the thing".into()),
            Event::Commit("um so the thing is i keep avoiding it".into()),
            Event::Done,
        ]);
        let p = run(&mut src, Target::Journal, &cfg(dir.path(), false)).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("The thing is i keep avoiding it."));
        assert!(text.contains("<!-- raw: um so the thing is i keep avoiding it -->"));
    }

    #[test]
    fn multiple_commits_accumulate_into_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Commit("the first thing".into()),
            Event::Commit("the second thing".into()),
            Event::Done,
        ]);
        let p = run(&mut src, Target::Journal, &cfg(dir.path(), false)).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("The first thing.") && text.contains("The second thing."));
    }

    #[test]
    fn spoken_command_words_do_not_survive() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::new(vec![
            Event::Commit("first point new line second point".into()),
            Event::Done,
        ]);
        let p = run(&mut src, Target::Journal, &RunConfig {
            base: dir.path(), date: "2026-06-08", time: "08:14", keep_raw: false, ephemeral: false,
        }).unwrap().unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("new line"));
    }

    #[test]
    fn ephemeral_run_persists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut src = FakeTranscript::from_text("the secret is");
        let out = run(&mut src, Target::Journal, &cfg(dir.path(), true)).unwrap();
        assert!(out.is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
```

Wire into `src/main.rs`:

```rust
mod paths;
mod session;
mod source;
mod writer;

fn main() {
    println!("talk: scaffold");
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test session`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**

```bash
git add src/session.rs src/main.rs
git commit -m "feat: session orchestrator (source → settle → cleanup → write)"
```

---

## Task 14: Config + state (persistence)

**Files:**
- Create: `src/config.rs`
- Create: `src/state.rs`
- Modify: `src/main.rs`

Per spec §12. `config.toml` mirrors meditate's pattern (commented init, zero-config launch). `state.json` persists selection state (and later streak).

- [ ] **Step 1: Write the config test**

Create `src/config.rs`:

```rust
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
```

- [ ] **Step 2: Write the state test**

Create `src/state.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub served_count: HashMap<String, u32>,
    pub last_served: HashMap<String, u64>,
    pub tick: u64,
    pub held_run: Option<(String, u32)>,
    pub streak: u32,
    pub last_session_date: Option<String>,
}

impl State {
    pub fn load(text: &str) -> State {
        serde_json::from_str(text).unwrap_or_default()
    }
    /// Serialize for persistence. Callers MUST write the result with
    /// `paths::write_private` (0600) — this file records which private questions
    /// you've engaged with and how often.
    pub fn save(&self) -> String {
        serde_json::to_string_pretty(self).unwrap()
    }

    /// Record that `id` was served now (advances the monotonic tick).
    pub fn record_served(&mut self, id: &str) {
        self.tick += 1;
        self.last_served.insert(id.to_string(), self.tick);
        *self.served_count.entry(id.to_string()).or_insert(0) += 1;
    }

    /// Build the pure selection state talk-core's `select()` consumes.
    pub fn selection_state(&self) -> talk_core::selection::SelectionState {
        talk_core::selection::SelectionState {
            served_count: self.served_count.clone(),
            last_served: self.last_served.clone(),
            held_run: self.held_run.clone(),
        }
    }

    /// After serving `q`, advance held-run bookkeeping for a `held:N` cadence so
    /// the same question keeps being chosen until the run completes, then releases.
    pub fn advance_held(&mut self, q: &talk_core::questions::Question) {
        if let Some(len) = talk_core::questions::Pack::held_len(&q.cadence) {
            let done = match &self.held_run {
                Some((id, n)) if id == &q.id => n + 1,
                _ => 1,
            };
            self.held_run = if done >= len { None } else { Some((q.id.clone(), done)) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_records() {
        let mut s = State::default();
        s.record_served("a");
        s.record_served("a");
        let reloaded = State::load(&s.save());
        assert_eq!(reloaded.served_count.get("a"), Some(&2));
        assert_eq!(reloaded.tick, 2);
    }
}
```

Wire into `src/main.rs`:

```rust
mod config;
mod paths;
mod session;
mod source;
mod state;
mod writer;

fn main() {
    println!("talk: scaffold");
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test config state`
Expected: PASS (4 tests).

- [ ] **Step 4: Commit**

```bash
git add src/config.rs src/state.rs src/main.rs
git commit -m "feat: config.toml + selection/streak state persistence"
```

---

## Task 15: CLI wiring + `--from-text` end-to-end

**Files:**
- Create: `src/cli.rs`
- Modify: `src/main.rs`
- Create: `questions/spine.toml`
- Create: `tests/integration.rs`

Wires everything together: loads `Config`, resolves/validates the base dir, and for bare `talk`/`reflect` loads the compiled-in spine pack, calls `selection::select()`, uses the chosen question's **authored `id`**, and persists `State` (`.state.json`, `0600`). `--from-text` feeds `FakeTranscript` so the whole pipeline runs without a mic — the temporary driver Plan 2 replaces with the real audio source. Bare `talk` = reflect (spine selection); `talk "..."` = BYO.

- [ ] **Step 1: Create a minimal spine for selection to load**

Create `questions/spine.toml`:

```toml
name = "spine"
description = "Placeholder spine — the real 65 are vendored in Plan 4."

[[questions]]
id = "carrying-not-yours"
text = "What are you carrying that isn't yours?"
slot = "evening"

[[questions]]
id = "morning-intention"
text = "What do you want to protect today?"
slot = "morning"
```

- [ ] **Step 2: Write the CLI**

Create `src/cli.rs`:

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "talk", version, about = "The terminal that listens.")]
pub struct Cli {
    /// A bring-your-own question (bare `talk "..."`).
    pub question: Option<String>,

    /// Drive the pipeline from text instead of a mic (Plan-1 testing seam).
    /// `global = true` so it parses AFTER a subcommand (e.g. `talk unburden --from-text ...`).
    #[arg(long, global = true)]
    pub from_text: Option<String>,

    /// Today's date override (tests pass this; real runs use the system date).
    #[arg(long, hide = true, global = true)]
    pub date: Option<String>,

    /// Time override (HH:MM).
    #[arg(long, hide = true, global = true)]
    pub time: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Reflect — asks a question, then listens (default).
    Reflect,
    /// Freeform daily journal — no prompt.
    Journal,
    /// Ephemeral — listens, shows, keeps nothing.
    Unburden,
    /// Alias for unburden.
    Vent,
    /// Print a question's accumulated file (no arg → list).
    Thread { question: Option<String> },
    /// Show your reflection streak.
    Streak,
    /// Config helpers.
    Config { action: Option<String> },
}
```

- [ ] **Step 3: Wire `main.rs` to dispatch**

Replace `src/main.rs` with:

```rust
mod cli;
mod config;
mod paths;
mod session;
mod source;
mod state;
mod writer;

use clap::Parser;
use cli::{Cli, Command};
use session::{run, RunConfig};
use source::FakeTranscript;
use std::path::{Path, PathBuf};
use writer::Target;

/// The spine pack is compiled in, so Plan 1 has no runtime file dependency.
const SPINE_TOML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/spine.toml"));

fn main() -> std::io::Result<()> {
    let args = Cli::parse();

    // Config first (it may relocate the base dir); then resolve + validate base.
    let cfg = load_config()?;
    let base = paths::resolve_base(cfg.base_dir.as_deref())?;
    paths::ensure_base_dir(&base)?;

    let date = args.date.clone().unwrap_or_else(system_date);
    let time = args.time.clone().unwrap_or_else(system_time_hm);

    match args.command {
        Some(Command::Journal) => {
            let text = require_text(&args.from_text);
            run_and_report(&base, Target::Journal, &date, &time, &text, cfg.keep_raw, false)?;
        }
        Some(Command::Unburden) | Some(Command::Vent) => {
            let text = require_text(&args.from_text);
            run_and_report(&base, Target::Journal, &date, &time, &text, cfg.keep_raw, true)?;
            println!("Released. Nothing was written.");
        }
        Some(Command::Config { action }) => return handle_config(&base, action.as_deref()),
        Some(Command::Thread { question }) => print_thread(&base, question.as_deref()),
        Some(Command::Streak) => println!("streak: (Plan 4)"),
        // Bare `talk`, `talk reflect`, or `talk "byo question"` → reflect.
        _ => reflect(&base, &args.question, &date, &time, &require_text(&args.from_text), &cfg)?,
    }
    Ok(())
}

/// Reflect: a BYO question if one was given, else select from the spine pack.
fn reflect(base: &Path, byo: &Option<String>, date: &str, time: &str, text: &str, cfg: &config::Config) -> std::io::Result<()> {
    let mut st = state::State::load(&std::fs::read_to_string(state_path(base)).unwrap_or_default());

    let (id, question, slug, pack, addressee) = match byo {
        Some(q) => {
            // BYO: id == slug, collision-suffixed against a DIFFERENT existing question.
            let slug = talk_core::slug::derive_slug_unique(q, |s| {
                file_question_differs(&base.join(format!("{}.md", s)), q)
            });
            (slug.clone(), q.clone(), slug, "byo".to_string(), "self".to_string())
        }
        None => {
            let spine = talk_core::questions::Pack::from_toml(SPINE_TOML)
                .expect("bundled spine.toml is valid");
            let chosen = talk_core::selection::select(&spine, &st.selection_state(), hour_of(time))
                .expect("spine is non-empty")
                .clone();
            // Filename slug: authored if present, else the id itself (authored
            // ids are already kebab / filename-safe; re-deriving would mangle them).
            let slug = chosen.slug.clone().unwrap_or_else(|| chosen.id.clone());
            st.record_served(&chosen.id);
            st.advance_held(&chosen);
            (chosen.id, chosen.text, slug, spine.name, chosen.addressee)
        }
    };

    let target = Target::Reflect { id: &id, question: &question, slug: &slug, pack: &pack, addressee: &addressee };
    run_and_report(base, target, date, time, text, cfg.keep_raw, false)?;
    paths::write_private(&state_path(base), &st.save())?;
    Ok(())
}

fn run_and_report(base: &Path, target: Target, date: &str, time: &str, text: &str, keep_raw: bool, ephemeral: bool) -> std::io::Result<()> {
    let path = run(&mut FakeTranscript::from_text(text), target,
        &RunConfig { base, date, time, keep_raw, ephemeral })?;
    if let Some(p) = path {
        println!("→ {}", p.display());
    }
    Ok(())
}

fn handle_config(base: &Path, action: Option<&str>) -> std::io::Result<()> {
    match action {
        Some("init") => {
            let p = base.join("config.toml");
            paths::write_private(&p, &config::Config::commented_template())?;
            println!("wrote {}", p.display());
        }
        Some("path") => println!("{}", base.join("config.toml").display()),
        _ => print!("{}", config::Config::commented_template()),
    }
    Ok(())
}

fn print_thread(base: &Path, question: Option<&str>) {
    match question {
        Some(q) => {
            let p: PathBuf = base.join(format!("{}.md", talk_core::slug::derive_slug(q)));
            match std::fs::read_to_string(&p) {
                Ok(text) => print!("{}", text),
                Err(_) => println!("No thread yet for \"{}\".", q),
            }
        }
        None => println!("(thread list — Plan 4)"),
    }
}

fn load_config() -> std::io::Result<config::Config> {
    let text = std::fs::read_to_string(paths::base_dir(None).join("config.toml")).unwrap_or_default();
    Ok(config::Config::load(&text).unwrap_or_default())
}

fn state_path(base: &Path) -> PathBuf {
    base.join(".state.json") // dot-prefixed so vault sync / indexing skip it
}

/// True only if a file exists AND stores a DIFFERENT question (a real collision).
fn file_question_differs(path: &Path, q: &str) -> bool {
    std::fs::read_to_string(path).ok()
        .and_then(|t| talk_core::frontmatter::Frontmatter::parse(&t).map(|(fm, _)| fm.question != q))
        .unwrap_or(false)
}

fn require_text(from: &Option<String>) -> String {
    match from {
        Some(t) if !t.trim().is_empty() => t.clone(),
        _ => {
            eprintln!("Plan 1: pass --from-text <text> to drive the pipeline (the real audio source lands in Plan 2).");
            std::process::exit(2);
        }
    }
}

fn hour_of(hm: &str) -> u32 {
    hm.split(':').next().and_then(|h| h.parse().ok()).unwrap_or(12)
}

// Plan 1 wires a real (UTC) system clock so files aren't epoch-dated; tests pass
// --date/--time for determinism. Plan 2 can add local-timezone handling.
fn system_date() -> String {
    let (y, m, d) = civil_from_days((unix_secs() / 86_400) as i64);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn system_time_hm() -> String {
    let s = unix_secs() % 86_400;
    format!("{:02}:{:02}", s / 3600, (s % 3600) / 60)
}

fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Howard Hinnant's civil-from-days (UTC), dependency-free.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as u32, d)
}
```

- [ ] **Step 4: Write the end-to-end integration test**

Create `tests/integration.rs`:

```rust
use std::path::Path;
use std::process::{Command, Output};

// `directories` resolves the base dir from $HOME on unix, so overriding HOME
// steers the binary's writes under the tempdir.
fn talk(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_talk"))
        .args(args).env("HOME", home).output().unwrap()
}

#[test]
fn byo_reflect_writes_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &[
        "What am I avoiding?", "--from-text", "um keep avoiding it",
        "--date", "2026-06-08", "--time", "08:14",
    ]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let text = std::fs::read_to_string(dir.path().join("talk/what-am-i-avoiding.md")).unwrap();
    assert!(text.contains("id: what-am-i-avoiding"));
    assert!(text.contains("pack: byo"));
    assert!(text.contains("## 2026-06-08"));
    assert!(text.contains("Keep avoiding it.")); // leading "um" stripped by Light
}

#[test]
fn bare_reflect_selects_from_spine_by_time_of_day() {
    let dir = tempfile::tempdir().unwrap();
    // 07:30 → morning slot → the spine's morning question, keyed by its AUTHORED id.
    let out = talk(dir.path(), &[
        "--from-text", "something came up", "--date", "2026-06-08", "--time", "07:30",
    ]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));

    let text = std::fs::read_to_string(dir.path().join("talk/morning-intention.md")).unwrap();
    assert!(text.contains("id: morning-intention"));
    assert!(text.contains("pack: spine"));
}

#[test]
fn unburden_keeps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &[
        "unburden", "--from-text", "the secret is", "--date", "2026-06-08", "--time", "08:14",
    ]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Released. Nothing was written."));

    let talk_dir = dir.path().join("talk");
    assert_eq!(std::fs::read_dir(&talk_dir).unwrap().count(), 0, "ephemeral left files behind");
}

#[test]
fn missing_from_text_errors_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &["journal", "--date", "2026-06-08", "--time", "08:14"]);
    assert!(!out.status.success()); // exits non-zero, writes nothing
    assert_eq!(std::fs::read_dir(dir.path().join("talk")).unwrap().count(), 0);
}
```

> Note: `paths::base_dir(None)` resolves `~/talk` via `directories`, which reads `HOME` on unix — so the `HOME` override lands files under the tempdir. (On a machine where `directories` consults a non-HOME source, set `TALK_BASE_DIR` instead — a hook Plan 2 can add to `paths::base_dir`.)

- [ ] **Step 5: Run the full suite**

Run: `cargo test`
Expected: PASS — all unit tests + the four integration tests green.

- [ ] **Step 6: Manual smoke check**

Run: `cargo run -- "what am I avoiding?" --from-text "um i keep putting it off" --date 2026-06-08 --time 09:00`
Expected: prints `→ .../talk/what-am-i-avoiding.md`; the file contains the cleaned text + raw comment.

- [ ] **Step 7: Commit**

```bash
git add src/cli.rs src/main.rs questions/spine.toml tests/integration.rs
git commit -m "feat: CLI wiring + --from-text end-to-end (reflect/journal/unburden)"
```

---

## Self-Review (completed during authoring; refreshed after the 2026-06-08 plan review)

- **Spec coverage (Plan-1 scope):** `RUST` const §2 (T2, `palette()` synthesis deferred to P2) · slug/id + collision-suffix §8 (T3) · frontmatter §8 (T4) · chronological append + journal-time-keyed `Mode` §8 (T5) · deterministic cleanup + content-word diff-guard §10 (T6) · three-phase settle (`live`/`committing`/`settled`) §7 (T7) · selection held/slot/rotation §8 (T8–T9) · `0700/0600` at-create + base-dir validation §8/§13 (T10) · reflect/journal/ephemeral/keep_raw write §7/§8 (T11) · source seam (T12) · orchestration with the deterministic layer wired §10 (T13) · config/state + held advance §12 (T14) · CLI verbs §6, **selection + config + state wired**, real UTC clock, bare-talk-is-reflect (T15). **Deferred by design:** live render + audio + `palette()` (P2), LLM rewrite/async swap/eval set (P3), real spine + flagship packs + download + integrity + streak + sidecar raw store + no-egress/tamper tests (P4). The latency spike (spec §5/§7) gates Plan 2; model-integrity verification must land in Plan 2 before any model loads.
- **Placeholder scan:** no TBD/TODO; every code step has complete code. The only intentional stubs are `streak` (display = P4) and the no-arg `thread` list (= P4) — each labeled. The clock is real (UTC); ephemeral's Plan-1 scope (happy-path file-skip only; §7 hardening = P4) is stated honestly up front.
- **Type consistency:** `append(body, &Entry, Mode)` is called with `Mode::Reflect`/`Mode::Journal` from the writer (T11); `Settle` exposes `commit`/`finalize`/`upgrade_committing`/`try_late_revision_settled` used consistently in T7/T13; `State::selection_state()`/`advance_held()` feed `selection::select()` in T15; `Question.slug` is `Option<String>`; `derive_slug` / `derive_slug_unique` are used identically across T3/T15; spine ids are used verbatim as filenames (never re-derived). All cross-task signatures line up.

---

## Post-review hardening (2026-06-08)

After implementation, `ce-code-review` (correctness · security · adversarial · reliability · testing · maintainability) ran on the branch and found issues the code blocks above predate. All were fixed in commit `24af606` (suite 60 green, clippy clean); a round-2 verification review then confirmed the fixes held and caught one regression in the `thread` fix (wrong file returned on a slug collision) + a `--date` path-traversal, both resolved in `d03817d` (suite 62 green). The module code in this plan is the pre-hardening version, so treat the committed code as authoritative where they differ. Fixes:

- **P0:** `apply_backtrack` sliced the original string with offsets from a lowercased copy → mid-codepoint panic on case-shrinking Unicode. `find_word_bounded` now searches case-insensitively over the original (byte offsets always valid).
- **P1:** spine load/select no longer `.expect()`-panic (clean errors); `write_private` is temp-file + atomic `rename` (no disk-full clobber); `write_entry` refuses to overwrite a non-empty *unparseable* reflect file; `config.toml` is fixed at the default `~/talk` and `default_mode` is wired; `advance_held` got tests.
- **P2/P3:** frontmatter `quote` folds newlines (no YAML breakout); empty-derivable slugs fall back to `short_hash`; a BYO slug colliding with a journal date file now suffixes instead of clobbering; `talk thread <q>` resolves spine/suffixed files via frontmatter scan; `resolve_base` rejects `..`; Plan-2/3 seams (`Level`, `Clock`, `streak`) carry comments.

Deferred (residual): full file-locking for the concurrent same-file write race (atomic-rename prevents corruption; lost-update is acceptable for a single-user journal) and wiring `default_pack` / `auto_end_silence_seconds` (no consumer until Plan 2). The content-word `guard_accepts` must be wired into the session path before Plan 3's LLM rewrite.

## Roadmap (Plans 2–4)

**Plan 2 — Listen + Render.** *Prerequisite spike (spec §5/§7): measure 0.5B Q4 Light-cleanup latency on M1 + a commodity x86 CPU before locking the engine; if it blows the ~250ms swap window, the async-swap fallback in §7 is what ships.* Then: `cpal` mic capture (input stream + macOS permission + sample-rate conversion); `listen/` sherpa-onnx façade (Moonshine streaming partials + Silero VAD) implementing `TranscriptSource`; restore `palette()` synthesis (edge/dim/season-time, deferred from Plan 1) for the renderer; `render/` crossterm painter for the settle model (live edge dim/jitter → committing → settled bright, the immutable-block invariant), status line + `● local · no network`, in-session keys (`space`/`u`/`p`/`esc`), the close screen, and the ephemeral screen. **Model integrity:** wire the SHA-256 verify gate (Plan 4's pinned hashes) *before* any model-load path merges — never load an unverified model, even in Plan 2. Add a `TALK_BASE_DIR` env override to `paths::base_dir`. DoD: real voice → live settle → correct file.

**Plan 3 — Formatter + restraint.** `format/` Candle 0.5B façade behind a cargo `formatter` feature; spoken-command + backtrack pre-layer wired into the live path; async LLM rewrite gated by `cleanup::guard_accepts`, swapped via `settle::upgrade_block` within the window; the checked-in **eval set** (fixtures + a must-fail over-editing mock) in CI. DoD: opt-in pretty cleanup that never over-edits, deterministic-Light fallback intact.

**Plan 4 — Packs, download, privacy & polish.** Vendor the real 65-spine (YAML→TOML + `id`/`addressee`/`cadence`) and the four flagship packs; `talk download` for models + packs with HTTPS + **pinned SHA-256** + `verify`; ephemeral hardening (`zeroize`/`mlock`, crash-recovery disabled); the **opt-in `~/talk/.raw/` sidecar raw store** (spec §8 — orphaned until now); streak display + `thread` list view; `talk config path`; the **write-error recovery flow** (retry / clipboard / timeout-zeroize) and the **BYO near-match "continue this thread?" prompt** (spec §8); the **sandboxed no-egress test**, **model-tamper test**, and **ephemeral zero-bytes-to-disk test**; first-run model-fetch flow + cloud-sync disclosure. DoD: all spec §17 acceptance criteria green.

---

## Execution Handoff

(See the prompt that follows for the two execution options.)
