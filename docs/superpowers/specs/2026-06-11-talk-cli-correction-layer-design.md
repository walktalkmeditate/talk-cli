---
title: talk-cli correction layer (sound-tag stripping + personal lexicon)
date: 2026-06-11
status: design
origin: user request — first real journal entry had (buzzer) artifacts and mis-heard proper nouns (talk→"TOC", walk→"WOC", Claude→"cloth")
---

# talk correction layer

**Goal:** Two deterministic, no-model fixes that improve every transcribed entry:
strip Whisper's non-speech tags (`(buzzer)`, `[BLANK_AUDIO]`), and let the user
teach talk the proper nouns it mishears via a personal lexicon. Both preserve
talk's restraint: the verbatim raw is never altered, and the lexicon corrects
only what the user explicitly authorizes.

**Architecture:** Add two pure functions to `talk-core` — `strip_sound_tags` and
`apply_lexicon` — and apply them to the transcript text **before the existing
pre-layer and the content-word guard**, on the clean path only, at the
**live mic path (`src/live.rs::apply_event`, the primary site — this is where the
originating bug lives) and the `--from-text` path (`src/session.rs::run`)**, on both
their Commit and Revise arms. The lexicon is loaded once from a new
`~/.config/talk/lexicon.toml` before the terminal screen is entered; the raw
verbatim transcript is stored untouched so `u`-toggle and recovery still show
exactly what was said.

**Tech stack:** Rust, `toml` (already a dep), `serde` (already a dep). No new
dependencies, no model, no network.

---

## Background

The first real `talk journal` entry surfaced two un-handled classes of noise:

1. **Whisper non-speech tags.** Pass-2 (Whisper base.en) emits sound events as
   bracketed/parenthesized tokens — `(buzzer)`, `[BLANK_AUDIO]`, `(music)` — which
   land verbatim in the written entry. Nothing strips them today (`cleanup.rs` has
   no such pass).
2. **Mis-heard proper nouns.** The speech models have never heard the
   Walk·Talk·Meditate / Claude vocabulary, so they guess the nearest common words:
   `talk → "TOC"`, `walk → "WOC"/"WAC"`, `Claude → "cloth"`. There is no correction
   layer at all.

Both are deterministic to fix. The two challenges are *which code path* to fix
(there are two transcript pipelines, and only one produced the bug) and *where* the
corrections sit relative to the existing **content-word guard** (`guard_accepts` in
`talk-core/src/cleanup.rs`), which accepts a rewrite only if it preserves every
content word, in order.

### The two transcript paths (the path that matters)

There are two independent transcript pipelines, each with its own Commit/Revise
handling:

- **`src/live.rs::apply_event` (the live mic path — PRIMARY).** This is what a real
  `talk` / `talk journal` session runs through (`live::run_loop`, main.rs:358). Its
  Commit arm computes `pre = apply_backtrack(apply_spoken_commands(raw))` and calls
  `deterministic_light(pre)` directly; its Revise arm calls `format_revise(raw2, …)`.
  **The mis-heard nouns and `(buzzer)` artifacts came from here.** It carries a
  `LiveConfig`, not a `RunConfig`, and never calls `session::run`.
- **`src/session.rs::run` (the `--from-text` path — SECONDARY).** Reached only by
  `run_and_report` (main.rs:174) for `talk "text"` / journal-from-text / the
  `FakeTranscript` tests. It calls `guarded_format` (Commit) and `format_revise`
  (Revise).

The correction layer must be wired into **both**, but `live.rs` is where the
originating bug is actually fixed. A fix that only touched `session.rs::run` would
leave the live journal broken.

### Why the corrections run pre-guard

