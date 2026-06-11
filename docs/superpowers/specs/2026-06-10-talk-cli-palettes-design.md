---
title: talk-cli palettes — legible default + pinnable themes
date: 2026-06-10
status: design
origin: field feedback (rust diff text hard to read on dark terminals)
---

# talk palettes — legible default + pinnable themes

**Goal:** Make talk's live transcription legible on any terminal, and let the user
pin a palette. Fix the crushed default tones, then ship a small baked-in set
(`rust`, `high-contrast`, `mono`) selectable via config + a `--palette` flag.

**Architecture:** Replace the multiplicative tone derivation in `talk-core::palette`
with explicit, hand-tuned `(core, dim, edge)` triples per named theme. Tones become a
`Tone` enum so `mono` can use the terminal's *own* foreground (matching any
background). The binary's renderer maps `Tone` → crossterm styling. A `--palette`
flag + `palette =` config select the theme; default is `rust`.

**Tech stack:** Rust, `talk-core` (pure), `crossterm` (binary render layer), `clap`.

---

## Background — the bug

`talk-core::palette::palette()` derives the three tones by multiplying the brand
`RUST = (160, 99, 75)` toward black:

```rust
Palette { core: RUST, dim: scale(RUST, 0.6), edge: scale(RUST, 0.35) }
```

So `edge = (56, 34, 26)` (luminance ~40). In `render_model::compose`, the **header,
the question box, the borders, and the hint/status line** all paint at `edge` — the
dimmest tone — so on a near-black terminal they are effectively invisible. The
settled text (`core`) is the only readable element. This is a legibility defect in
the default, independent of any theming feature: multiplicative dimming crushes the
quiet tones, and the most important chrome (the question) sits at the very bottom of
the hierarchy.

This was confirmed by rendering the four candidate palettes as real terminal frames
on a dark background (see the brainstorm preview): `CURRENT` loses the question and
status entirely; the re-derived `rust` keeps everything readable with the hierarchy
intact.

## Design

### Tone model

`talk-core::palette` gains two enums and a theme-parameterized constructor. It stays
pure (no `crossterm`, no `clap`):

```rust
/// One paintable tone. `Color` is an explicit RGB; the `Terminal*` variants defer
/// to the terminal's own foreground so `mono` matches any background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Color(Rgb),
    Terminal,        // the terminal's default foreground, normal intensity
    TerminalFaint,   // the terminal's default foreground, faint (ANSI dim)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Theme {
    #[default] Rust,
    HighContrast,
    Mono,
}

pub struct Palette { pub core: Tone, pub dim: Tone, pub edge: Tone }

pub fn palette(theme: Theme) -> Palette { /* explicit triples below */ }
```

`Theme` also exposes parsing for the config string and a name list for errors/help:

```rust
impl Theme {
    pub const NAMES: [&'static str; 3] = ["rust", "high-contrast", "mono"];
    pub fn from_str(s: &str) -> Option<Theme>; // trims + lowercases; canonical names only
}
```

The existing `RUST` const stays as the brand anchor and is referenced in a comment as
the light-mode source of the dark-variant `rust` core below.

### The three palettes (exact tuned values)

`core` ≥ `dim` ≥ `edge` in luminance, every `Color` tone above a no-crush floor.
Values are tuned against the preview; the implementer may nudge ±8 per channel only
if the luminance-floor and ordering tests below still pass.

| Theme | core | dim | edge |
|-------|------|-----|------|
| `rust` (default) | `Color(206,140,112)` | `Color(158,110,92)` | `Color(132,104,94)` |
| `high-contrast` | `Color(236,205,186)` | `Color(198,152,124)` | `Color(170,136,120)` |
| `mono` | `Terminal` | `TerminalFaint` | `TerminalFaint` |

`rust` is the dark-terminal variant of the brand rust — brighter than the light-mode
`(160,99,75)` because a light-mode color reads too dark on a dark background. `mono`
collapses `dim` and `edge` into one faint level (ANSI has a single faint attribute);
that is an accepted limitation — `mono`'s value is matching the user's terminal theme
on **any** background, including light terminals, without background detection.

### Renderer: Tone → crossterm

`src/render/mod.rs::paint` currently does `SetForegroundColor(Rgb)` per line. It now
takes the resolved `Palette` and maps each `Tone`, always setting intensity
explicitly so a faint line never bleeds into the next:

```rust
fn apply_tone(out: &mut impl Write, tone: Tone) -> io::Result<()> {
    match tone {
        Tone::Color(c) => queue!(out,
            style::SetAttribute(style::Attribute::NormalIntensity),
            style::SetForegroundColor(rust(c)))?,
        Tone::Terminal => queue!(out,
            style::SetAttribute(style::Attribute::NormalIntensity),
            style::SetForegroundColor(style::Color::Reset))?,
        Tone::TerminalFaint => queue!(out,
            style::SetAttribute(style::Attribute::Dim),
            style::SetForegroundColor(style::Color::Reset))?,
    }
    Ok(())
}
```

`paint` resets color **and** attributes at the end of the frame
(`ResetColor` + `SetAttribute(Reset)`). The `LineKind → Tone` mapping is unchanged:
`Settled → core`, `Edge → dim`, `Chrome → edge`.

### Selection surface

- **Flag:** a global `--palette <rust|high-contrast|mono>` on `Cli`, as a clap
  `ValueEnum` (`PaletteArg`) bridged to `talk_core::palette::Theme` — mirroring
  meditate's `PalettePin`. clap validates the value and lists choices in `--help`.
- **Config:** `palette: Option<String>` in `Config`, parsed via `Theme::from_str`.
  A new commented line in the `config init` template.
- **Resolution (precedence flag > config > default):**

  ```text
  let theme = cli.palette.map(Theme::from)            // ValueEnum → Theme
      .or_else(|| config.palette.as_deref().and_then(Theme::from_str))
      .unwrap_or_default();                            // Rust
  ```

  An invalid **config** string is a hard error at startup: `unknown palette "<x>";
  valid: rust, high-contrast, mono` (fail fast, per the error-handling standard). An
  invalid **flag** is rejected by clap before this point. Zero-config launches in
  `rust`.

- The resolved `Palette` is computed once before the live loop and passed into
  `paint`. `live.rs`/`session.rs` thread it through; `render_model` (pure, in
  `talk-core`) is untouched — it still emits `LineKind`.

### Scope

- Applies to the **live TUI tones only**: header, question box, settled text, live
  edge, status line.
- The close (`compose_close`) and released (`compose_released`) screens already print
  in the terminal's default color and **stay as-is** — the return to plain text after
  a session is intentional and already legible.
- `--from-text` and non-session commands (config/thread/streak/download) are
  unaffected.

## Testing

`talk-core::palette` (pure unit tests):
- For `Rust` and `HighContrast`: every `Color` tone's luminance
  (`0.2126r + 0.7152g + 0.0722b`) ≥ `90.0`, and `lum(core) ≥ lum(dim) ≥ lum(edge)`.
- For `Mono`: `core == Tone::Terminal`, `dim == Tone::TerminalFaint`,
  `edge == Tone::TerminalFaint`.
- `Theme::from_str`: `"rust"`, `"high-contrast"`, `"mono"`, `"  Rust "` (trim +
  case-insensitive) resolve; `"bogus"` → `None`.
- `Theme::default() == Theme::Rust`.

Binary:
- Resolution precedence: flag > config > default; an unknown config string yields the
  startup error listing the valid names; an unknown flag is rejected by clap.
- A pure `style_for(tone) -> StyleSpec` helper (color choice + intensity) is unit
  tested so the Tone→crossterm decision is covered without driving a real terminal.

Existing `render_model` tests are unchanged (they assert on `LineKind`/text, not
tones).

## Non-goals

- No terminal-background auto-detection (OSC 11). `mono` covers light terminals via
  the terminal's own foreground.
- No downloadable theme packs — the set is baked into `talk-core`, like meditate's
  seasons.
- No per-tone / arbitrary-RGB user customization. A single accent override could be a
  future addition; it is explicitly out of scope here.

## Follow-up (not blocking)

After the legible `rust` default lands, re-render `demo/talk.gif` from a fresh
loopback capture so the README hero shows the fixed tones (`demo/record.py` →
post-process → `agg`).

## Files

- `crates/talk-core/src/palette.rs` — `Tone`, `Theme`, `palette(theme)`, `from_str`,
  `NAMES`, tests. (~+70 lines)
- `src/render/mod.rs` — `paint(view, &palette)`, `apply_tone`/`style_for`, end-of-frame
  attribute reset.
- `src/cli.rs` — global `--palette` `ValueEnum` flag.
- `src/config.rs` — `palette: Option<String>` + template line.
- `src/main.rs` — resolve `Theme` (precedence + error), pass `Palette` into the
  session.
- `src/live.rs` (and `src/session.rs` if it owns the paint call) — thread the resolved
  `Palette` to `paint`.
