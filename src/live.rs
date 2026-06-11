use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use crossterm::event::{self, Event as CtEvent, KeyCode};
use talk_core::render_model::{compose_close, compose_released, Mode as RMode, View};
use talk_core::settle::Settle;
use crate::keymap::{action_for, Action};
use crate::render::{paint, paint_plain, Screen};
use crate::source::{Event, TranscriptSource};

const SPEECH_HANGOVER: Duration = Duration::from_millis(350);

/// Curated close phrases (spec §7) — rotated by entry count so a returning user
/// doesn't see the same line twice in a row.
pub const CLOSE_PHRASES: &[&str] = &[
    "Stillness carries forward.",
    "Said out loud, it weighs less.",
    "You showed up. That was the practice.",
    "Let it settle.",
    "The thread holds.",
    "Nothing to fix. Just to hear.",
    "The words keep working after you stop.",
    "Come back when it tugs.",
];

pub struct LiveConfig<'a> {
    pub mode: RMode,
    pub question: Option<&'a str>,
    pub held_label: Option<&'a str>,
    pub cleanup: &'a str,
    pub ephemeral: bool,
    pub palette: talk_core::palette::Palette,
    pub lexicon: &'a crate::lexicon::Lexicon,
}

/// Outcome the caller uses to persist (or not) and print the close screen.
pub struct LiveResult {
    pub raw: String,
    pub clean: String,
    pub cancelled: bool,
}

/// Pause + pass-2 pairing guards, shared by run_loop and the finish drain and
/// extracted so the privacy invariant is unit-testable without a terminal.
struct EventGuards {
    paused: bool,
    /// The worker emits Commit then Revise serially. A Revise whose paired Commit
    /// was dropped during pause must be dropped too, or pass-2 text of OFF-RECORD
    /// (paused) speech would overwrite the last accepted phrase. Re-armed only by
    /// the next ACCEPTED Commit — never by resume, because the off-record Revise
    /// can arrive after the user unpauses.
    commit_dropped: bool,
}

/// Apply one transcript event under the guards. Returns true on Done.
///
/// Arm ORDER is load-bearing: the Revise arms sit ABOVE the `_ if paused`
/// catch-all because Revise is gated on `commit_dropped`, not on `paused` — a
/// straddling Revise (Commit accepted on-record, pass-2 landing during a pause)
/// must still upgrade its block, while a Commit arriving during pause must be
/// dropped AND disarm its own in-flight Revise.
fn apply_event(ev: Event, g: &mut EventGuards, settle: &mut Settle, lexicon: &crate::lexicon::Lexicon) -> bool {
    match ev {
        Event::Done => return true,
        Event::Revise(raw2) if !g.commit_dropped => {
            let prev = settle.settled().last().map(|b| b.clean.clone());
            let clean = talk_core::cleanup::format_revise(
                &crate::lexicon::correct(&raw2, lexicon), prev.as_deref());
            settle.revise_committing(&raw2, &clean);
        }
        Event::Revise(_) => {} // paired Commit was dropped (off-record) → drop its pass-2 too
        Event::Commit(_) if g.paused => { g.commit_dropped = true; }
        _ if g.paused => {} // drain-and-discard while paused: don't record, don't grow the channel
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

/// Run the interactive loop. `source` is a LiveSource (mic) in production; tests
/// pass a scripted source. `speaking` reports streaming-partial activity via a
/// cloned Arc handle (tests pass `Arc::new(AtomicBool::new(false))`). `finish_flag` is the
/// cloned finish handle the [space] action sets; `pause` tells the worker to go
/// off-record at the audio level (destroy the hypothesis, drop chunks). Returns
/// the joined raw+clean transcript (or cancelled).
pub fn run_loop(
    source: &mut dyn TranscriptSource,
    finish_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pause: std::sync::Arc<crate::source::PauseSignal>,
    speaking: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cfg: &LiveConfig,
) -> std::io::Result<LiveResult> {
    let _screen = Screen::enter()?;
    let mut settle = Settle::new();
    let mut show_raw = false;
    let mut confirm_cancel = false;
    let started = Instant::now();
    let mut paused_total = Duration::ZERO;     // accumulated paused time (excluded from elapsed)
    let mut paused_at: Option<Instant> = None;
    let mut last_speech = started - SPEECH_HANGOVER; // start un-latched
    let mut finished = false;
    let mut guards = EventGuards { paused: false, commit_dropped: false };

    loop {
        // 1. drain transcript events (ALWAYS drain to keep the channel from growing;
        // discard them while paused)
        while let Some(ev) = source.next() {
            if apply_event(ev, &mut guards, &mut settle, cfg.lexicon) { finished = true; break; }
        }
        if !guards.paused && speaking.load(Ordering::Relaxed) { last_speech = Instant::now(); }

        // 2. paint
        let listening = !guards.paused && last_speech.elapsed() < SPEECH_HANGOVER; // latch → no flicker
        let elapsed = fmt_elapsed(started.elapsed() - paused_total
            - paused_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO));
        let v = View {
            mode: cfg.mode, question: cfg.question, held_label: cfg.held_label,
            settle: &settle, listening, elapsed: &elapsed, cleanup: cfg.cleanup,
            show_raw, paused: guards.paused, confirm_cancel,
        };
        paint(&v, cfg.palette)?;

        if finished { break; }

        // 3. poll a key for ~50ms
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                CtEvent::Resize(_, _) => {} // next compose recomputes from terminal size
                CtEvent::Key(k) => {
                    // While confirming a cancel, read the raw y/n decision here.
                    if confirm_cancel {
                        match k.code {
                            KeyCode::Char('y') | KeyCode::Esc => {
                                settle.finalize();
                                return Ok(LiveResult { raw: String::new(), clean: String::new(), cancelled: true });
                            }
                            _ => { confirm_cancel = false; } // any other key resumes
                        }
                        continue;
                    }
                    match action_for(k) {
                        Action::Finish => {
                            finish_flag.store(true, Ordering::Relaxed);
                            drain_until_done(source, &mut settle, &mut guards, cfg.lexicon)?;
                            break;
                        }
                        Action::Cancel => {
                            if cfg.ephemeral {
                                settle.finalize(); // nothing at risk — cancel immediately
                                return Ok(LiveResult { raw: String::new(), clean: String::new(), cancelled: true });
                            }
                            confirm_cancel = true; // show the discard prompt; decide next key
                        }
                        Action::ToggleRaw => show_raw = !show_raw,
                        Action::TogglePause => {
                            guards.paused = !guards.paused;
                            if guards.paused {
                                // The worker goes off-record at the audio level: it
                                // destroys the in-flight hypothesis, segment audio and
                                // parked chunks, and discards new chunks until resume.
                                pause.pause();
                                paused_at = Some(Instant::now());
                                // Clear the live edge: speech in flight is now off-record,
                                // and the worker's change-only emission never sends a
                                // clearing event — so the edge must not keep advertising
                                // the stale partial.
                                settle.on_partial("");
                            } else {
                                pause.resume();
                                if let Some(t) = paused_at.take() {
                                    paused_total += t.elapsed();
                                }
                            }
                        }
                        Action::None => {}
                    }
                }
                _ => {}
            }
        }
    }

    settle.finalize();
    let raw = settle.settled().iter().map(|b| b.raw.as_str()).collect::<Vec<_>>().join(" ");
    let clean = settle.settled().iter().map(|b| b.clean.as_str()).collect::<Vec<_>>().join(" ");
    Ok(LiveResult { raw, clean, cancelled: false })
}

