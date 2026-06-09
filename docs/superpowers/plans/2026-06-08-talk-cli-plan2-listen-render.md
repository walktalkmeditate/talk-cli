# talk-cli Listen + Render Implementation Plan (Plan 2 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `talk` ears and eyes — capture the microphone, transcribe speech on-device (Moonshine + Silero VAD, settle-on-pause), and render the live "settle" in the terminal — so a real spoken reflection lands in a file.

**Architecture:** A `listen/` façade (cpal mic capture → Silero VAD segmentation → offline Moonshine recognition) implements the existing `TranscriptSource` trait, emitting one `Commit` per detected speech segment (settle-on-pause; no mid-phrase partials — see Design delta). A `render/` layer turns the pure `Settle` model + status into a text frame (pure `compose()`, unit-tested) painted by a thin crossterm flush. A new interactive session loop interleaves transcript events, keypresses, and repaints. Models are fetched once with pinned-SHA-256 integrity. The Plan-1 `--from-text` path and `session::run` stay for tests.

**Tech Stack:** Rust 1.82; `cpal` (mic input); `sherpa-onnx` (k2-fsa Rust crate — `OfflineRecognizer` + Moonshine, `VoiceActivityDetector` + Silero VAD, `Wave`); `crossterm` (already used pattern in meditate-cli); `ureq` (model download, behind the `download` feature); `sha2` (integrity). Candle is **not** in Plan 2 (the formatter + its latency spike are Plan 3).

**Origin spec:** `docs/superpowers/specs/2026-06-08-talk-cli-design.md` (settle model amended for settle-on-pause).

---

## Design delta from the spec (read first)

Research against k2-fsa's `sherpa-onnx` Rust crate (`rust-api-examples/moonshine_v2.rs`, `streaming_zipformer_microphone.rs`, the `*_simulate_streaming_microphone` examples) established:

1. **Moonshine is offline (non-streaming) in the Rust toolchain.** So Plan 2 uses **settle-on-pause**: Silero VAD detects a speech segment; on your pause, offline Moonshine transcribes that segment and the clean phrase settles in at once. The dim "live edge" is a calm **listening indicator**, not jittering partials. (Spec §7 amended.) A streaming-Zipformer "live jitter" mode is a deliberate later option, not in Plan 2.
2. **The `talk-core::settle::Settle` machine is unchanged.** The listen façade calls `commit(raw, clean)` on each VAD endpoint; `on_partial` is used only to drive the listening indicator. `Live`/`Committing`/`Settled` and never-re-flow still hold.
3. **No LLM in Plan 2.** Cleanup stays deterministic-Light (already in `talk-core`). The 0.5B formatter and its **latency spike move to Plan 3** (they gate the formatter, not the STT engine, which is now settled).
4. **Execution venue is split.** The render/keymap/paths tasks (T1–T5, T11) are pure or crossterm-thin and are **built + tested in CI here**. The mic/VAD/STT/download/live-session tasks (T6–T10, T12) need **a real microphone, the model files, and network** — their code is written against the cited authoritative examples and is **verified on your machine** (each such task says so and gives the exact manual check).

---

## File structure

```
talk-cli/
  Cargo.toml                       # + cpal, sherpa-onnx (feature "listen"), sha2; ureq already behind "download"
  crates/talk-core/src/
    palette.rs                     # restore palette() synthesis (deferred from Plan 1)
    render_model.rs                # NEW: pure View model + compose() → Vec<String> (no I/O) [here-testable]
  src/
    paths.rs                       # + TALK_BASE_DIR override + models_dir()
    keymap.rs                      # NEW: pure KeyEvent → Action mapping [here-testable]
    render/
      mod.rs                       # crossterm paint(view) + raw-mode guard (thin over compose())
    listen/
      mod.rs                       # LiveSource: capture+VAD+STT implementing TranscriptSource [machine]
      capture.rs                   # cpal mic → mpsc<Vec<f32>> [machine]
      vad.rs                       # Silero VAD segmentation [machine]
      stt.rs                       # Moonshine offline recognizer [machine]
    download/
      mod.rs                       # model fetch + SHA-256 verify gate (behind "download") [network]
      models.rs                    # pinned model manifest (urls + sha256)
    live.rs                        # NEW: interactive session loop (events + keys + repaint) [machine]
    main.rs                        # wire: real mic session by default; --from-text keeps Plan-1 path
```

---

## Task 1: Restore palette() synthesis (deferred from Plan 1)

**Files:**
- Modify: `crates/talk-core/src/palette.rs`

Plan 1 trimmed palette to the `RUST` const. The renderer needs the derived tones. Restore the synthesis (the render layer reads `core`/`edge`/`dim`).

- [ ] **Step 1: Write the failing test**

Replace the contents of `crates/talk-core/src/palette.rs` with:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb { pub r: u8, pub g: u8, pub b: u8 }

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self { Rgb { r, g, b } }
}

/// The talk pillar base tone — `rust`, from pilgrim-ios rust.colorset (light).
pub const RUST: Rgb = Rgb::new(160, 99, 75);

/// The three tones the renderer paints from: `core` = settled text (bright),
/// `dim` = the live/listening edge, `edge` = borders/hints (dimmest).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub core: Rgb,
    pub dim: Rgb,
    pub edge: Rgb,
}

pub fn palette() -> Palette {
    Palette { core: RUST, dim: scale(RUST, 0.6), edge: scale(RUST, 0.35) }
}