- **Sound-tags must be stripped pre-guard.** `(buzzer)` tokenizes to the content
  word `buzzer`. On the `session.rs` path the guard (`guard_accepts(pre, candidate)`
  inside `guarded_format`) compares the pre-layer text against the formatter output;
  if the candidate dropped `buzzer` but `pre` still contained it, the guard would
  reject the strip. Stripping in the pre-layer (so `pre` is already clean) keeps the
  comparison consistent. (The `live.rs` Commit arm and both Revise arms have no
  content-word guard, so for them stripping pre-layer is simply correct, not
  guard-sensitive.)
- **Lexicon is a user-authorized meaning change.** `TOC → talk` is exactly the
  substitution the guard exists to forbid for the *model*. But the user asked for
  it, so it belongs in the pre-layer: it happens first, everything downstream sees
  "talk", and the guard's "never change meaning" promise stays intact for the model.

---

## Components

| Unit | File | Responsibility |
|------|------|----------------|
| `strip_sound_tags(text) -> String` | `talk-core/src/cleanup.rs` (pure) | Remove non-speech tags |
| `apply_lexicon(text, &corrections) -> String` | `talk-core/src/lexicon.rs` (NEW, pure) | Word-bounded user substitutions, single-pass |
| `find_word_bounded` visibility | `talk-core/src/cleanup.rs` | Change `fn` → `pub(crate) fn` so the sibling `lexicon` module can reuse the boundary scan |
| `Lexicon::load` / `Lexicon::correct` | `src/lexicon.rs` (NEW, binary) | Read `lexicon.toml`, hold the map, apply via `apply_lexicon` |
| `correct(raw, &Lexicon) -> String` | `src/lexicon.rs` (binary) | `strip_sound_tags(&lexicon.correct(raw))` — the one helper both paths call |
| `lexicon_path()` | `src/paths.rs` | `config_dir().join("lexicon.toml")` |
| lexicon template + `config init` write | `src/lexicon.rs` (template), `src/main.rs::handle_config` | The commented `lexicon.toml`; `handle_config`'s init arm writes it alongside `config.toml` |
| `LiveConfig.lexicon` + `apply_event` param | `src/live.rs` | Thread `lexicon: &Lexicon`; apply `correct` to `raw`/`raw2` in the Commit and Revise arms before the existing pre-layer |
| `RunConfig.lexicon` + Commit/Revise wiring | `src/session.rs` | Thread `lexicon: &Lexicon`; apply `correct` before `guarded_format` / `format_revise` |
| lexicon load site | `src/main.rs` | Load once before the live `Screen` is entered; thread into `LiveConfig`/`RunConfig` |

### `strip_sound_tags`

- **Bracketed `[...]` spans: removed only when the inner text is a known sound tag OR
  matches Whisper's event shape** (all-caps with optional underscores/spaces, e.g.
  `[BLANK_AUDIO]`, `[MUSIC]`). Removal is *not* unconditional: Whisper can bracket
  non-event text, and these spans run pre-guard where an over-deletion has no safety
  net.
- **Parenthesized `(...)` spans: removed when every inner word (trimmed, lowercased)
  is in the curated non-speech set.** This catches single- and multi-word events
  (`(buzzer)`, `(wind blowing)`, `(clears throat)`). Set: `buzzer, buzzing, music,
  applause, applauding, laughter, laughs, laughing, coughs, coughing, cough, sighs,
  sigh, beep, beeping, breathing, breath, breathes, static, noise, silence,
  blank_audio, wind, blowing, clears, throat, typing, footsteps, door, closes, knock,
  knocking, indistinct, inaudible, sniffles, chuckles`. A legitimate aside like
  `(I think)` is kept (its words are not all in the set).
- Collapses any whitespace left by a removed span (`"woke up (buzzer) early"` →
  `"woke up early"`).
- Lives in `talk-core` (pure) and is invoked from the binary `correct` helper.

### `apply_lexicon`

- Signature: `apply_lexicon(text: &str, corrections: &[(String, String)]) -> String`.
  The binary passes a slice **pre-sorted by descending key length** so longest-first
  matching is deterministic (a `BTreeMap` orders lexicographically, not by length, so
  the sort is explicit, not incidental).
