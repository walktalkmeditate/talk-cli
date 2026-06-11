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
`apply_lexicon` — and run both in the **pre-layer**, before the content-word guard
and the formatter, on the clean path only. The lexicon is loaded from a new
`~/.config/talk/lexicon.toml` (binary-side); the raw verbatim transcript is stored
untouched so `u`-toggle and recovery still show exactly what was said.

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

Both are deterministic to fix. The challenge is *where* they apply relative to the
existing **content-word guard** (`guard_accepts` in `talk-core/src/cleanup.rs`),
which accepts a rewrite only if it preserves every content word, in order.

### Why the pre-layer

The commit path is `guarded_format(formatter, level, raw)` (in
`talk-core/src/format.rs`), which computes `pre = apply_backtrack(apply_spoken_commands(raw))`,
runs the formatter, and accepts the candidate only if `guard_accepts(pre, candidate)`.

- **Sound-tags must be stripped pre-guard.** `(buzzer)` tokenizes to the content
  word `buzzer`. Stripping it *after* the guard would register as a content-word
  change and be rejected. Stripping it in the pre-layer means the guard compares
  already-clean text on both sides — consistent.
- **Lexicon is a user-authorized meaning change.** `TOC → talk` is exactly the
  substitution the guard exists to forbid for the *model*. But the user asked for
  it, so it belongs in the pre-layer: it happens first, everything downstream
  (caps, guard, formatter) sees "talk", and the guard's "never change meaning"
  promise stays intact for the model. The lexicon is *your* edit, not the model's.

---

## Components

| Unit | File | Responsibility |
|------|------|----------------|
| `strip_sound_tags(text) -> String` | `talk-core/src/cleanup.rs` (pure) | Remove non-speech tags |
| `apply_lexicon(text, &corrections) -> String` | `talk-core/src/lexicon.rs` (NEW, pure) | Word-bounded user substitutions |
| `Lexicon::load` / `Lexicon::correct` | `src/lexicon.rs` (NEW, binary) | Read `lexicon.toml`, hold the map, apply via `apply_lexicon` |
| `lexicon_path()` | `src/paths.rs` | `config_dir().join("lexicon.toml")` |
| lexicon template | `src/lexicon.rs` (binary) | The commented `lexicon.toml` that `config init` writes |
| pre-layer wiring | `talk-core/src/cleanup.rs`, `src/session.rs` | Apply lexicon → sound-tags ahead of the existing pre-layer, both Commit and Revise paths |

### `strip_sound_tags`

- **Bracketed `[...]` spans: always removed.** Whisper emits these only as sound
  events (`[BLANK_AUDIO]`, `[MUSIC]`); a user cannot dictate square brackets, so
  removal is unconditionally safe.
- **Parenthesized `(...)` spans: removed only when the inner text (trimmed,
  lowercased) is a known sound tag.** Curated set: `buzzer, buzzing, music,
  applause, laughter, laughs, coughs, coughing, cough, sighs, sigh, beep, beeping,
  breathing, breath, static, noise, silence, blank_audio`. This avoids deleting a
  legitimate spoken aside, even though dictating literal parentheses is unlikely.
- Collapses any whitespace left by a removed span so the surrounding text reads
  cleanly (`"woke up (buzzer) early"` → `"woke up early"`).

### `apply_lexicon`

- Signature: `apply_lexicon(text: &str, corrections: &BTreeMap<String, String>) -> String`.
- **Word-bounded substitution.** A key matches only on whole-word boundaries
  (non-alphanumeric or string edge on both sides), so `"TOC" → "talk"` fires on
  `TOC` but never inside `STOCK`. Reuses the boundary logic already proven in
  `find_word_bounded` (cleanup.rs).
- **Case sensitivity:** keys match **case-insensitively** (the model's casing of a
  mis-hear is unpredictable — `TOC`, `Toc`, `toc`); the **value** is emitted
  verbatim (the user controls the canonical casing).
