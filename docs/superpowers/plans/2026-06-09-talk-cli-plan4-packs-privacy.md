# talk-cli Packs, Privacy & Polish Implementation Plan (Plan 4 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish v1 — vendor the real 65-question spine + the four flagship packs, port the streak, complete the privacy story (sidecar raw store, ephemeral zeroize, disclosure, tamper/no-egress tests), and close every spec §17 acceptance criterion.

**Architecture:** Almost everything here is pure data + pure logic on seams that already exist: packs are TOML loaded by `talk_core::questions::Pack` (multi-pack registry + `default_pack` wiring in the binary); streak is a near-verbatim port of meditate-cli's `streak.rs` (file-locked read-modify-write via the already-present `fs2`); the sidecar raw store and held-label are writer/View wiring; hardening is `zeroize` on the ephemeral transcript plus integration tests that *prove* the privacy claims (tamper → refuse to run; sandboxed session → zero egress; ephemeral → zero bytes). Interactive-only UX (write-error recovery, BYO near-match, first-run fetch offer) lives in the live path and never touches the `--from-text` test seam.

**Tech Stack:** Rust 1.82 (no MSRV change); one new dep: `zeroize` (tiny, pure). No new network surface. GitHub Actions CI (authored here, verified on push).

**Origin spec:** `docs/superpowers/specs/2026-06-08-talk-cli-design.md` (§7 interaction states, §8 data model + sidecar + BYO near-match, §9 packs, §11 `download verify`, §12 streak, §13 error recovery, §14 privacy tests, §17 acceptance). **Roadmap source:** `docs/superpowers/plans/2026-06-08-talk-cli-foundation.md` (Plan 4 entry).

---

## Scope notes (read first)

- **Spine source exists:** the 65 curated questions live in `../walktalkmeditate/src/content/questions/{morning,evening,solo,walking}.yaml` (10+15+15+25 = 65, verified). T1 converts them to TOML; no content is invented.
- **Remote pack download is deferred post-launch** (spec §9 frames extra packs as the OSS contribution surface). There are zero post-launch packs in existence today, so building remote pack-fetch now would ship dead machinery (YAGNI). `talk download` with no arg lists what's installed (vendored packs + models status); `talk download models` and the new `talk download verify` work as before. The `Artifact`+`fetch` machinery from Plan 2 is the named path when a real pack exists to host.
- **`talk config path` already exists** (Plan 1) — dropped from this plan's scope.
- **mlock is deliberately NOT implemented.** Spec §7 says "best-effort mlock"; in safe Rust, `String`/`Vec` reallocation moves buffers, so mlocking the final address is security theater that complicates the code without pinning earlier copies (STT buffers, channel messages). The honest hardening is: ephemeral writes nothing (tested), the final transcript is `zeroize`d (T7), and the threat-model boundary (swap during hibernation, terminal scrollback) is stated in the disclosure (T9) exactly as spec §7 requires. This is surfaced for the doc-review to weigh.
- **Already-merged context this plan builds on:** `reflect_choice` (selection + state, shared by text/live paths), `run_live_session`/`live::run_loop`, `download::{fetch, verify, models::MODELS}`, `paths::{write_private, models_dir}`, `State`, `Config::cleanup_for`, the formatter moat (untouched here).
- **Ephemeral never earns streak or state** — T4/T7 must keep `talk unburden` writing zero bytes (the existing `unburden_keeps_nothing` test is extended into the zero-bytes test).

## File structure

```
talk-cli/
  questions/
    spine.toml                  # REPLACED: the real 65 (from walktalkmeditate YAML)
    future-self.toml            # NEW flagship (Address)
    parts.toml                  # NEW flagship (Inner-dialogue, IFS)
    examen.toml                 # NEW flagship (evening review)
    held.toml                   # NEW flagship (held:7 thread-builder)
  crates/talk-core/src/
    matchq.rs                   # NEW: question near-match similarity (pure)
  src/
    packs.rs                    # NEW: vendored-pack registry (include_str! + lookup)
    streak.rs                   # NEW: ported from meditate-cli (entry-day credit)
    main.rs                     # thread list, streak cmd, download list/verify, disclosure,
                                #   held label, near-match, first-run fetch offer
    writer.rs                   # sidecar raw store routing
    config.rs                   # + raw_sidecar
    live.rs                     # write-error recovery loop; ephemeral zeroize
  tests/
    integration.rs              # + thread list, streak, held-across-days, sidecar
    privacy.rs                  # NEW: zero-bytes, tamper (listen), no-egress (macOS sandbox-exec)
  .github/workflows/ci.yml     # NEW: test+clippy (macOS+Ubuntu), listen build, Linux no-egress
```

---

## Task 1: Vendor the real 65-question spine