fn scale(c: Rgb, f: f32) -> Rgb {
    Rgb::new((c.r as f32 * f) as u8, (c.g as f32 * f) as u8, (c.b as f32 * f) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_is_the_talk_base_tone() {
        assert_eq!(RUST, Rgb::new(160, 99, 75));
    }

    #[test]
    fn dim_is_darker_than_core_and_edge_darkest() {
        let p = palette();
        assert!(p.dim.r < p.core.r && p.edge.r < p.dim.r);
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core palette`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/palette.rs && git commit -m "feat(core): restore palette() synthesis for the renderer"
```

---

## Task 2: Render view model + pure compose() [here-testable]

**Files:**
- Create: `crates/talk-core/src/render_model.rs`
- Modify: `crates/talk-core/src/lib.rs` (add `pub mod render_model;`)

The renderable state is a pure `View`; `compose()` turns it into the lines to paint. Keeping it pure (no crossterm) makes the entire screen layout unit-testable. The crossterm flush (Task 5) is a thin wrapper.

- [ ] **Step 1: Write the failing test**

Create `crates/talk-core/src/render_model.rs`:

```rust
use crate::settle::Settle;

/// Which mode's chrome to show.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode { Reflect, Journal, Ephemeral }

/// Everything the screen needs, with no I/O. The live session rebuilds this each
/// frame from the Settle machine + clock + listening flag.
pub struct View<'a> {
    pub mode: Mode,
    pub question: Option<&'a str>, // reflect only
    pub held_label: Option<&'a str>, // e.g. "held 3 days"; None hides the box line
    pub settle: &'a Settle,
    pub listening: bool,           // VAD currently hears speech
    pub elapsed: &'a str,          // "2:14"
    pub cleanup: &'a str,          // "Light"
    pub show_raw: bool,            // `u` toggle: show raw verbatim instead of clean
}

/// Compose the full screen as lines (top to bottom). Pure — unit-testable.
pub fn compose(v: &View) -> Vec<String> {
    let mut out = Vec::new();
    out.push(header_line(v));
    out.push(String::new());

    if let (Mode::Reflect, Some(q)) = (v.mode, v.question) {
        if let Some(h) = v.held_label {
            out.push(format!("┌─ {} ", h) + &"─".repeat(60));
        } else {
            out.push("┌".to_string() + &"─".repeat(64));
        }
        out.push(format!("│  {}", q));
        out.push("└".to_string() + &"─".repeat(64));
        out.push(String::new());
    }
    if v.mode == Mode::Ephemeral {
        out.push("Say it. This keeps nothing.".to_string());
        out.push(String::new());
    }

    // Settled blocks (bright/locked), then the committing block, then the edge.
    for b in v.settle.settled() {
        out.push(if v.show_raw { b.raw.clone() } else { b.clean.clone() });
    }
    if let Some(c) = v.settle.committing() {
        out.push(if v.show_raw { c.raw.clone() } else { c.clean.clone() });
    }
    out.push(String::new());
    out.push(edge_line(v));
    out.push("─".repeat(66));
    out.push(status_line(v));
    out
}

fn header_line(v: &View) -> String {
    let label = match v.mode {
        Mode::Reflect => "talk · reflect",
        Mode::Journal => "talk · journal",
        Mode::Ephemeral => "talk · unburden",
    };
    let privacy = if v.mode == Mode::Ephemeral {
        "● local · no network · ✦ nothing saved"
    } else {
        "● local · no network"
    };
    format!("{}{}{}", label, " ".repeat(privacy_gap(label, privacy)), privacy)
}

fn privacy_gap(label: &str, privacy: &str) -> usize {
    66usize.saturating_sub(label.chars().count() + privacy.chars().count()).max(2)
}

fn edge_line(v: &View) -> String {
    // Settle-on-pause: the live edge is a listening indicator, not partials.
    if v.listening { "  …".to_string() } else { String::new() }
}

fn status_line(v: &View) -> String {
    let dot = if v.listening { "● listening" } else { "○ ready" };
    match v.mode {
        Mode::Ephemeral => format!("{}  {}   ✦ ephemeral    [space] release · esc cancel", dot, v.elapsed),
        _ => format!(
            "{}  {}   {}    [space] done · u raw⇄clean · p pause · esc cancel",
            dot, v.elapsed, v.cleanup
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settle::Settle;

    fn settled_one() -> Settle {
        let mut s = Settle::new();
        s.commit("um the raw words", "The clean words.");
        s.finalize();
        s
    }

    #[test]
    fn reflect_shows_question_box_and_settled_text() {
        let s = settled_one();
        let v = View {
            mode: Mode::Reflect, question: Some("What am I avoiding?"),
            held_label: Some("held 3 days"), settle: &s, listening: false,
            elapsed: "2:14", cleanup: "Light", show_raw: false,
        };
        let lines = compose(&v);
        let joined = lines.join("\n");
        assert!(joined.contains("talk · reflect") && joined.contains("● local · no network"));
        assert!(joined.contains("What am I avoiding?"));
        assert!(joined.contains("held 3 days"));
        assert!(joined.contains("The clean words."));
        assert!(joined.contains("[space] done"));
    }

    #[test]
    fn raw_toggle_shows_verbatim() {
        let s = settled_one();
        let v = View {
            mode: Mode::Reflect, question: Some("Q?"), held_label: None,
            settle: &s, listening: false, elapsed: "0:05", cleanup: "Light", show_raw: true,
        };
        let joined = compose(&v).join("\n");
        assert!(joined.contains("um the raw words"));
        assert!(!joined.contains("The clean words."));
    }

    #[test]
    fn ephemeral_shows_keeps_nothing_chrome() {
        let s = Settle::new();
        let v = View {
            mode: Mode::Ephemeral, question: None, held_label: None, settle: &s,
            listening: true, elapsed: "0:48", cleanup: "Light", show_raw: false,
        };
        let joined = compose(&v).join("\n");
        assert!(joined.contains("✦ nothing saved"));
        assert!(joined.contains("Say it. This keeps nothing."));
        assert!(joined.contains("[space] release"));
    }

    #[test]
    fn listening_flag_drives_the_indicator() {
        let s = Settle::new();
        let mk = |listening| {
            let v = View { mode: Mode::Journal, question: None, held_label: None, settle: &s,
                listening, elapsed: "0:01", cleanup: "Medium", show_raw: false };
            compose(&v).join("\n")
        };
        assert!(mk(true).contains("● listening"));
        assert!(mk(false).contains("○ ready"));
    }
}
```

Add to `crates/talk-core/src/lib.rs`: `pub mod render_model;`

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core render_model`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/render_model.rs crates/talk-core/src/lib.rs
git commit -m "feat(core): pure render view model + compose()"
```

---

## Task 3: Close + released frames [here-testable]

**Files:**
- Modify: `crates/talk-core/src/render_model.rs`

The end-of-session frames: reflect/journal "close" (path + provenance + curated phrase) and ephemeral "released".

- [ ] **Step 1: Write the failing test**

Add to `render_model.rs` (above the `#[cfg(test)]` block):

```rust
/// The closing screen after `[space]` in reflect/journal.
pub fn compose_close(path: &str, provenance: &str, phrase: &str) -> Vec<String> {
    vec![
        format!("  → {}     {}", path, provenance),
        format!("  \"{}\"", phrase),
    ]
}

/// The ephemeral release screen.
pub fn compose_released() -> Vec<String> {
    vec!["Released. Nothing was written.".to_string()]
}
```

Add to the tests module:

```rust
    #[test]
    fn close_frame_shows_path_and_phrase() {
        let lines = compose_close("~/talk/what-am-i-avoiding.md", "entry 3 · held 3 days", "Stillness carries forward.");
        let joined = lines.join("\n");
        assert!(joined.contains("→ ~/talk/what-am-i-avoiding.md"));
        assert!(joined.contains("entry 3 · held 3 days"));
        assert!(joined.contains("Stillness carries forward."));
    }

    #[test]
    fn released_frame_is_the_keeps_nothing_line() {
        assert_eq!(compose_released(), vec!["Released. Nothing was written.".to_string()]);
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p talk-core render_model`
Expected: PASS (6 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/talk-core/src/render_model.rs && git commit -m "feat(core): close + released render frames"
```

---

## Task 4: In-session keymap (pure) [here-testable]

**Files:**
- Create: `src/keymap.rs`
- Modify: `src/main.rs` (add `mod keymap;`)

Map a crossterm `KeyEvent` to an `Action`, purely, so the live loop's input handling is unit-tested. (Mirrors meditate-cli's keymap pattern.)

- [ ] **Step 1: Write the failing test**

Create `src/keymap.rs`:

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Finish,       // space — save & close (release, in ephemeral)
    ToggleRaw,    // u
    TogglePause,  // p
    Cancel,       // esc / ctrl-c — discard
    None,
}

pub fn action_for(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char(' ') => Action::Finish,
        KeyCode::Char('u') => Action::ToggleRaw,
        KeyCode::Char('p') => Action::TogglePause,
        KeyCode::Esc => Action::Cancel,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Cancel,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn k(c: KeyCode) -> KeyEvent { KeyEvent::new(c, KeyModifiers::NONE) }

    #[test]
    fn maps_the_core_keys() {
        assert_eq!(action_for(k(KeyCode::Char(' '))), Action::Finish);
        assert_eq!(action_for(k(KeyCode::Char('u'))), Action::ToggleRaw);
        assert_eq!(action_for(k(KeyCode::Char('p'))), Action::TogglePause);
        assert_eq!(action_for(k(KeyCode::Esc)), Action::Cancel);
        assert_eq!(action_for(k(KeyCode::Char('x'))), Action::None);
    }

    #[test]
    fn ctrl_c_cancels() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(action_for(ctrl_c), Action::Cancel);
        // plain 'c' is not cancel
        assert_eq!(action_for(k(KeyCode::Char('c'))), Action::None);
    }
}
```

Add `mod keymap;` to `src/main.rs` (with the other `mod` lines).

> Note: `crossterm` is currently an indirect concern; add it as a direct dependency of the `talk` binary in Task 6's Cargo.toml step if not already present. For this task, add `crossterm = "0.28"` to `[dependencies]` in the root `Cargo.toml` now so `keymap.rs` compiles (it's needed by render too).

- [ ] **Step 2: Run the tests**

Run: `cargo test keymap`
Expected: PASS (2 tests).

- [ ] **Step 3: Commit**

```bash
git add src/keymap.rs src/main.rs Cargo.toml Cargo.lock
git commit -m "feat: in-session keymap (pure KeyEvent → Action)"
```

---

## Task 5: Crossterm paint (thin flush over compose) [here-testable build, visual check on your machine]

**Files:**
- Create: `src/render/mod.rs`
- Modify: `src/main.rs` (add `mod render;`)

A thin renderer: enter raw mode + alternate screen (RAII guard, like meditate-cli's `TerminalGuard`), and a `paint()` that clears and writes the `compose()` lines with the rust palette. No business logic — all layout is in `compose()`.

- [ ] **Step 1: Write the implementation**

Create `src/render/mod.rs`:

```rust
use std::io::{self, Write};
use crossterm::{cursor, execute, queue, style, terminal};
use talk_core::palette::{palette, Rgb};
use talk_core::render_model::{compose, View};

/// RAII terminal guard — restores the terminal on drop (incl. on panic), exactly
/// like meditate-cli's session guard.
pub struct Screen;

impl Screen {
    pub fn enter() -> io::Result<Screen> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(Screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

fn rust(c: Rgb) -> style::Color { style::Color::Rgb { r: c.r, g: c.g, b: c.b } }

/// Paint a full frame. Clears, then writes each composed line in the rust tone.
pub fn paint(view: &View) -> io::Result<()> {
    let p = palette();
    let mut out = io::stdout();
    queue!(out, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
    for line in compose(view) {
        queue!(out, style::SetForegroundColor(rust(p.core)), style::Print(line), cursor::MoveToNextLine(1))?;
    }
    queue!(out, style::ResetColor)?;
    out.flush()
}

/// Paint plain lines (used for the close / released screens after the loop ends).
pub fn paint_plain(lines: &[String]) -> io::Result<()> {
    let mut out = io::stdout();
    queue!(out, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
    for line in lines {
        queue!(out, style::Print(line), cursor::MoveToNextLine(1))?;
    }
    out.flush()
}
```

Add `mod render;` to `src/main.rs`.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: clean compile (no warnings). There are no unit tests for the crossterm I/O itself — its logic lives in `compose()` (Task 2/3, already tested). A visual check happens in Task 12 on your machine.

- [ ] **Step 3: Commit**

```bash
git add src/render/mod.rs src/main.rs
git commit -m "feat: crossterm paint + RAII screen guard (thin over compose)"
```

---

## Task 6: cpal microphone capture [needs your machine — a microphone]

**Files:**
- Modify: `Cargo.toml` (add cpal + sherpa-onnx behind a `listen` feature; sha2)
- Create: `src/listen/capture.rs`
- Modify: `src/main.rs` (add `mod listen;` gated on the `listen` feature) and create `src/listen/mod.rs`

Capture mono f32 samples from the default input device into an `mpsc` channel. Modeled exactly on k2-fsa `rust-api-examples/streaming_zipformer_microphone.rs`'s `build_input_stream` (verify method names against that file — it is the authoritative source for the `cpal` + sherpa-onnx versions this plan pins).

- [ ] **Step 1: Cargo.toml — add deps behind a `listen` feature**

Add to `[features]`:
```toml
listen = ["dep:cpal", "dep:sherpa-onnx"]
```
Add to `[dependencies]`:
```toml
cpal = { version = "0.15", optional = true }
sherpa-onnx = { version = "0.x", optional = true }   # pin to the latest k2-fsa release at implementation time; see rust-api-examples
sha2 = "0.10"
```
On macOS, cpal needs the same `coreaudio-sys` MSRV pin meditate uses:
```toml
[target.'cfg(target_os = "macos")'.dependencies]
coreaudio-sys = { version = "=0.2.16", optional = true }
```
(Add `coreaudio-sys` to the `listen` feature's dep list too.)

> **Pin step (do this first, on your machine):** check the current `sherpa-onnx` crate version on crates.io and the matching `rust-api-examples` tag; pin both `sherpa-onnx` and the model URLs (Task 9) to that release so the API and model formats agree. Record the exact version in a comment.

- [ ] **Step 2: Implement capture**

Create `src/listen/capture.rs`:

```rust
use std::sync::mpsc::{self, Receiver, Sender};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct Capture {
    _stream: cpal::Stream,
    pub samples: Receiver<Vec<f32>>,
    pub sample_rate: u32,
}

impl Capture {
    /// Open the default input device and stream mono f32 chunks to the channel.
    pub fn start() -> Result<Capture, String> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or("no input device")?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let sample_rate = supported.sample_rate().0;
        let channels = supported.config().channels as usize;
        let (tx, rx): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = mpsc::channel();

        let err_fn = |e| eprintln!("audio stream error: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &supported.config(),
                move |data: &[f32], _: &_| { let _ = tx.send(downmix(data, channels)); },
                err_fn, None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &supported.config(),
                move |data: &[i16], _: &_| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                    let _ = tx.send(downmix(&f, channels));
                },
                err_fn, None,
            ),
            other => return Err(format!("unsupported sample format: {other:?}")),
        }.map_err(|e| e.to_string())?;

        stream.play().map_err(|e| e.to_string())?;
        Ok(Capture { _stream: stream, samples: rx, sample_rate })
    }
}

/// Average interleaved channels down to mono.
fn downmix(data: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 { return data.to_vec(); }
    data.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::downmix;

    #[test]
    fn downmix_averages_stereo_to_mono() {
        assert_eq!(downmix(&[0.0, 1.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(downmix(&[0.2, 0.4], 1), vec![0.2, 0.4]);
    }
}
```

Create `src/listen/mod.rs` with `pub mod capture;` (more added in T7–T8). Add to `src/main.rs`: `#[cfg(feature = "listen")] mod listen;`

- [ ] **Step 3: Build with the feature; unit-test the pure helper**

Run: `cargo test --features listen capture`
Expected: PASS (1 test: `downmix_averages_stereo_to_mono`).

- [ ] **Step 4: Manual mic check (your machine)**

Add a temporary `examples/mic_probe.rs` that calls `Capture::start()` and prints the peak amplitude of the first ~2s of chunks; run `cargo run --features listen --example mic_probe`, speak, and confirm the peak rises. Delete the example after. (Document the observed sample_rate.)

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/listen/ src/main.rs
git commit -m "feat(listen): cpal microphone capture → mono f32 channel"
```

---

## Task 7: Silero VAD segmentation [needs your machine — model + audio]

**Files:**
- Create: `src/listen/vad.rs`
- Modify: `src/listen/mod.rs` (`pub mod vad;`)

Wrap sherpa-onnx's Silero VAD to turn a stream of f32 chunks into completed **speech segments** (settle-on-pause = one segment per utterance). The exact `VoiceActivityDetector` API (config fields, `accept_waveform`, `is_speech` / `front` / `pop` / `flush`) must be **read from the cited example** at pin time — k2-fsa `rust-api-examples/<the *_simulate_streaming_microphone>.rs` is authoritative for the pinned version. The shape below matches those examples.

- [ ] **Step 1: Implement the VAD wrapper**

Create `src/listen/vad.rs`:

```rust
use sherpa_onnx::{Vad, VadConfig, SileroVadConfig};

/// A completed speech segment (mono f32 @ the model's sample rate, 16k).
pub struct Segment {
    pub samples: Vec<f32>,
    pub sample_rate: i32,
}

pub struct Segmenter {
    vad: Vad,
    sample_rate: i32,
}

impl Segmenter {
    /// `model` = path to silero_vad.onnx. 16 kHz mono is required; the caller
    /// resamples capture to 16k before feeding (see LiveSource in T8).
    pub fn new(model: &str) -> Result<Segmenter, String> {
        let mut cfg = VadConfig::default();
        cfg.silero_vad = SileroVadConfig {
            model: model.to_string(),
            threshold: 0.5,
            min_silence_duration: 0.5, // settle-on-pause: ~half-second pause ends a phrase
            min_speech_duration: 0.25,
            ..Default::default()
        };
        cfg.sample_rate = 16_000;
        let vad = Vad::new(&cfg, 30.0).map_err(|e| e.to_string())?; // 30s buffer
        Ok(Segmenter { vad, sample_rate: 16_000 })
    }

    /// Feed a chunk; returns any segments that completed (speaker paused).
    pub fn push(&mut self, chunk: &[f32]) -> Vec<Segment> {
        self.vad.accept_waveform(chunk);
        let mut done = Vec::new();
        while !self.vad.is_empty() {
            let seg = self.vad.front();
            done.push(Segment { samples: seg.samples.clone(), sample_rate: self.sample_rate });
            self.vad.pop();
        }
        done
    }

    /// True while the VAD currently hears speech (drives the listening indicator).
    pub fn is_speaking(&self) -> bool { self.vad.is_speech() }

    /// On finish, flush any in-progress segment.
    pub fn flush(&mut self) -> Vec<Segment> {
        self.vad.flush();
        self.push(&[])
    }
}
```

> The method names (`Vad::new`, `accept_waveform`, `is_empty`, `front`, `pop`, `is_speech`, `flush`, and the `VadConfig`/`SileroVadConfig` fields) are the documented shape; **confirm them against the pinned `sherpa-onnx` docs.rs + the example** in Step 2 and adjust to the exact symbols (the binding occasionally renames `Vad` ↔ `VoiceActivityDetector`). This is the one task where the persona must read the upstream example before writing.

- [ ] **Step 2: Verify against the example + a recorded wav (your machine)**

Download `silero_vad.onnx` (Task 9 manifest). Add a temporary `examples/vad_probe.rs` that loads a wav of you saying two sentences with a pause, feeds it through `Segmenter`, and prints the number of segments + each duration. Run `cargo run --features listen --example vad_probe`. Expect 2 segments. Adjust `min_silence_duration` if it over/under-splits. Delete the example.

- [ ] **Step 3: Commit**

```bash
git add src/listen/vad.rs src/listen/mod.rs
git commit -m "feat(listen): Silero VAD segmentation (settle-on-pause)"
```

---

## Task 8: Moonshine offline STT + LiveSource [needs your machine — model + mic]

**Files:**
- Create: `src/listen/stt.rs`
- Modify: `src/listen/mod.rs` (`pub mod stt;` + the `LiveSource`)

`stt.rs` wraps offline Moonshine (from `moonshine_v2.rs`). `LiveSource` ties capture → resample-to-16k → VAD → STT and implements `TranscriptSource`, emitting `Event::Commit(text)` per segment and `Event::Partial("")` to pulse the listening indicator. `Event::Done` is produced by the live loop (T12) on `[space]`, not here.

- [ ] **Step 1: Implement the Moonshine recognizer**

Create `src/listen/stt.rs`:

```rust
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

pub struct Stt {
    recognizer: OfflineRecognizer,
}

impl Stt {
    /// Paths from the unpacked Moonshine tiny model dir.
    pub fn new(encoder: &str, decoder: &str, tokens: &str) -> Result<Stt, String> {
        let mut cfg = OfflineRecognizerConfig::default();
        cfg.model_config.moonshine.encoder = Some(encoder.to_string());
        cfg.model_config.moonshine.merged_decoder = Some(decoder.to_string());
        cfg.model_config.tokens = Some(tokens.to_string());
        cfg.model_config.provider = Some("cpu".to_string());
        cfg.model_config.num_threads = 2;
        let recognizer = OfflineRecognizer::create(&cfg).map_err(|e| e.to_string())?;
        Ok(Stt { recognizer })
    }

    /// Transcribe one VAD segment (16 kHz mono).
    pub fn transcribe(&self, samples: &[f32], sample_rate: i32) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(sample_rate, samples);
        self.recognizer.decode(&stream);
        stream.get_result().map(|r| r.text).unwrap_or_default()
    }
}
```
(Confirm `model_config.moonshine.{encoder,merged_decoder}`, `tokens`, `create_stream`, `accept_waveform`, `decode`, `get_result().text` against `moonshine_v2.rs` at the pinned version.)

- [ ] **Step 2: Implement LiveSource**

Add to `src/listen/mod.rs`:

```rust
pub mod capture;
pub mod stt;
pub mod vad;

use crate::source::{Event, TranscriptSource};
use capture::Capture;
use stt::Stt;
use vad::Segmenter;

/// Live mic → VAD → Moonshine, as a TranscriptSource. Non-blocking `next()`:
/// returns Commit when a segment completed, Partial("") while speech is heard,
/// or None when no new audio is ready (the loop polls keys in between).
pub struct LiveSource {
    capture: Capture,
    seg: Segmenter,
    stt: Stt,
    pending: std::collections::VecDeque<Event>,
    finished: bool,
}

impl LiveSource {
    pub fn new(capture: Capture, seg: Segmenter, stt: Stt) -> Self {
        Self { capture, seg, stt, pending: Default::default(), finished: false }
    }

    /// Mark the turn finished (called by the loop on `[space]`): flush the VAD,
    /// transcribe any trailing segment, then a final Done.
    pub fn finish(&mut self) {
        for s in self.seg.flush() {
            let text = self.stt.transcribe(&s.samples, s.sample_rate);
            if !text.trim().is_empty() { self.pending.push_back(Event::Commit(text)); }
        }
        self.pending.push_back(Event::Done);
        self.finished = true;
    }

    /// Drain available audio into VAD segments → transcripts. Call each loop tick.
    fn pump(&mut self) {
        while let Ok(chunk) = self.capture.samples.try_recv() {
            let resampled = resample_to_16k(&chunk, self.capture.sample_rate);
            for s in self.seg.push(&resampled) {
                let text = self.stt.transcribe(&s.samples, s.sample_rate);
                if !text.trim().is_empty() { self.pending.push_back(Event::Commit(text)); }
            }
        }
        if !self.finished && self.seg.is_speaking() {
            self.pending.push_back(Event::Partial(String::new()));
        }
    }
}

impl TranscriptSource for LiveSource {
    fn next(&mut self) -> Option<Event> {
        if self.pending.is_empty() { self.pump(); }
        self.pending.pop_front()
    }
}

/// Linear resample to 16 kHz (good enough for VAD/STT; replace with a higher-
/// quality resampler if WER suffers — note it in a comment if you do).
fn resample_to_16k(input: &[f32], from_hz: u32) -> Vec<f32> {
    if from_hz == 16_000 || input.is_empty() { return input.to_vec(); }
    let ratio = 16_000.0 / from_hz as f32;
    let out_len = (input.len() as f32 * ratio) as usize;
    (0..out_len).map(|i| {
        let src = i as f32 / ratio;
        let lo = src.floor() as usize;
        let hi = (lo + 1).min(input.len() - 1);
        let frac = src - lo as f32;
        input[lo] * (1.0 - frac) + input[hi] * frac
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::resample_to_16k;

    #[test]
    fn resample_is_identity_at_16k() {
        let s = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_to_16k(&s, 16_000), s);
    }

    #[test]
    fn resample_downsamples_length_proportionally() {
        let s = vec![0.0; 48_000]; // 1s @ 48k → ~16k samples
        let out = resample_to_16k(&s, 48_000);
        assert!((out.len() as i32 - 16_000).abs() < 10);
    }
}
```

- [ ] **Step 3: Test the pure resampler; integration-verify on your machine**

Run: `cargo test --features listen listen::tests` (the resample tests). Expected: PASS (2).
On your machine, the full mic→VAD→STT path is exercised by Task 12's end-to-end.

- [ ] **Step 4: Commit**

```bash
git add src/listen/ && git commit -m "feat(listen): Moonshine STT + LiveSource (mic → VAD → transcript)"
```

---

## Task 9: Model download + SHA-256 integrity gate [network]

**Files:**
- Create: `src/download/models.rs` (pinned manifest)
- Create: `src/download/mod.rs`
- Modify: `Cargo.toml` (`download` feature already exists for `ureq`; add `sha2` use), `src/main.rs` (`talk download models` + load paths)
- Modify: `src/paths.rs` (`models_dir()`)

Per spec §11: models fetched once, **pinned SHA-256 verified before load**, then offline forever.

- [ ] **Step 1: paths::models_dir()**

Add to `src/paths.rs`:

```rust
/// Where downloaded models live (separate from journal entries). Honors
/// TALK_BASE_DIR's sibling cache, else the platform cache dir, else ~/.talk/models.
pub fn models_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("TALK_MODELS_DIR") {
        return PathBuf::from(custom);
    }
    directories::ProjectDirs::from("org", "walktalkmeditate", "talk")
        .map(|d| d.cache_dir().join("models"))
        .unwrap_or_else(|| base_dir(None).join("models"))
}
```

- [ ] **Step 2: Pinned manifest**

Create `src/download/models.rs`:

```rust
/// One downloadable artifact with a pinned hash. Fill `sha256` from the actual
/// release asset (run `shasum -a 256 <file>` after a manual download) at pin time.
pub struct Artifact {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
}

/// Plan-2 models: Moonshine tiny (en, int8) + Silero VAD. URLs are k2-fsa
/// release assets; HASHES MUST be filled in at pin time (they are release-stable).
pub const MODELS: &[Artifact] = &[
    Artifact {
        name: "sherpa-onnx-moonshine-tiny-en-int8.tar.bz2",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-moonshine-tiny-en-int8.tar.bz2",
        sha256: "FILL_AT_PIN_TIME",
    },
    Artifact {
        name: "silero_vad.onnx",
        url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx",
        sha256: "FILL_AT_PIN_TIME",
    },
];
```

> The `FILL_AT_PIN_TIME` values are the one allowed deferral in this plan: a hash can only be pinned by hashing the real asset on your machine. The code that *consumes* them (verify gate) is complete; pinning is a one-command step in Step 5.

- [ ] **Step 3: Download + verify**

Create `src/download/mod.rs`:

```rust
pub mod models;

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use models::Artifact;

/// Fetch `art` to `dir` over HTTPS, verifying its pinned SHA-256 before keeping it.
/// A mismatch deletes the bad file and errors — never load an unverified model.
pub fn fetch(art: &Artifact, dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let dest = dir.join(art.name);
    if dest.exists() && verify(&dest, art.sha256).unwrap_or(false) {
        return Ok(()); // already present + valid
    }
    if !art.url.starts_with("https://") {
        return Err(format!("refusing non-HTTPS url: {}", art.url));
    }
    let resp = ureq::get(art.url).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let got = hex(Sha256::digest(&bytes));
    if got != art.sha256 {
        return Err(format!("checksum mismatch for {}: got {got}, want {}", art.name, art.sha256));
    }
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
    Ok(())
}

/// Verify an existing file's SHA-256 (used before loading a cached model).
pub fn verify(path: &Path, want: &str) -> Result<bool, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(hex(Sha256::digest(&bytes)) == want)
}

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_matches_known_hash() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.bin");
        std::fs::write(&f, b"hello").unwrap();
        // sha256("hello")
        let want = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify(&f, want).unwrap());
        assert!(!verify(&f, "deadbeef").unwrap());
    }
}
```

Wire `talk download models` in `main.rs` (loop `MODELS`, `download::fetch(art, &paths::models_dir())`, print progress). Gate `mod download;` on the `download` feature. Before the binary loads a model (T8 setup, called from T12), it must `download::verify` the file against the pinned hash and refuse to run on mismatch.

- [ ] **Step 4: Test the verify logic here**

Run: `cargo test --features download verify_matches_known_hash`
Expected: PASS (1 test). (The actual fetch is exercised in Step 5 on your machine.)

- [ ] **Step 5: Pin the hashes + real fetch (your machine)**

`talk download models` once (it will fail the checksum with `FILL_AT_PIN_TIME`); take the "got <hash>" value it reports for each artifact, paste into `models.rs`, and re-run — it should now verify and keep the files. Commit the real hashes.

- [ ] **Step 6: Commit**

```bash
git add src/download/ src/paths.rs src/main.rs Cargo.toml Cargo.lock
git commit -m "feat(download): pinned-SHA-256 model fetch + verify gate"
```

---

## Task 10: TALK_BASE_DIR env override [here-testable]

**Files:**
- Modify: `src/paths.rs`

The round-1 review and the Plan-1 roadmap both noted `paths::base_dir` should honor a `TALK_BASE_DIR` env override (more robust than relying on `HOME` for tests + power users).

- [ ] **Step 1: Write the failing test**

In `src/paths.rs`, change `base_dir` to consult the env first:

```rust
pub fn base_dir(override_path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = override_path { return p; }
    if let Ok(custom) = std::env::var("TALK_BASE_DIR") {
        if !custom.is_empty() { return PathBuf::from(custom); }
    }
    directories::UserDirs::new()
        .map(|d| d.home_dir().join("talk"))
        .unwrap_or_else(|| PathBuf::from("talk"))
}
```

Add a test (note: env tests must not run in parallel with other env users; this var is unique to talk):

```rust
    #[test]
    fn base_dir_honors_talk_base_dir_env() {
        // Safe: TALK_BASE_DIR is talk-specific; restore after.
        std::env::set_var("TALK_BASE_DIR", "/tmp/talk-test-xyz");
        assert_eq!(base_dir(None), PathBuf::from("/tmp/talk-test-xyz"));
        std::env::remove_var("TALK_BASE_DIR");
        // explicit override still wins
        std::env::set_var("TALK_BASE_DIR", "/tmp/ignored");
        assert_eq!(base_dir(Some(PathBuf::from("/tmp/explicit"))), PathBuf::from("/tmp/explicit"));
        std::env::remove_var("TALK_BASE_DIR");
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test paths`
Expected: PASS (4 tests — the 3 from Plan 1 + this one).

- [ ] **Step 3: Commit**

```bash
git add src/paths.rs && git commit -m "feat: TALK_BASE_DIR env override for base dir"
```

---

## Task 11: Live interactive session loop [needs your machine — mic + models + a terminal]

**Files:**
- Create: `src/live.rs`
- Modify: `src/main.rs` (`mod live;` + route the real session to it when no `--from-text`)

The interactive loop: enter the `Screen`, then each ~50ms tick: pump `LiveSource` events into the `Settle` machine, poll a keypress (`crossterm::event::poll`/`read` → `keymap::action_for`), repaint via `render::paint`, and on `Finish` finalize (flush LiveSource, write via the Plan-1 `writer`, paint the close/released screen). Reuses the Plan-1 `writer`, `state`, selection, and config wiring from `reflect`/journal.

- [ ] **Step 1: Implement the loop**

Create `src/live.rs`:

```rust
use std::time::{Duration, Instant};
use crossterm::event::{self, Event as CtEvent};
use talk_core::render_model::{compose_close, compose_released, Mode as RMode, View};
use talk_core::settle::Settle;
use crate::keymap::{action_for, Action};
use crate::render::{paint, paint_plain, Screen};
use crate::source::{Event, TranscriptSource};

pub struct LiveConfig<'a> {
    pub mode: RMode,
    pub question: Option<&'a str>,
    pub held_label: Option<&'a str>,
    pub cleanup: &'a str,
    pub ephemeral: bool,
}