- **Single left-to-right pass over the original input.** Walk an output cursor across
  the input; at each word boundary, try the corrections longest-key-first; on a
  match, emit the value and advance the cursor **past the matched span** so
  substituted output is never re-scanned. This guarantees termination and
  well-defined behavior for cyclic maps (`{"a"="b","b"="a"}` swaps once, does not
  loop) and value-contains-key maps (no cascade).
- **Word-bounded.** A key matches only on whole-word boundaries, reusing
  `pub(crate) find_word_bounded` (which expects an **already-lowercased needle** and
  returns a byte offset valid in the original) — so `"TOC" → "talk"` fires on `TOC`
  but never inside `STOCK`.
- **Case sensitivity:** keys match **case-insensitively** (a mis-hear's casing is
  unpredictable — `TOC`/`Toc`/`toc`); the value is emitted as written. Note: the
  value is **not** immune to downstream casing — on the live/`--from-text` Commit
  path, `deterministic_light` still capitalizes sentence starts and a standalone `i`,
  so a deliberately-lowercase value (`"ebay"`) at a sentence start becomes `"Ebay"`.
  On the Revise path (`format_revise`) Whisper's casing is trusted and the value is
  preserved. This is accepted behavior, not a "verbatim" guarantee; it is tested.
- **Empty corrections → identity.** No corrections configured = no change (the
  default).

### `Lexicon` (binary)

- `Lexicon::load(path) -> Lexicon`: parse `[corrections]` from `lexicon.toml`,
  building the descending-length-sorted slice. **Missing file → empty (silent).
  Malformed TOML → warn once to stderr, treat as empty.** Loading happens in
  `main.rs` **before the live `Screen` is entered**, so a malformed-config warning
  prints to the normal terminal and never corrupts the alternate-screen TUI.
- `Lexicon::correct(&self, text) -> String` delegates to `apply_lexicon`; the
  module-level `correct(raw, &lexicon)` helper composes it with `strip_sound_tags`.