/// After [space]: setting finish_flag signaled the worker to flush trailing
/// Commit(s) + Done. Because `next()` is non-blocking (off-thread worker), BLOCK
/// here (poll with a short sleep + an 8s LIVENESS deadline, reset on every event)
/// until Done — otherwise a `while let Some` loop exits on the first empty poll and
/// drops the trailing phrase. The liveness deadline means a slow-but-progressing
/// transcribe is never cut off; only a genuinely hung worker (no progress for 8s)
/// is. Paint a calm "settling…" frame so [space] gives immediate feedback while the
/// last segment transcribes (no silent hang).
///
/// Shares run_loop's guards: the pairing state carries in so an in-flight Revise of
/// a pause-dropped Commit is still dropped here, while `paused` is lifted — the
/// worker goes off-record at the audio level, so anything still in the channel at
/// finish is on-record and must land.
fn drain_until_done(
    source: &mut dyn TranscriptSource,
    settle: &mut Settle,
    guards: &mut EventGuards,
    lexicon: &crate::lexicon::Lexicon,
) -> std::io::Result<()> {
    paint_plain(&["  settling…".to_string()])?;
    guards.paused = false;
    let idle_limit = Duration::from_secs(8);
    let mut last_event = Instant::now();
    loop {
        match source.next() {
            Some(ev) => {
                if apply_event(ev, guards, settle, lexicon) { break; }
                last_event = Instant::now();
            }
            None => {
                if last_event.elapsed() >= idle_limit { break; } // no progress for 8s → worker hung
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    Ok(())
}

fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

/// RAII raw-mode guard for prompts that run AFTER the `Screen` guard dropped:
/// cooked-mode line buffering would make a single-key prompt appear hung until
/// Enter. No alternate screen — the prompt text must stay visible.
struct RawPrompt;

impl RawPrompt {
    fn enter() -> std::io::Result<RawPrompt> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(RawPrompt)
    }
}

impl Drop for RawPrompt {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Wait for a single keypress (discarding non-key events) so a contemplative
/// close screen isn't flashed away before the reader sees it.
fn await_keypress() -> std::io::Result<()> {
    let _raw = RawPrompt::enter()?;
    loop {
        if let CtEvent::Key(_) = event::read()? { return Ok(()); }
    }
}

/// Print the close screen (caller passes path/provenance/phrase), then dwell.
pub fn show_close(path: &str, provenance: &str, phrase: &str) -> std::io::Result<()> {
    paint_plain(&compose_close(path, provenance, phrase))?;
    await_keypress()
}
/// Print the released screen, then dwell.
pub fn show_released() -> std::io::Result<()> {
    paint_plain(&compose_released())?;
    await_keypress()
}

/// What the user chose at the write-failure prompt (spec §13).
pub enum Recover {
    Retry,
    Clipboard,
    Discard,
}

/// Inline write-failure prompt. Returns the chosen action.
pub fn ask_recover(err: &str, attempts: u32) -> std::io::Result<Recover> {
    let hint = if attempts >= 3 {
        " (3 failures — clipboard recommended)"
    } else {
        ""
    };
    paint_plain(&[
        format!("  write failed: {err}{hint}"),
        "  [r]etry · [c]opy to clipboard · [d]iscard".to_string(),
    ])?;
    let _raw = RawPrompt::enter()?;
    loop {
        if let CtEvent::Key(k) = event::read()? {
            match k.code {
                KeyCode::Char('r') | KeyCode::Esc => return Ok(Recover::Retry),
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
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(text.as_bytes())?;
    if cfg!(target_os = "macos") {
        // pbcopy exits once stdin closes, so its status is meaningful.
        let status = child.wait()?;
        if !status.success() {
            return Err(std::io::Error::other("clipboard helper exited non-zero"));
        }
    }
    // Not on macOS: xclip stays alive to serve the X11 selection — waiting on it
    // would hang until another client takes the clipboard over.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guards(paused: bool, commit_dropped: bool) -> EventGuards {
        EventGuards { paused, commit_dropped }
    }

    fn empty_lex() -> crate::lexicon::Lexicon {
        crate::lexicon::Lexicon::from_map(std::collections::BTreeMap::new())
    }

    #[test]
    fn revise_of_a_pause_dropped_commit_is_dropped_even_after_resume() {
        let mut settle = Settle::new();
        let mut g = guards(false, false);
        apply_event(Event::Commit("kept phrase".into()), &mut g, &mut settle, &empty_lex());
        g.paused = true;
        apply_event(Event::Commit("OFF RECORD".into()), &mut g, &mut settle, &empty_lex());
        assert!(g.commit_dropped, "paused Commit must disarm its Revise");
        g.paused = false; // resume does NOT re-arm — the off-record Revise is still in flight
        apply_event(Event::Revise("off record, transcribed better".into()), &mut g, &mut settle, &empty_lex());
        assert_eq!(settle.committing().unwrap().raw, "kept phrase");
    }

    #[test]
    fn straddling_revise_applies_while_paused() {
        // Commit accepted on-record; pass-2 lands during a pause → still upgrades.
        let mut settle = Settle::new();
        let mut g = guards(false, false);
        apply_event(Event::Commit("rough streaming text".into()), &mut g, &mut settle, &empty_lex());
        g.paused = true;
        apply_event(Event::Revise("Rough streaming text, corrected.".into()), &mut g, &mut settle, &empty_lex());
        assert_eq!(settle.committing().unwrap().raw, "Rough streaming text, corrected.");
    }

    #[test]
    fn an_accepted_commit_rearms_the_pairing_guard() {
        let mut settle = Settle::new();
        let mut g = guards(false, true); // a prior paused Commit left the guard disarmed
        apply_event(Event::Commit("next on-record phrase".into()), &mut g, &mut settle, &empty_lex());
        assert!(!g.commit_dropped);
        apply_event(Event::Revise("next on-record phrase, revised".into()), &mut g, &mut settle, &empty_lex());
        assert_eq!(settle.committing().unwrap().raw, "next on-record phrase, revised");
    }

    #[test]
    fn finish_drain_inherits_the_disarmed_guard() {
        // [space] lands inside the pass-2 window of a pause-dropped Commit: the
        // drain applies events under the SAME guards, so the off-record Revise
        // must not overwrite the accepted phrase even though the drain lifts
        // `paused` (the leak the review's P0/P1 cluster named).
        let mut settle = Settle::new();
        let mut g = guards(true, false);
        apply_event(Event::Commit("OFF RECORD".into()), &mut g, &mut settle, &empty_lex());
        g.paused = false; // drain_until_done lifts paused but carries commit_dropped
        apply_event(Event::Revise("off record revised".into()), &mut g, &mut settle, &empty_lex());
        assert!(settle.committing().is_none(), "nothing may land from the paused pair");
        assert!(apply_event(Event::Done, &mut g, &mut settle, &empty_lex()));
    }

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

    /// On macOS dev machines `pbcopy` exists — copied text must read back via
    /// `pbpaste` verbatim. Headless runners without a pasteboard make `pbcopy`
    /// fail to spawn; skip there so CI doesn't flake.
    #[test]
    #[cfg(target_os = "macos")]
    fn clipboard_roundtrip() {
        match std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            // Reap the probe BEFORE the real copy: dropping its piped stdin makes
            // it write an EMPTY pasteboard, which must not race the sentinel.
            Ok(mut probe) => {
                let _ = probe.wait();
            }
            Err(_) => return,
        }
        let unique = "talk-cli clipboard roundtrip 2026-06-09T08:14 sentinel";
        copy_to_clipboard(unique).unwrap();
        let pasted = std::process::Command::new("pbpaste").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&pasted.stdout), unique);
    }
}