/// Outcome the caller uses to persist (or not) and print the close screen.
pub struct LiveResult {
    pub raw: String,
    pub clean: String,
    pub cancelled: bool,
}

/// Run the interactive loop. `source` is a LiveSource (mic) in production; tests
/// pass a scripted source. Returns the joined raw+clean transcript (or cancelled).
pub fn run_loop(
    source: &mut dyn TranscriptSource,
    finish: &mut dyn FnMut(),   // LiveSource::finish — flushes VAD on [space]
    cfg: &LiveConfig,
) -> std::io::Result<LiveResult> {
    let _screen = Screen::enter()?;
    let mut settle = Settle::new();
    let mut listening = false;
    let mut show_raw = false;
    let mut paused = false;
    let started = Instant::now();
    let mut finished = false;

    loop {
        // 1. drain transcript events
        if !paused {
            while let Some(ev) = source.next() {
                match ev {
                    Event::Partial(_) => listening = true,
                    Event::Commit(raw) => {
                        let pre = talk_core::cleanup::apply_backtrack(
                            &talk_core::cleanup::apply_spoken_commands(&raw));
                        let clean = talk_core::cleanup::deterministic_light(&pre);
                        settle.commit(&raw, &clean);
                        listening = false;
                    }
                    Event::Done => { finished = true; break; }
                }
            }
        }

        // 2. paint
        let elapsed = fmt_elapsed(started.elapsed());
        let v = View {
            mode: cfg.mode, question: cfg.question, held_label: cfg.held_label,
            settle: &settle, listening, elapsed: &elapsed, cleanup: cfg.cleanup, show_raw,
        };
        paint(&v)?;

        if finished { break; }

        // 3. poll a key for ~50ms
        if event::poll(Duration::from_millis(50))? {
            if let CtEvent::Key(k) = event::read()? {
                match action_for(k) {
                    Action::Finish => { finish(); finished_drain(source, &mut settle); break; }
                    Action::Cancel => {
                        settle.finalize();
                        return Ok(LiveResult { raw: String::new(), clean: String::new(), cancelled: true });
                    }
                    Action::ToggleRaw => show_raw = !show_raw,
                    Action::TogglePause => paused = !paused,
                    Action::None => {}
                }
            }
        }
    }

    settle.finalize();
    let raw = settle.settled().iter().map(|b| b.raw.as_str()).collect::<Vec<_>>().join(" ");
    let clean = settle.settled().iter().map(|b| b.clean.as_str()).collect::<Vec<_>>().join(" ");
    Ok(LiveResult { raw, clean, cancelled: false })
}