**Files:**
- Replace: `questions/spine.toml`
- Test: `crates/talk-core/src/questions.rs` (add spine-shape tests in the binary's integration? No — add `tests/integration.rs` data test in the binary, which `include_str!`s the same file)

The conversion is mechanical from the four YAML files in `../walktalkmeditate/src/content/questions/`:

| Source file | Count | `slot` | Notes |
|---|---|---|---|
| `morning.yaml` | 10 | `"morning"` | morning seeds |
| `evening.yaml` | 15 | `"evening"` | depth questions |
| `solo.yaml` | 15 | *(none)* | any time |
| `walking.yaml` | 25 | *(none)* | any time |

**Mapping rules (authored once at vendor time, then immutable):**
- `text` = the YAML `text` verbatim (do not edit wording).
- `id` = a kebab-case slug of 3–5 distinctive content words from the text (e.g. "What are you grateful for in this moment?" → `grateful-this-moment`). Ids must be unique across the whole file; on collision, add one more distinguishing word. **Ids are the immutable thread identity (spec §8) — once this file merges, never change them.**
- `addressee` omitted (defaults `self`); `cadence` omitted (defaults `daily`); the YAML `stage` field is dropped (it's the website's grouping, not talk's).
- File header: `name = "spine"`, `description = "The 65-question spine — curated, human-authored (from walktalkmeditate)."`

Worked examples (first entry of each file):

```toml
[[questions]]
id = "grateful-this-moment"
text = "What are you grateful for in this moment?"
slot = "morning"

[[questions]]
id = "define-wisdom-changed"
text = "How do you define wisdom — and has your definition changed over time?"
slot = "evening"
```

> **Replaces the two placeholder questions** (`carrying-not-yours`, `morning-intention`). Existing thread files keyed to those ids stay valid on disk (`talk thread` still reads them); they simply stop being served. This is the id-immutability model working as designed — files never orphan, packs evolve.

- [ ] **Step 1: Write the failing test**

Add to `tests/integration.rs`:

```rust
#[test]
fn spine_is_the_real_sixty_five() {
    let spine = talk_core::questions::Pack::from_toml(include_str!("../questions/spine.toml")).unwrap();
    assert_eq!(spine.questions.len(), 65);
    let mut ids: Vec<&str> = spine.questions.iter().map(|q| q.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 65, "ids must be unique");
    assert!(ids.iter().all(|id| id.chars().all(|c| c.is_ascii_lowercase() || c == '-')));
    assert_eq!(spine.questions.iter().filter(|q| q.slot.as_deref() == Some("morning")).count(), 10);
    assert_eq!(spine.questions.iter().filter(|q| q.slot.as_deref() == Some("evening")).count(), 15);
}
```

(Note: `talk-core` must be a dev-dependency of the binary's integration tests — it already is a dependency, so `talk_core::` resolves.)

- [ ] **Step 2: Run it to fail** — `cargo test spine_is_the_real_sixty_five` → FAIL (placeholder has 2).

- [ ] **Step 3: Convert the YAMLs** — read each of the four files at `/Users/rubberduck/GitHub/momentmaker/walktalkmeditate/src/content/questions/`, apply the mapping rules, write the full 65-entry `questions/spine.toml`. Existing binary tests that referenced the placeholder spine (`bare_reflect_selects_from_spine_by_time_of_day` expects `morning-intention.md`) must be updated to the new morning-slot selection result: with a fresh state at 07:30, selection picks the first-declared morning question → `grateful-this-moment.md` with `id: grateful-this-moment`.

- [ ] **Step 4: Run the full suite** — `cargo test` → all green (the updated integration expectation included).

- [ ] **Step 5: Commit** — `git add questions/spine.toml tests/integration.rs && git commit -m "feat(packs): vendor the real 65-question spine"`

---

## Task 2: Flagship packs + multi-pack registry + `default_pack` wiring

**Files:**
- Create: `questions/future-self.toml`, `questions/parts.toml`, `questions/examen.toml`, `questions/held.toml`
- Create: `src/packs.rs`
- Modify: `src/main.rs` (use the registry + `cfg.default_pack`; `talk download` no-arg lists)

The four flagship packs (spec §9: one per direction; vent/unburden is a mode, not a pack). **This content is authored here — contemplative, plain, second person; ids immutable from merge:**

`questions/future-self.toml`:
```toml
name = "future-self"
description = "Talk to the you that's coming."

[[questions]]
id = "tell-december-you"
text = "Tell the you of next December what you're most scared of right now."
addressee = "future-self"

[[questions]]
id = "hope-still-true"
text = "What do you hope is still true about your life a year from now?"
addressee = "future-self"

[[questions]]
id = "promise-to-hold"
text = "What promise do you want future-you to hold you to?"
addressee = "future-self"

[[questions]]
id = "what-they-forgave"
text = "What will the you of ten years from now have forgiven you for?"
addressee = "future-self"

[[questions]]
id = "ask-the-you-beyond"
text = "What would you ask the you who has already been through this?"
addressee = "future-self"

[[questions]]
id = "small-thing-noticed"
text = "Tell future-you about a small thing from today they'd be glad you noticed."
addressee = "future-self"
```

`questions/parts.toml`:
```toml
name = "parts"
description = "Talk to a part of you."

[[questions]]
id = "part-speaking-loudest"
text = "Which part of you is speaking loudest today? Let it talk."
addressee = "the-part"

[[questions]]
id = "what-the-worrier-protects"
text = "What is the worried part of you trying to protect?"
addressee = "the-part"

[[questions]]
id = "how-old-is-the-hurt"
text = "How old is the part that's hurting, and what does it need to hear?"
addressee = "the-part"

[[questions]]
id = "thank-the-protector"
text = "Thank the part that's been working hardest lately. What has it been carrying?"
addressee = "the-part"

[[questions]]
id = "part-not-heard"
text = "Which part of you hasn't been allowed to speak in a long time?"
addressee = "the-part"

[[questions]]
id = "calmest-part-answers"
text = "If the calmest part of you could answer, what would it say to the rest?"
addressee = "the-part"
```

`questions/examen.toml`:
```toml
name = "examen"
description = "An evening look back over the day."

[[questions]]
id = "most-alive-today"
text = "Where did you feel most alive today?"
slot = "evening"

[[questions]]
id = "what-drained-today"
text = "What drained you today — and did it have to?"
slot = "evening"

[[questions]]
id = "received-and-given"
text = "What did you receive today? What did you give?"
slot = "evening"

[[questions]]
id = "tomorrow-differently"
text = "Knowing what today taught you, what will you do differently tomorrow?"
slot = "evening"

[[questions]]
id = "gratitude-almost-missed"
text = "What almost went unnoticed today that deserves gratitude?"
slot = "evening"
```

`questions/held.toml`:
```toml
name = "held"
description = "One question, held for seven days. The artifact is the change across the week."

[[questions]]
id = "held-what-matters"
text = "What matters most right now?"
cadence = "held:7"

[[questions]]
id = "held-avoiding"
text = "What are you avoiding?"
cadence = "held:7"

[[questions]]
id = "held-becoming"
text = "Who are you becoming?"
cadence = "held:7"
```

- [ ] **Step 1: Write the failing test**

Create `src/packs.rs`:

```rust
use talk_core::questions::Pack;

const SPINE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/spine.toml"));
const FUTURE_SELF: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/future-self.toml"));
const PARTS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/parts.toml"));
const EXAMEN: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/examen.toml"));
const HELD: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/questions/held.toml"));

/// Every pack compiled into the binary, in display order.
pub fn vendored() -> Vec<Pack> {
    [SPINE, FUTURE_SELF, PARTS, EXAMEN, HELD]
        .iter()
        .map(|s| Pack::from_toml(s).expect("vendored pack TOML is valid"))
        .collect()
}

/// The pack to serve from. Unknown names fall back to the spine (never an error —
/// a stale config must not block a reflection).
pub fn by_name(name: &str) -> Pack {
    vendored()
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| Pack::from_toml(SPINE).expect("spine TOML is valid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_vendored_packs_load_with_unique_ids() {
        let packs = vendored();
        assert_eq!(packs.len(), 5);
        let mut ids: Vec<String> = packs.iter().flat_map(|p| p.questions.iter().map(|q| q.id.clone())).collect();
        let total = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total, "question ids must be unique ACROSS packs (they share one file namespace)");
    }

    #[test]
    fn by_name_finds_flagships_and_falls_back_to_spine() {
        assert_eq!(by_name("parts").name, "parts");
        assert_eq!(by_name("does-not-exist").name, "spine");
    }

    #[test]
    fn held_pack_is_all_held_cadence() {
        assert!(by_name("held").questions.iter().all(|q| q.cadence == "held:7"));
    }
}
```

Add `mod packs;` to `src/main.rs`.

- [ ] **Step 2: Run to fail** — `cargo test packs` → FAIL (toml files missing). Create the four TOML files exactly as authored above. Run again → PASS (3 tests).

- [ ] **Step 3: Wire `default_pack` + the download list**

In `src/main.rs`:
- `reflect_choice`'s spine-loading line (`Pack::from_toml(SPINE_TOML)`) becomes `let pack = crate::packs::by_name(&cfg.default_pack);` — which requires threading `cfg` (or just `&cfg.default_pack`) into `reflect_choice(base, byo, time, default_pack: &str)`. Update both callers (`reflect`, `run_live_session`). Delete the now-unused `SPINE_TOML` const from main.rs (packs.rs owns the data).
- The `handle_download(None)` arm (currently treated as `models`): change `talk download` **with no target** to LIST instead of fetch:

```rust
        None => {
            println!("installed packs:");
            for p in packs::vendored() {
                println!("  {:<12} {:>2} questions — {}", p.name, p.questions.len(), p.description);
            }
            #[cfg(feature = "download")]
            {
                println!("\nmodels ({}):", paths::models_dir().display());
                for art in download::models::MODELS {
                    let ok = download::verify(&paths::models_dir().join(art.name), art.sha256).unwrap_or(false);
                    println!("  {} {}", if ok { "✓" } else { "✗" }, art.name);
                }
                println!("\nfetch models with `talk download models` · re-check with `talk download verify`");
            }
            println!("\nmore packs arrive post-launch via `talk download <pack>`.");
            Ok(())
        }
        Some("models") => { /* existing fetch loop unchanged */ }
```

(The `#[cfg(not(feature = "download"))]` stub keeps its current behavior for explicit targets but should also print the pack list for no-arg — structure the function so the pack list is unconditional and only the models section is gated.)

- [ ] **Step 4: Integration test** — add to `tests/integration.rs`:

```rust
#[test]
fn default_pack_config_serves_from_that_pack() {
    let dir = tempfile::tempdir().unwrap();
    let talk_dir = dir.path().join("talk");
    std::fs::create_dir_all(&talk_dir).unwrap();
    std::fs::write(talk_dir.join("config.toml"), "default_pack = \"examen\"\n").unwrap();
    let out = talk(dir.path(), &["--from-text", "today held more than i noticed", "--date", "2026-06-09", "--time", "20:30"]);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    // 20:30 → evening slot → examen's first evening question by declaration order.
    let text = std::fs::read_to_string(talk_dir.join("most-alive-today.md")).unwrap();
    assert!(text.contains("pack: examen"));
}
```

Run `cargo test` → all green.

- [ ] **Step 5: Commit** — `git add questions/ src/packs.rs src/main.rs tests/integration.rs && git commit -m "feat(packs): four flagship packs + registry + default_pack wiring"`

---

## Task 3: `talk thread` list view

**Files:**
- Modify: `src/main.rs` (`print_thread` `None` arm)
- Test: `tests/integration.rs`

Spec §7: no-arg prints a static list sorted by recency, `slug · entries · last`; empty state: `No threads yet — run \`talk\` to start one.` Journal files (no frontmatter) are excluded.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn thread_lists_questions_by_recency() {
    let dir = tempfile::tempdir().unwrap();
    talk(dir.path(), &["Old question?", "--from-text", "first words", "--date", "2026-06-01", "--time", "08:00"]);
    talk(dir.path(), &["New question?", "--from-text", "second words", "--date", "2026-06-09", "--time", "08:00"]);
    let out = talk(dir.path(), &["thread"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let new_pos = stdout.find("new-question").unwrap();
    let old_pos = stdout.find("old-question").unwrap();
    assert!(new_pos < old_pos, "most recent first:\n{stdout}");
    assert!(stdout.contains("1 entr"));

    let empty = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(empty.path().join("talk")).unwrap();
    let out = talk(empty.path(), &["thread"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("No threads yet"));
}
```

- [ ] **Step 2: Implement** — replace `print_thread`'s `None` arm:

```rust
        None => {
            let mut rows: Vec<(String, u32, String)> = std::fs::read_dir(base)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .filter_map(|p| {
                    let text = std::fs::read_to_string(&p).ok()?;
                    let (fm, _) = talk_core::frontmatter::Frontmatter::parse(&text)?;
                    Some((fm.slug, fm.entries, fm.last))
                })
                .collect();
            if rows.is_empty() {
                println!("No threads yet — run `talk` to start one.");
                return;
            }
            rows.sort_by(|a, b| b.2.cmp(&a.2)); // last-date desc (ISO dates sort lexically)
            for (slug, entries, last) in rows {
                println!("{slug} · {entries} {} · {last}", if entries == 1 { "entry" } else { "entries" });
            }
        }
```

(Check `Frontmatter` field names against `crates/talk-core/src/frontmatter.rs` — `slug`/`entries`/`last` exist per the Plan-1 schema; `entries` is numeric.)

- [ ] **Step 3: Run** — `cargo test thread` → PASS (this + the existing collision test). **Step 4: Commit** — `git commit -am "feat: thread list view (recency-sorted, empty state)"`

---

## Task 4: Streak (ported from meditate-cli, entry-day credit)

**Files:**
- Create: `src/streak.rs`
- Modify: `src/main.rs` (`talk streak` + credit after successful writes)
- Test: unit in `src/streak.rs` + `tests/integration.rs`

Port `../meditate-cli/src/streak.rs` with two adaptations: (1) **credit = a saved entry** (writing is the practice — no minimum-seconds rule; ephemeral never credits because it never saves); (2) the file is **`.streak.toml`** (dot-prefixed, like `.state.json`, so vault sync/indexing skip it) and is written `0600`. Keep meditate's `fs2` exclusive-lock read-modify-write and its backward-clock tolerance verbatim.

- [ ] **Step 1: Failing unit tests + implementation**

Create `src/streak.rs`:

```rust
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const STREAK_FILE: &str = ".streak.toml";

/// Local, account-free reflection record. A day is credited when an entry is
/// saved that civil day (ported from meditate-cli; talk credits entries, not
/// session length — writing is the practice).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Streak {
    pub entries: u64,
    pub current_streak: u32,
    pub longest_streak: u32,
    pub last_day: Option<i64>,
}

impl Streak {
    /// Fold one saved entry into the record. `today` is the civil day number
    /// (days since the Unix epoch).
    pub fn record(&mut self, today: i64) {
        self.entries += 1;
        match self.last_day {
            Some(day) if day == today => {}
            Some(day) if today == day + 1 => self.current_streak += 1,
            // Clock moved backward (NTP correction, travel) — leave the streak intact.
            Some(day) if today < day => {}
            _ => self.current_streak = 1,
        }
        self.last_day = Some(today);
        self.longest_streak = self.longest_streak.max(self.current_streak);
    }

    pub fn path_in(dir: &Path) -> PathBuf {
        dir.join(STREAK_FILE)
    }

    /// Missing or corrupt files are no history — a bad file never blocks a launch.
    pub fn load_from(dir: &Path) -> Streak {
        std::fs::read_to_string(Self::path_in(dir))
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }
}

/// Civil day number for an ISO `YYYY-MM-DD` date (Hinnant's days_from_civil).
pub fn civil_day(date: &str) -> Option<i64> {
    let mut parts = date.splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Record one saved entry under an exclusive file lock (read-modify-write, not
/// last-writer-wins), creating the file 0600.
pub fn record_entry(dir: &Path, today: i64) -> std::io::Result<Streak> {
    std::fs::create_dir_all(dir)?;
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(Streak::path_in(dir))?;
    file.lock_exclusive()?;

    let result = (|| {
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        let mut streak: Streak = toml::from_str(&text).unwrap_or_default();
        streak.record(today);
        let serialized = toml::to_string_pretty(&streak).expect("streak serializes to TOML");
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(serialized.as_bytes())?;
        Ok(streak)
    })();

    let _ = file.unlock();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_days_grow_the_streak() {
        let mut s = Streak::default();
        s.record(100);
        s.record(100); // same day: counted as an entry, not a new streak day
        s.record(101);
        assert_eq!(s.entries, 3);
        assert_eq!(s.current_streak, 2);
        s.record(105); // gap resets
        assert_eq!(s.current_streak, 1);
        assert_eq!(s.longest_streak, 2);
        s.record(103); // clock moved backward: streak intact
        assert_eq!(s.current_streak, 1);
    }

    #[test]
    fn civil_day_matches_known_dates() {
        assert_eq!(civil_day("1970-01-01"), Some(0));
        assert_eq!(civil_day("1970-01-02"), Some(1));
        assert_eq!(civil_day("2026-06-09"), Some(20_613));
        assert_eq!(civil_day("not-a-date"), None);
    }

    #[test]
    fn record_entry_locks_and_persists_0600() {
        let dir = tempfile::tempdir().unwrap();
        record_entry(dir.path(), 50).unwrap();
        let s = Streak::load_from(dir.path());
        assert_eq!(s.current_streak, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(Streak::path_in(dir.path())).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
```

(Verify `civil_day("2026-06-09")` == the value `main.rs`'s `civil_from_days` inverts to — compute once during implementation and pin the real number; 20_613 is the expected value, confirm with a quick `civil_from_days(20613)` check.)

- [ ] **Step 2: Wire it** — in `src/main.rs`: add `mod streak;`. `Some(Command::Streak)` becomes:

```rust
        Some(Command::Streak) => {
            let s = streak::Streak::load_from(&base);
            if s.entries == 0 {
                println!("No reflections yet — run `talk` to start.");
            } else {
                println!("current {} day{} · longest {} · {} entries",
                    s.current_streak, if s.current_streak == 1 { "" } else { "s" },
                    s.longest_streak, s.entries);
            }
            return Ok(());
        }
```

Credit after every **successful non-ephemeral write**, in exactly two places: (a) `run_and_report` — after `run(...)` returns `Some(path)`, `if let Some(day) = streak::civil_day(r.date) { let _ = streak::record_entry(r.base, day); }`; (b) `run_live_session` — after `write_entry` returns `Some(path)`, same call with the session `date`. (Streak failures never block the save — `let _ =`.)

- [ ] **Step 3: Integration test**

```rust
#[test]
fn streak_credits_consecutive_days() {
    let dir = tempfile::tempdir().unwrap();
    talk(dir.path(), &["journal", "--from-text", "day one", "--date", "2026-06-08", "--time", "08:00"]);
    talk(dir.path(), &["journal", "--from-text", "day two", "--date", "2026-06-09", "--time", "08:00"]);
    let out = talk(dir.path(), &["streak"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("current 2 days"), "{stdout}");
}

#[test]
fn ephemeral_never_credits_streak() {
    let dir = tempfile::tempdir().unwrap();
    talk(dir.path(), &["unburden", "--from-text", "let it go", "--date", "2026-06-09", "--time", "08:00"]);
    let out = talk(dir.path(), &["streak"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("No reflections yet"));
}
```

- [ ] **Step 4: Run all + commit** — `cargo test` green → `git add src/streak.rs src/main.rs tests/integration.rs && git commit -m "feat: reflection streak (entry-day credit, ported from meditate)"`

---

## Task 5: Held-run label (live view + close provenance)

**Files:**
- Modify: `src/main.rs` (`ReflectChoice` gains `held_day`; both paths use it)

Spec §7: the question box header reads `held 3 days` during a `held:N` run, and the close line reads `entry N · held D days`. Day number = the serving's position in the run: before `advance_held`, `state.held_run` for this id holds `done` (servings completed), so **this serving is day `done + 1`**; a fresh run starts at day 1.

- [ ] **Step 1: Failing unit test** — `reflect_choice` is private to main.rs; test via a small pure helper. Add to `main.rs`:

```rust
/// The 1-based day of a held run for `id`, given the PRE-advance state.
fn held_day_for(held_run: &Option<(String, u32)>, id: &str, cadence: &str) -> Option<u32> {
    talk_core::questions::Pack::held_len(cadence)?;
    match held_run {
        Some((run_id, done)) if run_id == id => Some(done + 1),
        _ => Some(1),
    }
}
```

with `#[cfg(test)]` tests in main.rs (the binary has a unit-test module precedent? If not, put `held_day_for` in `src/state.rs` where tests exist):

```rust
    #[test]
    fn held_day_is_one_based_and_only_for_held_cadence() {
        assert_eq!(held_day_for(&None, "h", "held:7"), Some(1));
        assert_eq!(held_day_for(&Some(("h".into(), 2)), "h", "held:7"), Some(3));
        assert_eq!(held_day_for(&Some(("other".into(), 2)), "h", "held:7"), Some(1));
        assert_eq!(held_day_for(&None, "d", "daily"), None);
    }
```

- [ ] **Step 2: Wire** — `ReflectChoice` gains `held_day: Option<u32>` (computed inside `reflect_choice` BEFORE `record_served`/`advance_held`, only for the spine-selection arm; BYO → `None`). In `run_live_session`: `held_label: choice.as_ref().and_then(|c| c.held_day).map(|d| format!("held {} day{}", d, if d == 1 { "" } else { "s" }))` — note `LiveConfig.held_label` is `Option<&str>`, so format into a `let` binding that outlives the config. Close provenance: `format!("entry {}{}", n, choice.held_day.map(|d| format!(" · held {d} days")).unwrap_or_default())`.

- [ ] **Step 3: Run + commit** — `cargo test` green → `git commit -am "feat: held-run label in live view + close provenance"`

---

## Task 6: Sidecar raw store (`~/talk/.raw/`)

**Files:**
- Modify: `src/config.rs` (+ `raw_sidecar: bool`, default false, documented in template)
- Modify: `src/writer.rs` (route raw)
- Test: `src/writer.rs` unit + `tests/integration.rs`

Spec §8: opt-in — the main file omits the inline raw comment; the verbatim raw appends to `~/talk/.raw/<same-filename>` (dir `0700`, file `0600`, dot-prefixed so vault sync and Obsidian indexing skip it). `keep_raw = false` still means *no raw anywhere*.

- [ ] **Step 1: Failing test** (in `src/writer.rs` tests):

```rust
    #[test]
    fn sidecar_routes_raw_out_of_the_main_file() {
        let dir = tempfile::tempdir().unwrap();
        let req = WriteRequest {
            base: dir.path(), target: Target::Journal,
            date: "2026-06-09", time: "08:14",
            raw: Some("um the verbatim words"), clean: "The verbatim words.",
            keep_raw: true, raw_sidecar: true, ephemeral: false,
        };
        let p = write_entry(&req).unwrap().unwrap();
        let main_text = std::fs::read_to_string(&p).unwrap();
        assert!(!main_text.contains("<!-- raw"));
        let side = dir.path().join(".raw").join(p.file_name().unwrap());
        let side_text = std::fs::read_to_string(&side).unwrap();
        assert!(side_text.contains("## 2026-06-09"));
        assert!(side_text.contains("um the verbatim words"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dmode = std::fs::metadata(dir.path().join(".raw")).unwrap().permissions().mode();
            assert_eq!(dmode & 0o777, 0o700);
        }
    }
```

- [ ] **Step 2: Implement** — `WriteRequest` gains `pub raw_sidecar: bool`. In `write_entry`, before building contents:

```rust
    let raw = if req.keep_raw { req.raw } else { None };
    let (inline_raw, sidecar_raw) = if req.raw_sidecar { (None, raw) } else { (raw, None) };
```

Use `inline_raw` for the `Entry`; after the successful `write_private` of the main file:

```rust
    if let Some(r) = sidecar_raw {
        let raw_dir = req.base.join(".raw");
        std::fs::create_dir_all(&raw_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&raw_dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let side = raw_dir.join(path.file_name().expect("entry paths have file names"));
        let existing = std::fs::read_to_string(&side).unwrap_or_default();
        let appended = format!("{existing}## {} {}\n{r}\n\n", req.date, req.time);
        crate::paths::write_private(&side, &appended)?;
    }
```

All existing `WriteRequest` literals (writer tests, session.rs) gain `raw_sidecar: false`. `session::RunConfig` gains `raw_sidecar: bool` threaded from `cfg.raw_sidecar` at both call paths (`Report` struct + `run_live_session`'s `WriteRequest`). Config template documents it: `raw_sidecar = false  # true: verbatim raw goes to ~/talk/.raw/ (skipped by vault sync) instead of inline comments`.

- [ ] **Step 3: Run + commit** — `cargo test` green → `git commit -am "feat: opt-in sidecar raw store (.raw/, 0700/0600)"`

---

## Task 7: Ephemeral zeroize + the zero-bytes test

**Files:**
- Modify: `Cargo.toml` (+ `zeroize = "1"`)
- Modify: `src/live.rs` (zeroize the ephemeral transcript)
- Create: `tests/privacy.rs`

Spec §7 hardening, honestly scoped (see Scope notes on mlock): the ephemeral transcript's final buffers are zeroized; ephemeral writes **zero bytes** under the base dir (no entry, no state, no streak, no sidecar) — proven by test. Crash-recovery doesn't exist yet (the write-error recovery in T8 is in-memory only and skips ephemeral), so "recovery disabled in ephemeral" is enforced by construction in T8.

- [ ] **Step 1: Failing test** — create `tests/privacy.rs`:

```rust
use std::path::Path;
use std::process::{Command, Output};

fn talk(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_talk"))
        .args(args).env("HOME", home).output().unwrap()
}

/// Spec §7: after a full ephemeral session, zero bytes of transcript touch the
/// base dir — no entry, no raw sidecar, no state, no streak.
#[test]
fn ephemeral_leaves_zero_bytes_in_the_base_dir() {
    let dir = tempfile::tempdir().unwrap();
    let out = talk(dir.path(), &[
        "unburden", "--from-text", "the secret that must not persist",
        "--date", "2026-06-09", "--time", "08:14",
    ]);
    assert!(out.status.success());
    let entries: Vec<_> = std::fs::read_dir(dir.path().join("talk")).unwrap().flatten().collect();
    assert!(entries.is_empty(), "ephemeral persisted: {entries:?}");
}
```

- [ ] **Step 2: Zeroize** — add `zeroize = "1"` to `[dependencies]`. In `src/live.rs`, after the ephemeral branch finishes (`show_released` and the ephemeral cancel return):

```rust
use zeroize::Zeroize;
// in run_live_session's caller path (main.rs), for ephemeral results:
result.raw.zeroize();
result.clean.zeroize();
```

(`LiveResult` fields are `String`s — zeroize them in `run_live_session` right after `show_released()?` and in the cancelled-ephemeral arm. This wipes the final joined transcript; intermediate STT buffers are out of safe-Rust reach and the threat boundary is stated in T9's disclosure.)

- [ ] **Step 3: Run + commit** — `cargo test --test privacy` PASS (the test passes already if Plan 1–3 behavior holds — it should; the value is the *pinned guarantee*) → `git add Cargo.toml Cargo.lock src/live.rs src/main.rs tests/privacy.rs && git commit -m "feat: ephemeral zeroize + zero-bytes-to-disk guarantee test"`

---

## Task 8: Write-error recovery (live path)

**Files:**
- Modify: `src/main.rs` (`run_live_session` write step)
- Modify: `src/live.rs` (a small prompt helper)

Spec §13: on write failure the session must not lose words. Live path: an inline prompt `write failed: <err> — [r]etry · [c]opy to clipboard · [d]iscard`; after 3 failed retries, nudge toward clipboard. Clipboard is best-effort `pbcopy` (macOS) / `xclip -selection clipboard` (else), feeding stdin. The close phrase only shows after a successful write (already true structurally). Ephemeral never enters this path (it never writes). The `--from-text` path keeps its plain error (it's the test seam, not session UX).

- [ ] **Step 1: Implement the helper** in `src/live.rs`:

```rust
/// Inline write-failure prompt. Returns the chosen action.
pub enum Recover { Retry, Clipboard, Discard }

pub fn ask_recover(err: &str, attempts: u32) -> std::io::Result<Recover> {
    let hint = if attempts >= 3 { " (3 failures — clipboard recommended)" } else { "" };
    paint_plain(&[
        format!("  write failed: {err}{hint}"),
        "  [r]etry · [c]opy to clipboard · [d]iscard".to_string(),
    ])?;
    loop {
        if let CtEvent::Key(k) = event::read()? {
            match k.code {
                KeyCode::Char('r') => return Ok(Recover::Retry),
                KeyCode::Char('c') => return Ok(Recover::Clipboard),
                KeyCode::Char('d') => return Ok(Recover::Discard),
                _ => {}
            }
        }
    }
}

/// Best-effort system clipboard (pbcopy / xclip). Errors surface to the caller.
pub fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("pbcopy")
    } else {
        let mut c = Command::new("xclip");
        c.args(["-selection", "clipboard"]);
        c
    };
    let mut child = cmd.stdin(Stdio::piped()).spawn()?;
    child.stdin.take().expect("piped stdin").write_all(text.as_bytes())?;
    let status = child.wait()?;
    if status.success() { Ok(()) } else {
        Err(std::io::Error::other("clipboard helper exited non-zero"))
    }
}
```

- [ ] **Step 2: Wire the loop** in `run_live_session`, replacing the bare `write_entry(...)?`:

```rust
    let mut attempts = 0u32;
    let written = loop {
        match writer::write_entry(&writer::WriteRequest { /* as before */ }) {
            Ok(w) => break w,
            Err(e) => {
                attempts += 1;
                match live::ask_recover(&e.to_string(), attempts)? {
                    live::Recover::Retry => continue,
                    live::Recover::Clipboard => {
                        match live::copy_to_clipboard(&result.clean) {
                            Ok(()) => { render::paint_plain(&["  copied — your words are safe on the clipboard.".to_string()])?; }
                            Err(ce) => { render::paint_plain(&[format!("  clipboard failed too: {ce} — try [r]etry")])?; continue; }
                        }
                        return Ok(Some(Ok(())));
                    }
                    live::Recover::Discard => return Ok(Some(Ok(()))),
                }
            }
        }
    };
```

- [ ] **Step 3: Verify** — `cargo build --features listen` clean + clippy clean (the prompt path needs a TTY; verified manually by making the base dir read-only in a live run — an on-machine check, listed in the final sweep). Unit-test `copy_to_clipboard` lightly: on macOS dev machines `pbcopy` exists — `#[test] #[cfg(target_os = "macos")] fn clipboard_roundtrip() { copy_to_clipboard("x").unwrap(); }` (paste-back verification is manual; the test asserts the helper runs).

- [ ] **Step 4: Commit** — `git commit -am "feat: write-error recovery (retry / clipboard / discard) in the live path"`

---

## Task 9: First-run disclosure + `talk download verify` + first-run fetch offer

**Files:**
- Modify: `src/state.rs` (+ `disclosed: bool`)
- Modify: `src/main.rs`

Three small §8/§11/§7 items:

(a) **Disclosure (spec §8, "surfaced on first run"):** before the first non-ephemeral session ever starts, print once (then record in state):

```
talk keeps everything local. one honest note: your verbatim words are stored
as plaintext in ~/talk (inline raw comments — or ~/talk/.raw/ with
raw_sidecar = true). if you point ~/talk at a cloud-synced folder, your words
go to that cloud. `keep_raw = false` stores only the cleaned text. ephemeral
(`talk unburden`) keeps nothing — though your terminal's scrollback and OS
swap are beyond any app's reach.
```

Implementation: `State` gains `disclosed: bool` (serde default false). Add a helper `fn disclose_once(base: &Path) -> std::io::Result<()>` (load state → if `!disclosed`, eprint the note, set + `write_private` 0600). **Ordering matters, twice:** (1) it must fire only when a session actually proceeds — i.e. AFTER `require_text` in the `--from-text` arms and at the top of `run_live_session`'s session-shape branch — never before, or `talk journal` with no `--from-text` would write `.state.json` and break `missing_from_text_errors_without_writing`'s empty-dir assert; (2) it must fire BEFORE `reflect_choice` loads its own `State` copy, or reflect's later save (loaded pre-disclosure with `disclosed: false`) would clobber the flag. So: call `disclose_once` as the FIRST action of each non-ephemeral session arm (Journal/Reflect/bare, both text and live paths), before any other `State` load. Ephemeral (unburden/vent) never discloses — it writes nothing to disclose about. Integration test: first journal run's stderr contains "local"; a second run's doesn't; and `missing_from_text_errors_without_writing` still passes (no state written on the error path).

(b) **`talk download verify` (spec §11):** new arm in `handle_download`:

```rust
        Some("verify") => {
            #[cfg(feature = "download")]
            {
                let mut bad = 0;
                for art in download::models::MODELS {
                    let p = paths::models_dir().join(art.name);
                    match download::verify(&p, art.sha256) {
                        Ok(true) => println!("  ✓ {}", art.name),
                        _ => { println!("  ✗ {} (missing or hash mismatch)", art.name); bad += 1; }
                    }
                }
                if bad > 0 { std::process::exit(1); }
                Ok(())
            }
            #[cfg(not(feature = "download"))]
            { eprintln!("this build has no download support"); std::process::exit(2); }
        }
```

Integration test (download feature): with `TALK_MODELS_DIR` pointing at a temp dir containing a wrong-content file under a manifest name, `talk download verify` exits non-zero and names the artifact.

(c) **First-run fetch offer (spec §7, scoped):** in `run_live_session`'s models-not-ready arm, when stdin is a TTY (`libc::isatty(0) == 1` — `libc` is already a unix dependency), prompt `models not downloaded (~30 MB, one time). download now? [y/N] `, read one stdin line; `y`/`Y` → run the same fetch loop as `talk download models`, then continue into the session; anything else → the existing exit(1) hint. Non-TTY keeps today's exact behavior (tests unaffected). Resumable-partial download is NOT implemented (a failed fetch re-runs from zero; the verify-cached-skip already makes re-runs cheap) — recorded as the spec §7 deviation for the doc-review to weigh.

- [ ] Steps: failing integration tests for (a)+(b) → implement (a)(b)(c) → `cargo test` + `cargo test --features listen` green → `git commit -am "feat: first-run disclosure, download verify, first-run fetch offer"`

---

## Task 10: BYO near-match ("continue this thread?")

**Files:**
- Create: `crates/talk-core/src/matchq.rs` (+ `pub mod matchq;` in lib.rs)
- Modify: `src/main.rs` (live BYO path only)

Spec §8: a *rephrased* BYO question must not silently diverge into a new thread — on a near-match, offer to continue the existing one. The similarity is pure core logic (testable); the prompt is live-path-only (the `--from-text` seam keeps current behavior, so every existing test stands).

- [ ] **Step 1: Failing test + pure similarity** — create `crates/talk-core/src/matchq.rs`:

```rust
//! Near-match detection for bring-your-own questions, so a rephrasing offers to
//! continue the existing thread instead of silently forking it (spec §8).

use std::collections::HashSet;

fn content_set(q: &str) -> HashSet<String> {
    q.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2) // drop a/i/am/is-grade tokens
        .map(str::to_string)
        .collect()
}

/// Jaccard similarity over content tokens. 1.0 = same words, 0.0 = disjoint.
pub fn similarity(a: &str, b: &str) -> f32 {
    let (sa, sb) = (content_set(a), content_set(b));
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    inter / union
}

/// The existing question most similar to `q`, if it clears the near-match bar
/// and isn't an exact match (exact reuses the thread already, by slug).
pub fn near_match<'a>(q: &str, existing: &'a [String]) -> Option<&'a String> {
    existing
        .iter()
        .filter(|e| e.as_str() != q)
        .map(|e| (similarity(q, e), e))
        .filter(|(s, _)| *s >= 0.6)
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, e)| e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rephrasing_clears_the_bar_unrelated_does_not() {
        let existing = vec![
            "What am I avoiding?".to_string(),
            "Where does my anger live?".to_string(),
        ];
        assert_eq!(
            near_match("what am i avoiding right now", &existing),
            Some(&existing[0])
        );
        assert_eq!(near_match("how do I rest more deeply", &existing), None);
    }

    #[test]
    fn exact_match_is_not_offered() {
        let existing = vec!["What am I avoiding?".to_string()];
        assert_eq!(near_match("What am I avoiding?", &existing), None);
    }
}
```

- [ ] **Step 2: Wire (live only)** — in `run_live_session`'s Reflect arm, before `reflect_choice`, when `args.question` is `Some(q)`: collect existing questions (scan base-dir frontmatter, reusing the same iteration as `print_thread`/`find_by_question` — extract a small `fn existing_questions(base: &Path) -> Vec<String>` helper used by both); on `near_match` → stdin prompt `you've sat with "<existing>" before — continue that thread? [Y/n] ` (TTY only); `Y`/default → substitute the existing question string for `q` (the normal exact-match path then reuses its file). Non-TTY skips the prompt (current behavior).

- [ ] **Step 3: Run + commit** — `cargo test -p talk-core matchq` + full suites green → `git commit -am "feat: BYO near-match offers to continue the existing thread"`

---

## Task 11: Tamper test, no-egress test, CI

**Files:**
- Modify: `tests/privacy.rs`
- Create: `.github/workflows/ci.yml`

Spec §14's runtime privacy proofs, made executable:

- [ ] **Step 1: Model-tamper test** (spec §14: "a corrupted checksum must refuse to load") — append to `tests/privacy.rs`:

```rust
/// A tampered cached model must refuse to run (verify-before-load, spec §11/§14).
/// Runs only in listen builds: the gate lives in the live-session path.
#[cfg(feature = "listen")]
#[test]
fn tampered_model_refuses_to_run() {
    let home = tempfile::tempdir().unwrap();
    let models = tempfile::tempdir().unwrap();
    // Place WRONG bytes at every manifest name: present, but hash-mismatched.
    for name in [
        "sherpa-onnx-moonshine-tiny-en-quantized-2026-02-27.tar.bz2",
        "silero_vad.onnx",
    ] {
        std::fs::write(models.path().join(name), b"tampered").unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_talk"))
        .args(["reflect"]) // no --from-text → live path → verify gate fires pre-mic
        .env("HOME", home.path())
        .env("TALK_MODELS_DIR", models.path())
        .output()
        .unwrap();
    assert!(!out.status.success(), "tampered models must exit non-zero");
    assert!(String::from_utf8_lossy(&out.stderr).contains("talk download models"));
}
```

(Verified here with `cargo test --features listen --test privacy`. The happy path with REAL models is already covered by the on-machine Plan-2 verification.)

- [ ] **Step 2: No-egress runtime test (macOS)** — append:

```rust
/// Spec §14: a full session makes zero outbound connections. macOS: run the
/// session under a deny-network sandbox profile; if the session path ever
/// gained a network call, the sandbox would kill it and the run would fail.
#[cfg(target_os = "macos")]
#[test]
fn session_runs_under_deny_network_sandbox() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new("sandbox-exec")
        .args([
            "-p",
            "(version 1)(allow default)(deny network*)",
            env!("CARGO_BIN_EXE_talk"),
            "journal", "--from-text", "no packets were harmed",
            "--date", "2026-06-09", "--time", "08:14",
        ])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert!(std::fs::read_to_string(home.path().join("talk/2026-06-09.md")).unwrap().contains("No packets were harmed."));
}
```

(`sandbox-exec` is deprecated-but-present on macOS; if it's ever removed the test fails loudly and gets ported, not silently skipped. Verified here.)

- [ ] **Step 3: CI** — create `.github/workflows/ci.yml`:

```yaml
name: ci
on:
  push: { branches: [main] }
  pull_request:

jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: clippy }
      - run: cargo test --workspace
      - run: cargo clippy --all-targets -- -D warnings
      - name: no-egress (Linux netns)
        if: runner.os == 'Linux'
        run: |
          cargo build
          HOME=$(mktemp -d) unshare -rn target/debug/talk journal \
            --from-text "no packets were harmed" --date 2026-06-09 --time 08:14
      - name: no-egress (macOS sandbox) + privacy suite
        if: runner.os == 'macOS'
        run: cargo test --test privacy

  listen-build:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo build --features listen
      - run: cargo test --features listen
```

(Authored here; the runner behavior is verified on the first push — the one thing this environment can't execute. The MSRV check is deliberately not in CI yet: the 1.82 toolchain check is a release-time gate, documented in the plan, to keep CI fast.)

- [ ] **Step 4: Run what's runnable + commit** — `cargo test --test privacy` (bare + `--features listen`) green locally → `git add tests/privacy.rs .github/ && git commit -m "test: tamper + no-egress privacy proofs; ci workflow"`

---

## Task 12: Spec §17 acceptance sweep

**Files:** none new — a verification task. Walk every §17 criterion and record the proving artifact:

| §17 criterion | Proof |
|---|---|
| `talk` asks a curated question, listens on-device, settles live, appends with frontmatter+raw | Plan-2 acoustic-loopback session + T1/T2 (real spine) + a fresh on-machine `talk reflect` after this plan merges |
| `talk journal` writes/appends date-keyed | existing integration tests |
| `talk "my question"` derives slug + reuses file | existing integration tests + T10 near-match |
| `talk unburden` keeps nothing; Released screen | T7 zero-bytes test |
| `talk thread` surfaces accumulated file | T3 + existing collision test |
| cleanup levels work; raw recoverable via `u`; diff-guard rejects over-edits | Plan-3 suite (moat + eval) |
| core compiles `--no-default-features`, zero network in session | bare build (no features = no ureq/cpal) + T11 no-egress |
| `held:7` serves one question across 7 days | NEW integration test (below) |
| terminal restores on quit and panic | Screen RAII (Plan 2) + on-machine check |

- [ ] **Step 1: The held:7 acceptance test** — add to `tests/integration.rs`:

```rust
#[test]
fn held_seven_serves_one_question_across_days() {
    let dir = tempfile::tempdir().unwrap();
    let talk_dir = dir.path().join("talk");
    std::fs::create_dir_all(&talk_dir).unwrap();
    std::fs::write(talk_dir.join("config.toml"), "default_pack = \"held\"\n").unwrap();
    for day in 1..=7 {
        let date = format!("2026-06-{:02}", day);
        let out = talk(dir.path(), &["--from-text", "held words", "--date", &date, "--time", "12:00"]);
        assert!(out.status.success());
    }
    // All seven landed in ONE held question's file, as seven dated sections.
    let files: Vec<_> = std::fs::read_dir(&talk_dir).unwrap().flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    assert_eq!(files.len(), 1, "one thread file, got {files:?}");
    let text = std::fs::read_to_string(&files[0]).unwrap();
    assert_eq!(text.matches("## 2026-06-").count(), 7);
    // The 8th day releases the run to a different question.
    talk(dir.path(), &["--from-text", "released", "--date", "2026-06-08", "--time", "12:00"]);
    let count = std::fs::read_dir(&talk_dir).unwrap().flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md")).count();
    assert_eq!(count, 2);
}
```

(Selection serves the held question on day 1 only if it wins rotation — the `held` pack contains ONLY held questions, so day 1 picks one and the run then pins it. Day 8 picks a *different* held question by rotation.)

- [ ] **Step 2: Full sweep** — `cargo test` + `cargo test --features listen` + `cargo clippy --all-targets --features listen` all green; run the table above top to bottom and record each proof in the commit message.

- [ ] **Step 3: Commit** — `git commit -am "test: held:7 acceptance + spec §17 sweep"`

---

## Self-Review (completed during authoring)

- **Spec coverage (Plan-4 scope):** real 65-spine vendored from the named source §9 (T1) · four flagship packs + registry + `default_pack` §9/§12 (T2) · `talk download` no-arg lists installed §7 (T2) · thread list + empty state §7 (T3) · streak port §12 (T4) · held label + close provenance §7 (T5) · sidecar raw store §8 (T6) · ephemeral zeroize + zero-bytes test §7/§14 (T7) · write-error recovery retry/clipboard/discard §13 (T8) · first-run disclosure §8 + `download verify` §11 + first-run fetch offer §7 (T9) · BYO near-match §8 (T10) · model-tamper + no-egress tests §14 + CI (T11) · held:7 acceptance + full §17 sweep (T12). **Deliberate deviations, surfaced for review:** remote pack download deferred (no packs exist to fetch — YAGNI; the machinery exists); mlock skipped with stated rationale; resumable-partial model download not implemented (verify-cached-skip covers the practical case); streak-gated depth stays v2 per the spec's resolved decision.
- **Placeholder scan:** T1's 65 entries are a mechanical conversion from a named, verified source (counts confirmed: 10/15/15/40) with binding rules + worked examples + a shape-asserting test — not invented content, not a placeholder. T2's flagship questions are fully authored in this document. All other tasks carry complete code.
- **Type consistency:** `Pack`/`Question` (existing core) consumed by `packs::{vendored, by_name}` · `reflect_choice(base, byo, time, default_pack)` updated at both call sites · `ReflectChoice.held_day: Option<u32>` ↔ `held_day_for(&Option<(String,u32)>, &str, &str)` ↔ `LiveConfig.held_label: Option<&str>` (formatted into an outliving binding) · `WriteRequest.raw_sidecar` + `RunConfig.raw_sidecar` threaded from `Config.raw_sidecar` · `streak::{Streak, civil_day, record_entry}` used in `run_and_report` (via `Report.date`) + `run_live_session` · `matchq::{similarity, near_match}` live-only · `Frontmatter.{slug, entries, last}` (verify field names at T3 implementation) · disclosure flag `State.disclosed` (serde default).
- **Test seam discipline:** every interactive behavior (fetch offer, near-match prompt, write-error recovery) is TTY-gated, so the entire `--from-text` test suite is untouched by default; each gets either a pure-logic unit test, an integration test, or a named on-machine check.
- **Execution venue:** everything is buildable AND verifiable in this environment (it's the user's Mac: sandbox-exec, pbcopy, TTY-via-`script` if needed) except the CI runner behavior (verified on first push) and the final feel-check of interactive prompts.

---

## Execution Handoff

Two options (per superpowers:writing-plans): **1. Subagent-Driven (recommended)** — fresh subagent per task with spec+quality review between tasks; **2. Inline Execution** with checkpoints. Consistent with the review-gated workflow: run `/ce-doc-review` on this plan before building.
