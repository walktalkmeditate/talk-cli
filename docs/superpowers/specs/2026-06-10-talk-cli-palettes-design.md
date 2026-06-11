---
title: talk-cli palettes — legible default + pinnable themes
date: 2026-06-10
status: design
origin: field feedback (rust diff text hard to read on dark terminals)
revised: 2026-06-10 (ce-doc-review round 1 — all findings auto-resolved)
---

# talk palettes — legible default + pinnable themes

**Goal:** Make talk's live transcription legible on a dark terminal out of the box,
and legible on *any* terminal via a pinned palette. Fix the crushed default tones,
lift the question off the dimmest tier, and ship a small baked-in set (`rust`,
`high-contrast`, `mono`) selectable via config + a `--palette` flag.

**Architecture:** Replace the multiplicative tone derivation in `talk-core::palette`
with explicit, contrast-tuned `(core, dim, edge)` triples per named theme. Tones
become a `Tone` enum so `mono` can use the terminal's *own* foreground (matching any
background) and so `NO_COLOR`/`TERM=dumb` degrade cleanly. The binary's renderer maps
`Tone` → crossterm styling. A `--palette` flag + `palette =` config select the theme;
default is `rust`.

**Tech stack:** Rust, `talk-core` (pure), `crossterm` (binary render layer), `clap`.
The `Color` tones assume a truecolor (24-bit) terminal — which talk already requires
today (it emits `Color::Rgb` SGR unconditionally); see *Terminal assumptions*.

---

## Background — the bug

`talk-core::palette::palette()` derives the three tones by multiplying the brand
`RUST = (160, 99, 75)` toward black:

```rust
Palette { core: RUST, dim: scale(RUST, 0.6), edge: scale(RUST, 0.35) }
```

So `edge = (56, 34, 26)` (luminance ~40). In `render_model::compose`, the header,
the question box, the borders, and the hint/status line **all** paint at `edge` — the
dimmest tone — so on a near-black terminal they are effectively invisible.

There are **two** defects, and the fix must address both:
1. **Crushed tones.** Multiplicative dimming pushes `dim`/`edge` toward black.
2. **Importance inversion.** The *question* — the prompt the user is answering — is
   `LineKind::Chrome`, so it paints at the dimmest tier, identical to the borders. The
   most important text on the screen is the faintest.

Confirmed by rendering candidate palettes as real terminal frames on a dark
background (brainstorm preview): `CURRENT` loses the question and status entirely; the
re-derived `rust` keeps everything readable with the hierarchy intact.

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

The existing `RUST` const stays as the brand anchor. Note for the record: the
rendered `rust` default below is intentionally **brighter than the brand swatch**
`(160,99,75)` — the on-screen default diverges from the shared Walk·Talk·Meditate
identity color on purpose, traded for dark-terminal legibility.

### Fixing the importance inversion (the question's tone)

The question text line is reclassified from `Chrome` to a new
`LineKind::Question` in `talk-core::render_model`, mapped to the **`dim`** tone — the
mid level, clearly readable and a step below the user's settled words (the focal
content), but well off the dimmest `edge` tier. The box borders, header, and status
line stay `Chrome → edge`. `Palette` keeps three tones; the new `Question` LineKind
reuses `dim` (no fourth tone).

Final `LineKind → Tone` mapping:

| LineKind | Tone | What it paints |
|----------|------|----------------|
| `Settled` | `core` | the user's finalized words (brightest, focal) |
| `Question` | `dim` | the prompt — readable, a step below settled |
| `Edge` | `dim` | the live streaming partial (provisional) |
| `Chrome` | `edge` | header, box borders, status/hint line (quiet furniture) |

### The three palettes

`core` ≥ `question`/`dim` ≥ `edge` in luminance; every `Color` tone clears the
contrast target in *Testing*. The triples below are **starting values from the
preview**; implementation verifies them against the contrast test and brightens any
that fall short (they may only move *up*).

| Theme | core | dim (live edge + question) | edge |
|-------|------|----------------------------|------|
| `rust` (default) | `Color(206,140,112)` | `Color(158,110,92)` | `Color(140,112,102)` |
| `high-contrast` | `Color(236,205,186)` | `Color(198,152,124)` | `Color(176,142,126)` |
| `mono` | `Terminal` | `TerminalFaint` | `TerminalFaint` |

