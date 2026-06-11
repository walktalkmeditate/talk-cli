# talk Palettes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make talk's live transcription legible on a dark terminal out of the box and on any terminal via a pinned palette — fix the crushed tones, lift the question off the dimmest tier, and add a `rust`/`high-contrast`/`mono` set selectable by `--palette` and config.

**Architecture:** `talk-core::palette` gains a pure `Tone`/`Theme` model with explicit per-theme `(core, dim, edge)` triples (verified by a WCAG-contrast test), replacing the multiplicative `scale()` derivation. The binary's `render/mod.rs` maps `Tone` → crossterm via a pure `style_for` seam wrapped by `apply_tone`. A clap `--palette` flag + a `palette =` config string resolve to a `Theme` (with `NO_COLOR`/`TERM=dumb` forcing `mono`), and the resolved `Palette` threads through `LiveConfig` → `run_loop` → `paint`.

**Tech Stack:** Rust, `talk-core` (pure), `crossterm` 0.28 (binary render), `clap` 4.5 (`derive`/`ValueEnum`). Spec: `docs/superpowers/specs/2026-06-10-talk-cli-palettes-design.md`.

**Per-commit invariant:** every task leaves `cargo build`, `cargo test --workspace`, and `cargo clippy --all-targets -- -D warnings` green. Tasks 1–4 keep `paint`'s signature unchanged (it uses `palette(Theme::default())` internally) so nothing downstream breaks; Task 5 threads the real palette through and updates every call site in one commit.

---

## File Structure

- `crates/talk-core/src/palette.rs` — **rewrite.** `Rgb` (unchanged), `RUST` const (kept as the brand anchor), new `Tone` enum, new `Theme` enum (`from_str`, `NAMES`), `Palette { core, dim, edge: Tone }`, `palette(theme)`. Owns the contrast/ordering/parse tests.
- `crates/talk-core/src/render_model.rs` — add `LineKind::Question`; emit it for the question line.
- `src/render/mod.rs` — `style_for` (pure seam) + `apply_tone`; `paint` maps `Tone` and resets attributes at end-of-frame. Already `#![allow(dead_code)]` + ungated.
- `src/cli.rs` — `PaletteArg` (`ValueEnum`), `From<PaletteArg> for Theme`, `--palette` flag, pure `resolve_theme` (listen-gated) + its tests.
- `src/config.rs` — `palette: Option<String>` field + template line + tests.
- `src/main.rs` — read `NO_COLOR`/`TERM`, call `resolve_theme`, build the `Palette`, set it on `LiveConfig`.
- `src/live.rs` — `LiveConfig.palette`; `run_loop` passes it to `paint`.

---

## Task 1: talk-core palette — Tone, Theme, contrast-tuned triples

**Files:**
- Modify: `crates/talk-core/src/palette.rs` (full rewrite of the non-`Rgb` parts)
- Modify: `src/render/mod.rs` (map the new `Tone`; keep `paint(&View)` signature)