- **Multi-word keys** supported (`"talk CLI"`). Matching is left-to-right,
  non-overlapping; longer keys are tried before shorter to avoid partial shadowing.
- **Empty map → identity.** No corrections configured = no change (the default).

### `Lexicon` (binary)

- `Lexicon::load(path) -> Lexicon`: parse `[corrections]` from `lexicon.toml` into a
  `BTreeMap<String, String>`. **Missing file → empty map (silent).** **Malformed
  TOML → warn once to stderr, treat as empty** — a config typo never blocks a
  session.
- `Lexicon::correct(&self, text) -> String` delegates to `apply_lexicon`.
- The shipped template (written by `talk config init`) is a fully-commented file —
  examples visible, nothing active until uncommented:

```toml
# talk lexicon — teach talk the proper nouns it mishears.
# Word-bounded, case-insensitive match; the value sets the exact casing.
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

In `session.rs::run`, on both the `Commit` and `Revise` paths, the correction layer
runs ahead of the existing cleanup, on the **clean path only**:

```
raw ──► lexicon.correct ──► strip_sound_tags ──► [existing pre-layer + format + guard] ──► clean
raw (verbatim, untouched) ──────────────────────────────────────────────────────────► <!-- raw: -->
```

- **Lexicon runs before sound-tags** so a corrected word can never be mistaken for a
  sound tag.
- `strip_sound_tags` is also folded into the shared pre-layer used by `format_revise`
  (the Whisper pass-2 path in `talk-core/src/cleanup.rs`), since sound tags arrive
  mostly on the revise text.
- The verbatim `raw` passed to `settle.commit(&raw, &clean)` is the **original**
  string — corrections and tag-stripping touch only what becomes the written entry.
  `keep_raw`/`raw_sidecar` therefore still preserve the literal mishearing for
  recovery and the `u` raw⇄clean toggle.

The lexicon is loaded once at session start (binary) and threaded into `RunConfig`
(a new `lexicon: &Lexicon` field), so the pure core stays map-driven and testable.

---

## Error handling

- Missing `lexicon.toml` → empty map, no message (the zero-config default).
- Malformed `lexicon.toml` → single stderr warning, empty map, session continues.
- A correction value that re-introduces a sound-tag word is harmless: sound-tag
  stripping only fires on *bracketed/parenthesized* spans, not bare words.

---

## Testing

**`apply_lexicon` (pure):**
- `"TOC"→"talk"` fires on the whole word, not inside `STOCK`.
- Case-insensitive key match; value casing preserved (`toc`/`TOC`/`Toc` → `talk`).
- Multi-word key (`"talk CLI"→"talk"`); longest-key-first when keys overlap.
- Empty map is identity; no-match text is unchanged.

**`strip_sound_tags` (pure):**
- `(buzzer)` and `[BLANK_AUDIO]` removed; surrounding whitespace collapsed.
- Multiple tags in one line all removed.
- A bare word `buzzer` (no brackets) is **kept**.
- A non-sound parenthetical (`(I think)`) is **kept** (not in the curated set).

**Integration (`session.rs`):**
- A `Commit` containing `TOC` with `{TOC→talk}` writes `talk` in the entry while the
  `<!-- raw: ... TOC ... -->` comment stays verbatim.
- A `Commit` with `(buzzer)` writes the entry without it; raw keeps it.
- Guard-consistency: lexicon-corrected + tag-stripped text still passes the existing
  content-word guard (no spurious fallback).

**Config:**
- The shipped `lexicon.toml` template parses to an **empty** corrections map.
- `talk config init` writes both `config.toml` and `lexicon.toml`.

---

## Out of scope

- Fuzzy / phonetic matching or auto-suggesting corrections from entries (the
  "suggest from your entries" option) — a possible later enhancement.
- Correcting the verbatim raw transcript (deliberately preserved as-said).
- Any model-based correction (that is Spec B, the formatter).