`rust` and `high-contrast` are tuned for a **dark** terminal background; on a light
or unusually-tinted background their bright tones lose contrast — those users should
pin `mono`. `mono` paints `core` in the terminal's own foreground and `dim`/`edge`
faint, so it matches whatever theme the user already reads comfortably.

**`mono` limitations (documented, not bugs):**
- It collapses `question`/`dim` and `edge` into one faint level — ANSI has a single
  faint attribute. The live edge and chrome look alike under `mono`.
- It is legible only insofar as the user's own terminal theme is. It is **not** a
  guaranteed-high-contrast fallback — for that, `high-contrast` is the tuned option.
- It relies on the terminal honoring ANSI faint (`SGR 2`). Where faint is a no-op
  (e.g. tmux often strips `SGR 2`, some terminals ignore it), `core`, `dim`, and
  `edge` all collapse to the default foreground — a flat single-tone screen. Users on
  such setups should pin `rust`/`high-contrast` instead.

### Renderer: Tone → crossterm

The mapping is split into a **pure, testable** `style_for` and a thin I/O wrapper
`apply_tone` that queues it. `style_for` has no terminal dependency, so the
Tone→style decision is unit-tested without driving a terminal:

```rust
/// (foreground, intensity) for a tone — pure, no I/O. The testable seam.
pub fn style_for(tone: Tone) -> (style::Color, style::Attribute) {
    match tone {
        Tone::Color(c) => (rust(c),               style::Attribute::NormalIntensity),
        Tone::Terminal => (style::Color::Reset,    style::Attribute::NormalIntensity),
        Tone::TerminalFaint => (style::Color::Reset, style::Attribute::Dim),
    }
}

fn apply_tone(out: &mut impl Write, tone: Tone) -> io::Result<()> {
    let (fg, intensity) = style_for(tone);
    queue!(out, style::SetAttribute(intensity), style::SetForegroundColor(fg))
}
```

Intensity is always set explicitly (`NormalIntensity` for non-faint) so a faint line
never bleeds into the next. `paint` resets color **and** attributes at the end of the
frame (`ResetColor` + `SetAttribute(Reset)`); a test asserts the reset runs even on
the early-return/error paths so `Dim` can't leak into the restored terminal.

### Terminal assumptions & `NO_COLOR`

The `Color` tones emit 24-bit `Color::Rgb` SGR (as talk already does today); crossterm
does not downsample to 256/16-color, so 256/16-color rendering is a **non-goal** — the
tuned tones may render approximately on such terminals.

talk honors **`NO_COLOR`** and **`TERM=dumb`**: when either is set, the resolved theme
is forced to `mono` regardless of flag/config, so output uses the terminal's own
foreground (no app-imposed RGB) — matching the precedent in meditate's `caps.rs`. Full
color-depth detection/downsampling is out of scope (see Non-goals).

### Selection surface