- The shipped template (written by `talk config init` via `handle_config`'s init arm)
  is fully commented — examples visible, nothing active until uncommented:

```toml
# talk lexicon — teach talk the proper nouns it mishears.
# Word-bounded, case-insensitive match; the value sets the spelling
# (sentence-start capitalization still applies on the live path).
# Uncomment and edit; talk corrects nothing until you do.
#
# [corrections]
# "TOC"   = "talk"        # the tool's own name
# "WOC"   = "walk"
# "WAC"   = "walk"
# "cloth" = "Claude"
# "Obsidian" = "Obsidian" # force exact casing
# "Pilgrim"  = "Pilgrim"
# "Ellen"    = "Ellen"    # names talk guesses wrong
```

---

## Data flow

At each Commit and Revise arm of **both** transcript paths, the binary `correct`
helper runs on the transcript text before the existing cleanup, feeding only the
clean path; the verbatim raw is stored untouched.

**`live.rs::apply_event`** (primary — gains a `lexicon: &Lexicon` parameter):

```
Commit(raw):   let c = correct(&raw, lexicon);                      // lexicon then sound-tags
               let pre = apply_backtrack(apply_spoken_commands(c)); // existing pre-layer
               settle.commit(&raw, &deterministic_light(&pre));     // raw verbatim, clean corrected
Revise(raw2):  let c = correct(&raw2, lexicon);
               settle.revise_committing(&raw2, &format_revise(&c, prev));  // raw2 verbatim
```

**`session.rs::run`** (secondary — `RunConfig` gains `lexicon: &Lexicon`):

```
Commit(raw):   settle.commit(&raw, &guarded_format(fmt, level, &correct(&raw, lexicon)));
Revise(raw2):  settle.revise_committing(&raw2, &format_revise(&correct(&raw2, lexicon), prev));
```

- **`correct` = `strip_sound_tags(&lexicon.correct(raw))`** — lexicon first, so a
  corrected word can never be mistaken for a sound tag.
- The verbatim `raw`/`raw2` passed to `settle.commit` / `settle.revise_committing` is
  the **original** string. `keep_raw`/`raw_sidecar` therefore preserve the literal
  mishearing for recovery and the `u` raw⇄clean toggle.
- On the `session.rs` Commit path, `guarded_format` recomputes its own pre-layer over
  the already-corrected text, so the guard compares corrected-vs-corrected —
  consistent, no spurious fallback.

### Note on precedence vs spoken commands

Because `correct` runs before `apply_spoken_commands`/`apply_backtrack`, a lexicon
value (or a corrected token) that spells a command literal (`period`, `comma`,
`new line`, `new paragraph`) or forms a backtrack trigger (`scratch that`) will then
be consumed by the command layer — **by design** (corrections produce the user's
intended words, which the command layer then interprets). This precedence is an
explicit, tested contract.

---

## Error handling

- Missing `lexicon.toml` → empty corrections, no message (the zero-config default).
- Malformed `lexicon.toml` → single stderr warning **emitted before the live screen
  is entered**, empty corrections, session continues.
- A correction whose value re-introduces a sound-tag word is harmless: sound-tag
  stripping only fires on *bracketed/parenthesized* spans, not bare words.

---

## Testing

**`apply_lexicon` (pure):**
- `"TOC"→"talk"` fires on the whole word, not inside `STOCK`.
- Case-insensitive key match (`toc`/`TOC`/`Toc` → `talk`).
- Multi-word key (`"talk CLI"→"talk"`); longest-key-first when keys overlap
  (`"talk CLI"` beats `"talk"`).
- **Single-pass termination:** cyclic map `{"a"="b","b"="a"}` swaps exactly once;
  value-contains-key map does not cascade.
- Empty corrections is identity; no-match text unchanged.

**`strip_sound_tags` (pure):**
- `(buzzer)`, `(wind blowing)`, `(clears throat)`, `[BLANK_AUDIO]`, `[MUSIC]` removed;
  surrounding whitespace collapsed; multiple tags per line all removed.
- A bare word `buzzer` (no brackets) is **kept**; a non-sound aside `(I think)` is
  **kept**; a non-event bracketed span (e.g. `[7]` or `[Smith]`) is **kept** (not
  all-caps-event shape, not a known tag).

**Integration (`live.rs::apply_event` — primary, and `session.rs::run`):**
- A Commit/Revise carrying `TOC` with `{TOC→talk}` writes `talk` in the entry while
  the `<!-- raw: … TOC … -->` comment stays verbatim — asserted on the `live.rs` path.
- A Commit/Revise with `(buzzer)` writes the entry without it; raw keeps it.
- **Raw/clean divergence is a pinned contract:** a multi-word correction produces a
  documented, asserted difference between the stored raw and the written clean (so the
  `u`-toggle divergence is intentional and tested, not accidental).
- Sentence-start casing: a lowercase-leading value at a sentence start is capitalized
  by `deterministic_light` on the Commit path (asserted), and preserved on the Revise
  path.
- The existing `apply_event` pairing/pause tests pass an empty lexicon (identity), so
  their behavior is unchanged.
- Guard-consistency (`session.rs` path): corrected + tag-stripped text still passes
  the content-word guard (no spurious Light fallback).

**Config:**
- The shipped `lexicon.toml` template parses to an **empty** corrections map.
- `talk config init` writes both `config.toml` and `lexicon.toml`.

---

## Out of scope

- Fuzzy / phonetic matching or auto-suggesting corrections from entries — a possible
  later enhancement.
- Correcting the verbatim raw transcript (deliberately preserved as-said).
- Protecting lexicon values from downstream sentence-start capitalization (accepted
  behavior for v1; proper nouns are normally capitalized anyway).
- Any model-based correction (that is Spec B, the formatter).