/// After [space]: finish() was called (queues trailing Commit(s) + Done); drain them.
fn finished_drain(source: &mut dyn TranscriptSource, settle: &mut Settle) {
    while let Some(ev) = source.next() {
        match ev {
            Event::Commit(raw) => {
                let pre = talk_core::cleanup::apply_backtrack(
                    &talk_core::cleanup::apply_spoken_commands(&raw));
                settle.commit(&raw, &talk_core::cleanup::deterministic_light(&pre));
            }
            Event::Done => break,
            Event::Partial(_) => {}
        }
    }
}

fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// Print the close screen (caller passes path/provenance/phrase) or released.
pub fn show_close(path: &str, provenance: &str, phrase: &str) -> std::io::Result<()> {
    paint_plain(&compose_close(path, provenance, phrase))
}
pub fn show_released() -> std::io::Result<()> {
    paint_plain(&compose_released())
}
```

Add `mod live;` to `src/main.rs`.

> **Testability note:** `run_loop` takes a `&mut dyn TranscriptSource` and a `finish` closure, so it can be driven by a `FakeTranscript` in a headless test that pipes scripted `Commit`/`Done` events and a synthetic finish — but the crossterm `Screen`/`event::poll` make a true unit test impractical here. The pure pieces it composes (`compose`, `action_for`, `settle`, `cleanup`) are all already unit-tested. This loop is verified end-to-end on your machine in Step 3.

- [ ] **Step 2: Wire main.rs**

In `src/main.rs`, when `--from-text` is **absent** and the `listen` feature is on, the real session uses the mic: build `Capture` → `Segmenter` (verified `silero_vad.onnx`) → `Stt` (verified Moonshine paths), wrap in `LiveSource`, pick the question/mode exactly as the Plan-1 `reflect`/journal/unburden paths do, call `live::run_loop`, then on a non-cancelled result write via the existing `writer` (or skip for ephemeral) and `live::show_close`/`show_released`. When `--from-text` is present, keep the Plan-1 `session::run` path unchanged (so all existing tests pass). Build models-missing handling: if a model file is absent or fails `download::verify`, print "run `talk download models`" and exit cleanly (no panic).

- [ ] **Step 3: End-to-end on your machine**

```
talk download models                 # fetch + verify (Task 9 Step 5 first)
talk reflect                         # speak a sentence, pause, speak another, press space
```
Confirm: the question shows, `● listening` pulses while you speak, each phrase settles in clean on your pause, `u` toggles raw⇄clean, `space` writes `~/talk/<slug>.md` and shows the close phrase, `esc` discards. Then `talk journal`, and `talk unburden` (confirm "Released. Nothing was written." + no file). Verify `cargo test` (the `--from-text` paths) still all pass.

- [ ] **Step 4: Commit**

```bash
git add src/live.rs src/main.rs
git commit -m "feat: live interactive session loop (mic → settle → file)"
```

---

## Self-Review (completed during authoring)

- **Spec coverage (Plan-2 scope):** restore palette §2 (T1) · settle render incl. status line + question box + close + ephemeral screens §7 (T2–T3, T5) · in-session keys §7 (T4) · cpal mic capture §5 (T6) · Silero VAD + Moonshine settle-on-pause §5/§7 (T7–T8) · `● local · no network` + privacy chrome §7/§11 (T2) · model download + pinned-SHA-256 verify gate §11 (T9) · `TALK_BASE_DIR` (T10) · live session → file §17 (T11). **Deferred to Plan 3/4 (correctly):** the 0.5B formatter + its latency spike (Plan 3); real spine + flagship packs, ephemeral zeroize/mlock, sidecar raw store, streak display, no-egress/tamper tests (Plan 4). The settle-on-pause amendment is recorded in the spec and the Design-delta section.
- **Placeholder scan:** the only deferral is the two `sha256: "FILL_AT_PIN_TIME"` values (a hash is physically unknowable until the asset is fetched — Task 9 Step 5 pins them in one command; the *verify code* that consumes them is complete). The `sherpa-onnx` version + a few binding symbol names (`Vad`/`VoiceActivityDetector`, VAD config fields) are marked "confirm against the pinned example" — legitimate because the binding is version-sensitive and the authoritative source is named; not a logic placeholder.
- **Type consistency:** `View` fields, `compose`/`compose_close`/`compose_released`, `Action`, `Capture`/`Segmenter`/`Stt`/`LiveSource`, `Event` (reused from Plan 1's `source.rs`), and `Settle`'s `commit`/`finalize`/`settled`/`committing` are used consistently across tasks. `LiveSource` implements the existing `TranscriptSource` so the Plan-1 `session::run` and the new `live::run_loop` accept the same seam.
- **Execution venue:** T1–T5, T10 are built + unit-tested in CI here; T6–T9, T11 need a mic / model files / network and are verified on your machine via the named manual checks.

---

## Execution Handoff

(See the prompt that follows for the two execution options. Note: only T1–T5 and T10 are fully executable in this environment; T6–T9 and T11 need your hardware/models — those will be handed to you with the exact on-machine checks above.)