- **Flag:** a global `--palette <rust|high-contrast|mono>` on `Cli`, a clap
  `ValueEnum` (`PaletteArg`) bridged to `talk_core::palette::Theme` via
  `From<PaletteArg> for Theme` defined in the binary crate (the orphan-rule-legal
  placement, mirroring meditate's `From<PalettePin> for Pin`). clap validates the
  value and lists choices in `--help`.
- **Config:** `palette: Option<String>` in `Config`, parsed via `Theme::from_str`.
  A new commented line in the `config init` template, noting light-terminal users may
  prefer `mono`.
- **Resolution (precedence: `NO_COLOR`/`TERM=dumb` > flag > config > default).** A
  present-but-invalid config string is a **hard error** at startup — the chain must
  distinguish *absent* from *present-but-invalid*, which `and_then(...).unwrap_or_default()`
  cannot do:

  ```rust
  let theme = if no_color_or_dumb() {
      Theme::Mono
  } else if let Some(arg) = cli.palette {
      Theme::from(arg)                 // ValueEnum → Theme (clap already validated)
  } else if let Some(s) = config.palette.as_deref() {
      Theme::from_str(s).ok_or_else(|| /* startup error, see below */)?
  } else {
      Theme::default()                 // Rust
  };
  ```

  The error text: `unknown palette "<x>"; valid: rust, high-contrast, mono` (fail
  fast, per the error-handling standard). An invalid **flag** is rejected by clap
  before this point. Zero-config launches in `rust` (or `mono` under `NO_COLOR`).

- The resolved `Palette` is computed once before the live loop and passed into
  `paint`. The threading touches `main.rs` (resolution) and `live.rs` (`run_loop`
  builds the `View` and is the **only** caller of `paint` — `session.rs` does not own
  the paint call). `render_model` gains the `Question` `LineKind` but otherwise still
  emits `LineKind` only (no crossterm, stays pure).

### Scope

- Applies to the **live TUI tones only**: header, question box, settled text, live
  edge, status line — across all of its states. The **paused** UI uses the same
  mapping with no tone changes; only the status-line text differs (`paused` vs
  `listening`) and the live-edge line is absent — no new tone is needed. The
  **confirm-cancel** and **ephemeral** chrome are `Chrome → edge` like the rest.
- The close (`compose_close`) and released (`compose_released`) screens already print
  in the terminal's default color and **stay as-is**.
- `--from-text` and non-session commands (config/thread/streak/download) are
  unaffected.

## Testing

`talk-core::palette` (pure unit tests):
- **Contrast, not absolute luminance.** For `Rust` and `HighContrast`, compute the
  WCAG contrast ratio (sRGB-gamma relative luminance) of each `Color` tone against a
  reference dark-terminal background `rgb(40, 44, 52)` (a representative "light" dark
  theme, e.g. One Dark — the worst case among dark terminals for bright tones):
  `core ≥ 4.5:1`, and `question`/`dim` and `edge` `≥ 3:1`. Tones that fail are
  brightened until they pass (the values above are starting points).
- Ordering: `lum(core) ≥ lum(dim) ≥ lum(edge)`.
- For `Mono`: `core == Tone::Terminal`, `dim == Tone::TerminalFaint`,
  `edge == Tone::TerminalFaint`.
- `Theme::from_str`: `"rust"`, `"high-contrast"`, `"mono"`, `"  Rust "` (trim +
  case-insensitive) resolve; `"bogus"` → `None`. `Theme::default() == Theme::Rust`.

Binary:
- `style_for(tone)` returns the expected `(Color, Attribute)` for each variant
  (`Color`→`(Rgb, NormalIntensity)`, `Terminal`→`(Reset, NormalIntensity)`,
  `TerminalFaint`→`(Reset, Dim)`).
- Resolution precedence: `NO_COLOR`/`TERM=dumb` forces `mono` over flag and config;
  flag > config > default; an unknown **config** string yields the startup error
  listing the valid names; an unknown **flag** is rejected by clap.
- `render_model`: the question line is emitted as `LineKind::Question`, not `Chrome`
  (existing render_model tests update accordingly).

## Non-goals

- **No terminal-background auto-detection (OSC 11).** `mono` covers other backgrounds
  via the terminal's own foreground. If bg-aware contrast is adopted later, the
  `Tone`/`palette(theme)` boundary moves from compile-time-fixed triples to a
  bg-parameterized resolver — naming this bet so the later pivot is a known cost, not
  a surprise.
- **No 256/16-color downsampling.** The `Color` tones assume truecolor; `NO_COLOR`/
  `TERM=dumb` route to `mono` rather than approximating.
- **No downloadable theme packs** — the set is baked into `talk-core`, like meditate's
  seasons.
- **No per-tone / arbitrary-RGB user customization.**

## Follow-up (not blocking)

After the legible `rust` default lands, re-render `demo/talk.gif` from a fresh
loopback capture so the README hero shows the fixed tones (`demo/record.py` →
post-process → `agg`).

## Files

- `crates/talk-core/src/palette.rs` — `Tone`, `Theme`, `Palette` (with `question`),
  `palette(theme)`, `from_str`, `NAMES`, contrast/ordering tests. (~+90 lines)
- `crates/talk-core/src/render_model.rs` — add `LineKind::Question`; emit it for the
  question line; update affected tests.
- `src/render/mod.rs` — `paint(view, &palette)`, pure `style_for` + `apply_tone`,
  end-of-frame attribute reset (incl. error-path reset test).
- `src/cli.rs` — global `--palette` `ValueEnum` flag + `From<PaletteArg> for Theme`.
- `src/config.rs` — `palette: Option<String>` + template line.
- `src/main.rs` — `no_color_or_dumb()` check, resolve `Theme` (precedence + hard
  error), pass `Palette` into the session.
- `src/live.rs` — thread the resolved `Palette` to `paint` (the sole call site).