- [ ] **Step 1: Write the failing tests** — replace the entire `#[cfg(test)] mod tests` block in `crates/talk-core/src/palette.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG sRGB relative luminance (0.0–1.0).
    fn rel_lum(c: Rgb) -> f64 {
        fn ch(v: u8) -> f64 {
            let s = v as f64 / 255.0;
            if s <= 0.03928 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        }
        0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
    }
    fn contrast(fg: Rgb, bg: Rgb) -> f64 {
        let (a, b) = (rel_lum(fg), rel_lum(bg));
        let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }
    fn rgb_of(t: Tone) -> Rgb {
        match t { Tone::Color(c) => c, _ => panic!("expected a Color tone") }
    }

    /// A representative "light" dark-terminal background (One Dark) — the worst
    /// case among dark terminals for bright foreground tones.
    const DARK_BG: Rgb = Rgb::new(40, 44, 52);

    #[test]
    fn rust_is_the_brand_anchor() {
        assert_eq!(RUST, Rgb::new(160, 99, 75));
    }

    #[test]
    fn color_palettes_clear_the_contrast_targets() {
        for theme in [Theme::Rust, Theme::HighContrast] {
            let p = palette(theme);
            assert!(contrast(rgb_of(p.core), DARK_BG) >= 4.5, "{theme:?} core too low");
            assert!(contrast(rgb_of(p.dim), DARK_BG) >= 3.0, "{theme:?} dim too low");
            assert!(contrast(rgb_of(p.edge), DARK_BG) >= 3.0, "{theme:?} edge too low");
        }
    }

    #[test]
    fn tones_keep_their_brightness_order() {
        for theme in [Theme::Rust, Theme::HighContrast] {
            let p = palette(theme);
            assert!(rel_lum(rgb_of(p.core)) >= rel_lum(rgb_of(p.dim)));
            assert!(rel_lum(rgb_of(p.dim)) >= rel_lum(rgb_of(p.edge)));
        }
    }

    #[test]
    fn mono_uses_the_terminal_foreground() {
        let p = palette(Theme::Mono);
        assert_eq!(p.core, Tone::Terminal);
        assert_eq!(p.dim, Tone::TerminalFaint);
        assert_eq!(p.edge, Tone::TerminalFaint);
    }

    #[test]
    fn theme_from_str_parses_canonical_names() {
        assert_eq!(Theme::from_str("rust"), Some(Theme::Rust));
        assert_eq!(Theme::from_str("high-contrast"), Some(Theme::HighContrast));
        assert_eq!(Theme::from_str("  Mono "), Some(Theme::Mono));
        assert_eq!(Theme::from_str("bogus"), None);
        assert_eq!(Theme::default(), Theme::Rust);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p talk-core palette 2>&1 | head -30`
Expected: FAIL to compile — `Tone`, `Theme`, and `palette(theme)` don't exist yet.

- [ ] **Step 3: Implement** — replace everything **above** the test module in `crates/talk-core/src/palette.rs` with:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb { pub r: u8, pub g: u8, pub b: u8 }

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self { Rgb { r, g, b } }
}

/// The talk pillar brand tone — rust, from pilgrim-ios rust.colorset (light). The
/// rendered `rust` palette is a brighter dark-terminal variant of this anchor.
pub const RUST: Rgb = Rgb::new(160, 99, 75);

/// One paintable tone. `Color` is an explicit RGB; the `Terminal*` variants defer to
/// the terminal's own foreground so `mono` matches any background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Color(Rgb),
    Terminal,
    TerminalFaint,
}

/// A named, pinnable palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Rust,
    HighContrast,
    Mono,
}

impl Theme {
    pub const NAMES: [&'static str; 3] = ["rust", "high-contrast", "mono"];

    /// Parse a config string (trimmed, case-insensitive). `None` for unknown names.
    #[allow(clippy::should_implement_trait)] // inherent parser returns Option, not FromStr's Result
    pub fn from_str(s: &str) -> Option<Theme> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rust" => Some(Theme::Rust),
            "high-contrast" => Some(Theme::HighContrast),
            "mono" => Some(Theme::Mono),
            _ => None,
        }
    }
}

/// The three tones a renderer paints from: `core` = settled text (brightest), `dim` =
/// the live edge and the question (mid), `edge` = borders/header/status (quietest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub core: Tone,
    pub dim: Tone,
    pub edge: Tone,
}

/// The tones for a theme. The `Color` triples are tuned to clear the WCAG contrast
/// targets in the palette tests against a representative dark background; they may be
/// re-tuned for the demo as long as those tests stay green.
pub fn palette(theme: Theme) -> Palette {
    use Tone::{Color, Terminal, TerminalFaint};
    match theme {
        Theme::Rust => Palette {
            core: Color(Rgb::new(210, 146, 118)),
            dim: Color(Rgb::new(170, 124, 104)),
            edge: Color(Rgb::new(150, 122, 112)),
        },
        Theme::HighContrast => Palette {
            core: Color(Rgb::new(236, 205, 186)),
            dim: Color(Rgb::new(198, 152, 124)),
            edge: Color(Rgb::new(176, 142, 126)),
        },
        Theme::Mono => Palette { core: Terminal, dim: TerminalFaint, edge: TerminalFaint },
    }
}
```

- [ ] **Step 4: Update `render/mod.rs` to the new `Tone` model (keep `paint(&View)` signature).** Replace the top `use` lines and the `rust`/`paint` functions in `src/render/mod.rs` with:

```rust
use std::io::{self, Write};
use crossterm::{cursor, execute, queue, style, terminal};
use talk_core::palette::{palette, Rgb, Theme, Tone};
use talk_core::render_model::{compose, LineKind, View};
```

```rust
fn rust(c: Rgb) -> style::Color { style::Color::Rgb { r: c.r, g: c.g, b: c.b } }

/// (foreground, intensity) for a tone — pure, no I/O. The testable seam.
pub fn style_for(tone: Tone) -> (style::Color, style::Attribute) {
    match tone {
        Tone::Color(c) => (rust(c), style::Attribute::NormalIntensity),
        Tone::Terminal => (style::Color::Reset, style::Attribute::NormalIntensity),
        Tone::TerminalFaint => (style::Color::Reset, style::Attribute::Dim),
    }
}

fn apply_tone(out: &mut impl Write, tone: Tone) -> io::Result<()> {
    let (fg, intensity) = style_for(tone);
    queue!(out, style::SetAttribute(intensity), style::SetForegroundColor(fg))
}

/// Paint a full frame. Clears, then writes each composed line in its tone.
pub fn paint(view: &View) -> io::Result<()> {
    let p = palette(Theme::default());
    let mut out = io::stdout();
    queue!(out, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
    for (line, kind) in compose(view) {
        let tone = match kind {
            LineKind::Settled => p.core,
            LineKind::Edge => p.dim,
            LineKind::Chrome => p.edge,
        };
        apply_tone(&mut out, tone)?;
        queue!(out, style::Print(line), cursor::MoveToNextLine(1))?;
    }
    queue!(out, style::ResetColor, style::SetAttribute(style::Attribute::Reset))?;
    out.flush()
}
```

(Leave `Screen` and `paint_plain` as they are.)

- [ ] **Step 5: Run tests + build to verify green**

Run: `cargo test -p talk-core palette && cargo build --features listen 2>&1 | tail -5`
Expected: palette tests PASS; the listen build compiles (`paint` still takes `&View`).

- [ ] **Step 6: Commit**

```bash
git add crates/talk-core/src/palette.rs src/render/mod.rs
git commit -m "feat(palette): Tone/Theme model with contrast-tuned triples"
```

---

## Task 2: render_model — lift the question off the dimmest tone

**Files:**
- Modify: `crates/talk-core/src/render_model.rs` (add `LineKind::Question`; emit it)
- Modify: `src/render/mod.rs` (map `Question → dim`)

- [ ] **Step 1: Write the failing test** — add to the `tests` module in `crates/talk-core/src/render_model.rs`:

```rust
    #[test]
    fn question_line_is_its_own_kind_not_chrome() {
        let s = Settle::new();
        let mut v = base(Mode::Reflect, &s);
        v.question = Some("What am I avoiding?");
        let kind = compose(&v)
            .into_iter()
            .find(|(l, _)| l.contains("What am I avoiding?"))
            .map(|(_, k)| k)
            .expect("question line present");
        assert_eq!(kind, LineKind::Question);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p talk-core render_model::tests::question_line_is_its_own_kind 2>&1 | head -20`
Expected: FAIL — `LineKind::Question` does not exist (compile error).

- [ ] **Step 3: Implement** — in `crates/talk-core/src/render_model.rs`:

Change the `LineKind` enum to add the variant:

```rust
pub enum LineKind { Chrome, Settled, Edge, Question }
```

In `compose`, change the question text line (only the `│  {q}` line — leave the borders as `Chrome`) from `LineKind::Chrome` to `LineKind::Question`:

```rust
        out.push((format!("│  {}", q), LineKind::Question));
```

- [ ] **Step 4: Map `Question → dim` in the renderer** — in `src/render/mod.rs`, add the arm to the `match kind` in `paint`:

```rust
        let tone = match kind {
            LineKind::Settled => p.core,
            LineKind::Question => p.dim,
            LineKind::Edge => p.dim,
            LineKind::Chrome => p.edge,
        };
```

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test -p talk-core && cargo build --features listen 2>&1 | tail -3`
Expected: all talk-core tests PASS (including the existing `reflect_shows_question_box_and_settled_text`); listen build compiles.

- [ ] **Step 6: Commit**

```bash
git add crates/talk-core/src/render_model.rs src/render/mod.rs
git commit -m "feat(palette): question paints at the mid tone, not the dimmest"
```

---

## Task 3: cli — `--palette` flag, bridge, and pure resolution

**Files:**
- Modify: `src/cli.rs` (add `PaletteArg`, `From`, `--palette`, `resolve_theme` + tests)

- [ ] **Step 1: Write the failing tests** — add to `src/cli.rs`:

```rust
#[cfg(all(test, feature = "listen"))]
mod resolve_tests {
    use super::*;
    use talk_core::palette::Theme;

    #[test]
    fn no_color_forces_mono_over_everything() {
        assert_eq!(
            resolve_theme(true, Some(PaletteArg::Rust), Some("high-contrast")).unwrap(),
            Theme::Mono
        );
    }
    #[test]
    fn flag_beats_config() {
        assert_eq!(resolve_theme(false, Some(PaletteArg::Mono), Some("rust")).unwrap(), Theme::Mono);
    }
    #[test]
    fn config_used_when_no_flag() {
        assert_eq!(resolve_theme(false, None, Some("high-contrast")).unwrap(), Theme::HighContrast);
    }
    #[test]
    fn bad_config_is_a_listed_error() {
        let e = resolve_theme(false, None, Some("bogus")).unwrap_err();
        assert!(e.contains("bogus"), "{e}");
        assert!(e.contains("rust") && e.contains("mono"), "{e}");
    }
    #[test]
    fn default_when_unset() {
        assert_eq!(resolve_theme(false, None, None).unwrap(), Theme::Rust);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --features listen -p talk-cli resolve_tests 2>&1 | head -20`
Expected: FAIL to compile — `PaletteArg` and `resolve_theme` don't exist.

- [ ] **Step 3: Implement** — in `src/cli.rs`, add the import and the new items (keep the existing `Cli`/`Command`):

At the top, alongside the existing `use clap::{Parser, Subcommand};`:

```rust
use clap::ValueEnum;
```

Add a `palette` field to the `Cli` struct (global, like `from_text`):

```rust
    /// Color palette: rust (default) · high-contrast · mono (terminal-native).
    #[arg(long, global = true)]
    pub palette: Option<PaletteArg>,
```

Add the value enum, the bridge, and the pure resolver at the bottom of the file:

```rust
/// The `--palette` flag's accepted values. clap renders these kebab-cased:
/// `rust`, `high-contrast`, `mono`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PaletteArg { Rust, HighContrast, Mono }

impl From<PaletteArg> for talk_core::palette::Theme {
    fn from(a: PaletteArg) -> Self {
        use talk_core::palette::Theme;
        match a {
            PaletteArg::Rust => Theme::Rust,
            PaletteArg::HighContrast => Theme::HighContrast,
            PaletteArg::Mono => Theme::Mono,
        }
    }
}

/// Resolve the palette theme. Precedence: NO_COLOR/TERM=dumb (caller passes
/// `no_color`) > `--palette` flag > config string > default (`rust`). A
/// present-but-invalid config string is a hard error (fail fast). Pure: the env
/// read happens at the call site so this stays unit-testable.
#[cfg(feature = "listen")]
pub fn resolve_theme(
    no_color: bool,
    flag: Option<PaletteArg>,
    config: Option<&str>,
) -> Result<talk_core::palette::Theme, String> {
    use talk_core::palette::Theme;
    if no_color {
        return Ok(Theme::Mono);
    }
    if let Some(arg) = flag {
        return Ok(Theme::from(arg));
    }
    match config {
        Some(s) => Theme::from_str(s)
            .ok_or_else(|| format!("unknown palette \"{s}\"; valid: {}", Theme::NAMES.join(", "))),
        None => Ok(Theme::default()),
    }
}
```

- [ ] **Step 4: Run tests + clippy to verify green across feature sets**

Run: `cargo test --features listen -p talk-cli resolve_tests && cargo clippy --all-targets -- -D warnings 2>&1 | tail -5`
Expected: resolve tests PASS; bare clippy clean (`resolve_theme` is `#[cfg(feature = "listen")]`, so it is not dead code in the bare build).

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs
git commit -m "feat(palette): --palette flag, Theme bridge, pure resolve_theme"
```

---

## Task 4: config — `palette` field + template line

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write the failing tests** — add to the `tests` module in `src/config.rs`:

```rust
    #[test]
    fn palette_pin_loads() {
        let c = Config::load("palette = \"mono\"\n").unwrap();
        assert_eq!(c.palette.as_deref(), Some("mono"));
    }

    #[test]
    fn palette_defaults_to_none() {
        assert_eq!(Config::load("").unwrap().palette, None);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p talk-cli config 2>&1 | head -20`
Expected: FAIL to compile — `Config` has no `palette` field.

- [ ] **Step 3: Implement** — in `src/config.rs`:

Add the field to the `Config` struct (after `journal_cleanup`):

```rust
    pub palette: Option<String>,    // rust (default) | high-contrast | mono
```

Add it to `Default`:

```rust
            palette: None,
```

Add a commented line to `commented_template`. The current last line of the format string ends the literal:

```rust
             journal_cleanup = \"{jc}\"       # medium/high: deterministic-only in v1 (LLM enhances light); full LLM rewrite is future work\n",
```

Change its trailing `\n",` to a `\n\` line-continuation and append the palette line (which carries the new closing `\n",`):

```rust
             journal_cleanup = \"{jc}\"       # medium/high: deterministic-only in v1 (LLM enhances light); full LLM rewrite is future work\n\
             # palette = \"rust\"             # rust (default) · high-contrast · mono — pin mono on light terminals\n",
```

Keep the `format!` named args unchanged — the palette line has no `{}` interpolation.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test -p talk-cli config`
Expected: PASS (including the existing `template_is_loadable`, since the new line is a valid commented TOML line).

- [ ] **Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(palette): config palette field + template line"
```

---

## Task 5: wire the resolved palette through to `paint`

**Files:**
- Modify: `src/render/mod.rs` (`paint` takes a `Palette`)
- Modify: `src/live.rs` (`LiveConfig.palette`; `run_loop` passes it)
- Modify: `src/main.rs` (resolve the theme; set `live_cfg.palette`)

- [ ] **Step 1: `paint` takes the palette** — in `src/render/mod.rs`, change the imports and `paint` to accept a `Palette` and drop the internal default:

```rust
use talk_core::palette::{Palette, Rgb, Tone};
```

```rust
pub fn paint(view: &View, palette: Palette) -> io::Result<()> {
    let mut out = io::stdout();
    queue!(out, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
    for (line, kind) in compose(view) {
        let tone = match kind {
            LineKind::Settled => palette.core,
            LineKind::Question => palette.dim,
            LineKind::Edge => palette.dim,
            LineKind::Chrome => palette.edge,
        };
        apply_tone(&mut out, tone)?;
        queue!(out, style::Print(line), cursor::MoveToNextLine(1))?;
    }
    queue!(out, style::ResetColor, style::SetAttribute(style::Attribute::Reset))?;
    out.flush()
}
```

(`Theme` and `palette` fn are no longer used here — the updated `use` line drops them.)

- [ ] **Step 2: `LiveConfig` carries the palette** — in `src/live.rs`, add the field to `LiveConfig` (after `ephemeral`):

```rust
    pub palette: talk_core::palette::Palette,
```

And change the `paint(&v)?` call in `run_loop` to pass it:

```rust
        paint(&v, cfg.palette)?;
```

- [ ] **Step 3: Resolve and set the palette in `main.rs`** — in `src/main.rs`, inside `run_live_session`, immediately **before** the `let live_cfg = live::LiveConfig {` line (around line 344), insert:

```rust
    let no_color = std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false);
    let theme = match cli::resolve_theme(no_color, args.palette, cfg.palette.as_deref()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let palette = talk_core::palette::palette(theme);
```

Then add `palette` to the `LiveConfig` literal:

```rust
    let live_cfg = live::LiveConfig {
        mode: rmode, question, held_label: held_label.as_deref(), cleanup, ephemeral, palette,
    };
```

- [ ] **Step 4: Build, test, and clippy across feature sets**

Run:
```bash
cargo build --features listen 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -5
cargo test --features listen 2>&1 | tail -5
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```
Expected: all compile and pass; clippy clean.

- [ ] **Step 5: Smoke-test the flag**

Run: `cargo run --features listen -- --palette bogus journal --from-text "x" --date 2026-06-10 --time 08:00 2>&1 | head -3`
Expected: clap rejects `bogus` with an error listing `[possible values: rust, high-contrast, mono]` (exit non-zero — proves the `ValueEnum` wiring).

Run: `cargo run --features listen -- --palette mono journal --from-text "a quiet note" --date 2026-06-10 --time 08:00 2>&1 | tail -3`
Expected: runs the deterministic `--from-text` path and writes an entry (the flag parses and is accepted; the TUI palette is exercised in a real session / the demo follow-up).

- [ ] **Step 6: Commit**

```bash
git add src/render/mod.rs src/live.rs src/main.rs
git commit -m "feat(palette): resolve and thread the palette through to paint"
```

---

## Task 6: full verification gate

**Files:** none (verification only)

- [ ] **Step 1: Run the CI gates locally** (mirrors `.github/workflows/ci.yml`)

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo build --features listen && cargo test --features listen
```
Expected: all green.

- [ ] **Step 2: Confirm the no-color path**

Run: `NO_COLOR=1 cargo run --features listen -- --palette rust journal --from-text "x" --date 2026-06-10 --time 08:00 2>&1 | tail -2`
Expected: runs cleanly (resolution forces `mono` regardless of `--palette rust`; `--from-text` doesn't render the TUI, so this just confirms no panic in the resolve path).

- [ ] **Step 3: Note the follow-up** — re-rendering `demo/talk.gif` with the fixed default (`demo/record.py` → post-process → `agg`) is a **separate** task, not part of this plan; the README still points at the existing GIF until then.

---

## Self-Review

**1. Spec coverage** — every spec section maps to a task:
- Tone model + Theme + `from_str`/`NAMES` → Task 1. Question off the dimmest tier → Task 2. Contrast-ratio test (not absolute luminance) → Task 1 test. `style_for`/`apply_tone` + end-of-frame reset → Tasks 1/5. `--palette` flag + `From` bridge + config field + hard-error resolution + `NO_COLOR`/`TERM=dumb`→mono → Tasks 3/4/5. Live-TUI-only scope (close/released untouched) → `paint_plain` left alone (Tasks 1/5). Tests for resolution precedence + bad-config error → Task 3.
- `mono` Dim-reliability and "inherits terminal contrast" are documentation facts in the spec (config template comment in Task 4 steers light terminals to `mono`); no code beyond `mono`'s tones (Task 1).
- Demo re-render is explicitly out of this plan (Task 6 Step 3), matching the spec's "Follow-up (not blocking)."

**2. Placeholder scan** — no TBD/TODO; every code step shows complete code; every run step shows the exact command and expected result. ✓

**3. Type consistency** — `Tone`/`Theme`/`Palette` defined in Task 1 are used verbatim in Tasks 2–5; `palette(theme)`, `Theme::from_str`, `Theme::NAMES`, `Theme::default()`, `PaletteArg`, `From<PaletteArg> for Theme`, `resolve_theme(no_color, flag, config)`, `style_for(tone)`, and `LiveConfig.palette` match across tasks. `paint` is `paint(&View)` in Tasks 1–4 (internal `palette(Theme::default())`) and becomes `paint(&View, Palette)` only in Task 5, which updates its sole call site in the same commit. The contrast triples in Task 1 are the values the Task 1 tests assert against (verified to clear core ≥ 4.5:1, dim/edge ≥ 3:1 vs `rgb(40,44,52)` with margin, preserving `core ≥ dim ≥ edge`). ✓
