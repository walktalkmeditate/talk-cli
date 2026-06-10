mod cli;
mod config;
#[cfg(feature = "download")]
mod download;
mod keymap;
#[cfg(feature = "listen")]
mod live;
#[cfg(feature = "listen")]
mod listen;
mod packs;
mod paths;
mod render;
mod session;
mod source;
mod state;
mod streak;
mod writer;

use clap::Parser;
use cli::{Cli, Command};
use session::{run, RunConfig};
use source::FakeTranscript;
use std::path::{Path, PathBuf};
use writer::Target;

fn main() -> std::io::Result<()> {
    let args = Cli::parse();

    // Config first (it may relocate the base dir); then resolve + validate base.
    let cfg = load_config()?;
    let base = paths::resolve_base(cfg.base_dir.as_deref())?;
    paths::ensure_base_dir(&base)?;

    let date = args.date.clone().unwrap_or_else(system_date);
    let time = args.time.clone().unwrap_or_else(system_time_hm);

    // Live mic session: only when this build has `listen` AND no `--from-text`
    // override AND the command is a session command. Non-session commands and the
    // `--from-text` path fall through to the existing match below, unchanged.
    #[cfg(feature = "listen")]
    if args.from_text.is_none() {
        if let Some(handled) = run_live_session(&base, &args, &date, &time, &cfg)? {
            return handled;
        }
    }

    match args.command {
        Some(Command::Journal) => {
            let text = require_text(&args.from_text);
            disclose_once(&base)?;
            run_and_report(Report { base: &base, target: Target::Journal, date: &date, time: &time, text: &text, keep_raw: cfg.keep_raw, raw_sidecar: cfg.raw_sidecar, ephemeral: false, level: cfg.cleanup_for("journal"), held_day: None })?;
        }
        Some(Command::Unburden) | Some(Command::Vent) => {
            let text = require_text(&args.from_text);
            run_and_report(Report { base: &base, target: Target::Journal, date: &date, time: &time, text: &text, keep_raw: cfg.keep_raw, raw_sidecar: cfg.raw_sidecar, ephemeral: true, level: talk_core::cleanup::Level::None, held_day: None })?;
            println!("Released. Nothing was written.");
        }
        Some(Command::Config { action }) => return handle_config(action.as_deref()),
        Some(Command::Thread { question }) => print_thread(&base, question.as_deref()),
        Some(Command::Streak) => {
            let s = streak::Streak::load_from(&base);
            if s.entries == 0 {
                println!("No reflections yet — run `talk` to start.");
            } else {
                println!("{} reflection{} · current run {} day{} · longest {}",
                    s.entries, if s.entries == 1 { "" } else { "s" },
                    s.current_streak, if s.current_streak == 1 { "" } else { "s" },
                    s.longest_streak);
            }
            return Ok(());
        }
        Some(Command::Download { target }) => return handle_download(target.as_deref()),
        Some(Command::Reflect) => reflect(&base, &args.question, &date, &time, &require_text(&args.from_text), &cfg)?,
        // Bare `talk`: honor config.default_mode (journal) unless a BYO question was given.
        _ => {
            if args.question.is_none() && cfg.default_mode == "journal" {
                let text = require_text(&args.from_text);
                disclose_once(&base)?;
                run_and_report(Report { base: &base, target: Target::Journal, date: &date, time: &time, text: &text, keep_raw: cfg.keep_raw, raw_sidecar: cfg.raw_sidecar, ephemeral: false, level: cfg.cleanup_for("journal"), held_day: None })?;
            } else {
                reflect(&base, &args.question, &date, &time, &require_text(&args.from_text), &cfg)?;
            }
        }
    }
    Ok(())
}

/// The chosen reflect question plus the `State` mutated by the selection (which
/// must be persisted AFTER a successful write, so a failed write doesn't burn a
/// rotation). Shared by the `--from-text` path and the live mic path.
struct ReflectChoice {
    id: String,
    question: String,
    slug: String,
    pack: String,
    addressee: String,
    /// 1-based day of an active `held:N` run for this serving; `None` for daily
    /// questions and BYO. Computed BEFORE the serving advances state.
    held_day: Option<u32>,
    state: state::State,
}

/// Select a reflect question: a BYO question if one was given, else from the
/// configured pack (recording the serving in the returned `State`).
fn reflect_choice(base: &Path, byo: &Option<String>, time: &str, default_pack: &str) -> std::io::Result<ReflectChoice> {
    let mut st = state::State::load(&std::fs::read_to_string(state_path(base)).unwrap_or_default());

    let (id, question, slug, pack, addressee, held_day) = match byo {
        Some(q) => {
            // BYO: id == slug, collision-suffixed against a DIFFERENT existing
            // question. A slug shaped like a journal filename (YYYY-MM-DD) is also
            // treated as taken, so a BYO can never collide with the journal date-file
            // namespace (a journal append onto reflect frontmatter would corrupt it).
            let slug = talk_core::slug::derive_slug_unique(q, |s| {
                is_journal_date_slug(s) || slug_taken_by_other(&base.join(format!("{}.md", s)), q)
            });
            (slug.clone(), q.clone(), slug, "byo".to_string(), "self".to_string(), None)
        }
        None => {
            let pack = crate::packs::by_name(default_pack);
            let chosen = match talk_core::selection::select(&pack, &st.selection_state(), hour_of(time)) {
                Some(q) => q.clone(),
                None => return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData, "pack has no questions")),
            };
            // A held run lives in the pack that started it; if the active pack
            // changed mid-run, the run's question won't be served here — abandon
            // it cleanly rather than stranding a half-finished run.
            if let Some((run_id, _)) = &st.held_run {
                if run_id != &chosen.id {
                    st.held_run = None;
                    eprintln!("your held run paused — it lives in the `held` pack");
                }
            }
            let slug = chosen.slug.clone().unwrap_or_else(|| chosen.id.clone());
            // The held day reflects the PRE-advance state — this serving is day
            // `done + 1` of the run (or day 1 of a fresh one).
            let held_day = state::held_day_for(&st.held_run, &chosen.id, &chosen.cadence);
            st.record_served(&chosen.id);
            st.advance_held(&chosen);
            (chosen.id, chosen.text, slug, pack.name, chosen.addressee, held_day)
        }
    };

    Ok(ReflectChoice { id, question, slug, pack, addressee, held_day, state: st })
}

/// Reflect: a BYO question if one was given, else select from the spine pack.
fn reflect(base: &Path, byo: &Option<String>, date: &str, time: &str, text: &str, cfg: &config::Config) -> std::io::Result<()> {
    disclose_once(base)?;
    let c = reflect_choice(base, byo, time, &cfg.default_pack)?;

    let target = Target::Reflect { id: &c.id, question: &c.question, slug: &c.slug, pack: &c.pack, addressee: &c.addressee };
    run_and_report(Report { base, target, date, time, text, keep_raw: cfg.keep_raw, raw_sidecar: cfg.raw_sidecar, ephemeral: false, level: cfg.cleanup_for("reflect"), held_day: c.held_day })?;
    paths::write_private(&state_path(base), &c.state.save())?;
    Ok(())
}

struct Report<'a> {
    base: &'a Path,
    target: Target<'a>,
    date: &'a str,
    time: &'a str,
    text: &'a str,
    keep_raw: bool,
    raw_sidecar: bool,
    ephemeral: bool,
    level: talk_core::cleanup::Level,
    /// 1-based held-run day for the printed provenance; `None` for non-held entries.
    held_day: Option<u32>,
}

fn run_and_report(r: Report) -> std::io::Result<()> {
    let path = run(&mut FakeTranscript::from_text(r.text), r.target,
        &RunConfig {
            base: r.base, date: r.date, time: r.time, keep_raw: r.keep_raw, raw_sidecar: r.raw_sidecar, ephemeral: r.ephemeral,
            formatter: &talk_core::format::DeterministicFormatter, level: r.level,
        })?;
    if let Some(p) = path {
        match r.held_day {
            Some(d) => println!("→ {} · held day {}", p.display(), d),
            None => println!("→ {}", p.display()),
        }
        // A saved entry credits the streak (ephemeral writes nothing, so it never
        // reaches here with a path — gated anyway). Streak failure never blocks a save.
        if !r.ephemeral {
            if let Some(day) = streak::civil_day(r.date) {
                let _ = streak::record_entry(r.base, day);
            }
        }
    }
    Ok(())
}

/// Drive a real mic session for a session command (reflect / journal / unburden /
/// vent / bare). Returns `Ok(None)` for non-session commands (Config / Thread /
/// Streak / Download) so `main` falls through to the existing dispatch. A returned
/// `Ok(Some(result))` is the value `main` should return.
#[cfg(feature = "listen")]
fn run_live_session(
    base: &Path,
    args: &Cli,
    date: &str,
    time: &str,
    cfg: &config::Config,
) -> std::io::Result<Option<std::io::Result<()>>> {
    use talk_core::render_model::Mode as RMode;
    use zeroize::Zeroize;

    // Which session shape are we in? Non-session commands → fall through.
    enum Shape { Reflect, Journal, Ephemeral }
    let shape = match &args.command {
        Some(Command::Reflect) => Shape::Reflect,
        Some(Command::Journal) => Shape::Journal,
        Some(Command::Unburden) | Some(Command::Vent) => Shape::Ephemeral,
        // Bare `talk`: honor default_mode (journal) unless a BYO question was given.
        None => {
            if args.question.is_none() && cfg.default_mode == "journal" {
                Shape::Journal
            } else {
                Shape::Reflect
            }
        }
        Some(Command::Config { .. })
        | Some(Command::Thread { .. })
        | Some(Command::Streak)
        | Some(Command::Download { .. }) => return Ok(None),
    };

    // First-run disclosure: once a real (non-ephemeral) session is about to begin,
    // before the models gate, so a brand-new user sees the privacy note first, then
    // any fetch offer. Ephemeral discloses nothing — it writes nothing to disclose.
    // TTY-gated: a non-TTY listen invocation (e.g. `talk journal` with no
    // `--from-text` under test) never reaches a real session, and must write zero
    // bytes (it falls through to the models gate's clean non-zero exit).
    let interactive = unsafe { libc::isatty(0) } == 1;
    if interactive && !matches!(shape, Shape::Ephemeral) {
        disclose_once(base)?;
    }

    // Verify every model before loading anything; an unpinned / missing / mismatched
    // artifact means the session can't run yet. On a TTY we offer to fetch them now
    // (a one-time ~219 MB download) and continue; otherwise we print the hint and
    // exit non-zero (a clean failure — no terminal state or file has been touched).
    if !models_ready() {
        if interactive && offer_first_run_fetch()? {
            fetch_all_models()?;
        } else {
            eprintln!("models not ready — run `talk download models`");
            std::process::exit(1);
        }
    }

    // Pass-2 transcription loads Moonshine base (drop-in for the old tiny `Stt`).
    // The streaming Zipformer-20M paths are computed here too; the `Streaming`
    // facade that consumes them lands in Plan 5 T4/T5 — for now this build still
    // segments via the VAD path while the manifest gate already requires the new
    // models. Field order tracks the eventual `LiveSource::new` signature.
    let models = paths::models_dir();
    let moonshine = models.join("sherpa-onnx-moonshine-base-en-quantized-2026-02-27");
    let zipformer = models.join("sherpa-onnx-streaming-zipformer-en-20M-2023-02-17");
    let (Some(enc), Some(dec), Some(tok), Some(silero)) = (
        moonshine.join("encoder_model.ort").to_str().map(str::to_owned),
        moonshine.join("decoder_model_merged.ort").to_str().map(str::to_owned),
        moonshine.join("tokens.txt").to_str().map(str::to_owned),
        // Plan 5 T4 deletes the VAD path; until then the live session still
        // segments with silero, so this path is still constructed here.
        models.join("silero_vad.onnx").to_str().map(str::to_owned),
    ) else {
        eprintln!("model path is not valid UTF-8 — run `talk download models`");
        std::process::exit(1);
    };
    let _ = &zipformer; // wired into the `Streaming` facade in Plan 5 T5.

    // Build the per-mode View config and the write target. For reflect we resolve
    // the question (BYO or spine) up front; its owned strings back the Target.
    // For a BYO question, a near-match against an EXISTING BYO thread offers to
    // continue that thread instead of silently forking it (spec §8) — live + TTY
    // only; pack/spine threads are never merged (they're served by id). Resolved
    // BEFORE the mic starts: a stdin prompt over a hot mic would transcribe the
    // user's answer-thinking into the entry.
    let byo_question = match (&shape, &args.question) {
        (Shape::Reflect, Some(q)) if interactive => byo_continue_or(base, q)?,
        _ => args.question.clone(),
    };

    let capture = match listen::capture::Capture::start() {
        Ok(c) => c,
        Err(e) => { eprintln!("microphone unavailable: {e}"); std::process::exit(1); }
    };
    let seg = match listen::vad::Segmenter::new(&silero) {
        Ok(s) => s,
        Err(e) => { eprintln!("VAD failed to load: {e}"); std::process::exit(1); }
    };
    let stt = match listen::stt::Stt::new(&enc, &dec, &tok) {
        Ok(s) => s,
        Err(e) => { eprintln!("speech model failed to load: {e}"); std::process::exit(1); }
    };

    let mut source = listen::LiveSource::new(capture, seg, stt);
    let finish_flag = source.finish_handle();
    let speaking = source.speaking_handle();

    let choice = match shape {
        Shape::Reflect => Some(reflect_choice(base, &byo_question, time, &cfg.default_pack)?),
        _ => None,
    };
    let (rmode, target, cleanup, ephemeral, question): (RMode, Target, &str, bool, Option<&str>) =
        match (&shape, &choice) {
            (Shape::Reflect, Some(c)) => (
                RMode::Reflect,
                Target::Reflect {
                    id: &c.id, question: &c.question, slug: &c.slug,
                    pack: &c.pack, addressee: &c.addressee,
                },
                "Light", false, Some(c.question.as_str()),
            ),
            (Shape::Journal, _) => (RMode::Journal, Target::Journal, "Medium", false, None),
            (Shape::Ephemeral, _) => (RMode::Ephemeral, Target::Journal, "Medium", true, None),
            (Shape::Reflect, None) => unreachable!("reflect always resolves a choice"),
        };

    // Held-run header label (e.g. "held 3 days"); bound here so it outlives
    // `live_cfg`, whose `held_label` borrows it.
    let held_label = choice.as_ref().and_then(|c| c.held_day).map(|d| {
        format!("held {} day{}", d, if d == 1 { "" } else { "s" })
    });
    let live_cfg = live::LiveConfig {
        mode: rmode, question, held_label: held_label.as_deref(), cleanup, ephemeral,
    };
    let mut result = live::run_loop(&mut source, finish_flag, speaking, &live_cfg)?;

    if result.cancelled {
        return Ok(Some(Ok(())));
    }
    if ephemeral {
        live::show_released()?;
        // The ephemeral transcript was never written; wipe its final buffers here
        // (intermediate STT buffers are out of safe-Rust reach — the threat
        // boundary is stated in the first-run disclosure).
        result.raw.zeroize();
        result.clean.zeroize();
        return Ok(Some(Ok(())));
    }
    if result.clean.trim().is_empty() {
        render::paint_plain(&["  nothing captured.".to_string()])?;
        return Ok(Some(Ok(())));
    }

    // Write with in-session recovery (spec §13): on failure offer retry /
    // clipboard / discard so the spoken words are never silently lost. Clipboard
    // and discard exits return before the rotation save and streak credit — a
    // failed write never burns the question (intentional). Discard zeroizes the
    // in-memory transcript.
    let mut attempts = 0u32;
    let written = loop {
        match writer::write_entry(&writer::WriteRequest {
            base, target, date, time,
            raw: Some(&result.raw), clean: &result.clean,
            keep_raw: cfg.keep_raw, raw_sidecar: cfg.raw_sidecar, ephemeral,
        }) {
            Ok(w) => break w,
            Err(e) => {
                attempts += 1;
                match live::ask_recover(&e.to_string(), attempts)? {
                    live::Recover::Retry => continue,
                    live::Recover::Clipboard => {
                        match live::copy_to_clipboard(&result.clean) {
                            Ok(()) => {
                                render::paint_plain(&["  copied — note: clipboard managers and Universal Clipboard may keep or sync a copy.".to_string()])?;
                            }
                            Err(ce) => {
                                render::paint_plain(&[format!("  clipboard failed too: {ce} — try [r]etry")])?;
                                continue;
                            }
                        }
                        return Ok(Some(Ok(())));
                    }
                    live::Recover::Discard => {
                        result.raw.zeroize();
                        result.clean.zeroize();
                        return Ok(Some(Ok(())));
                    }
                }
            }
        }
    };

    // Persist the reflect rotation only after the write succeeded.
    if let Some(c) = &choice {
        paths::write_private(&state_path(base), &c.state.save())?;
    }

    // A saved live entry credits the streak — BEFORE the close dwell, so Ctrl-C
    // at the close screen can't skip the credit. Ephemeral already returned above,
    // so this path is never ephemeral. Streak failure never blocks the save.
    if written.is_some() {
        if let Some(day) = streak::civil_day(date) {
            let _ = streak::record_entry(base, day);
        }
    }

    if let Some(path) = written {
        let entry_count = written_entry_count(&path);
        let held = choice.as_ref().and_then(|c| c.held_day)
            .map(|d| format!(" · held {d} days"))
            .unwrap_or_default();
        let provenance = format!("entry {entry_count}{held}");
        let phrase = live::CLOSE_PHRASES[entry_count % live::CLOSE_PHRASES.len()];
        live::show_close(&path.display().to_string(), &provenance, phrase)?;
    }
    Ok(Some(Ok(())))
}

/// For a BYO question `q`, if a near-match exists among prior BYO threads, prompt
/// to continue that thread; on `Y`/default substitute the existing question string
/// (so the normal exact-match path reuses its file). Returns `Some(question)` for
/// `reflect_choice`. Caller gates this on a TTY — non-TTY keeps current behavior.
#[cfg(feature = "listen")]
fn byo_continue_or(base: &Path, q: &str) -> std::io::Result<Option<String>> {
    let existing: Vec<String> = existing_threads(base)
        .into_iter()
        .filter(|t| t.pack == "byo")
        .map(|t| t.question)
        .collect();
    let Some(existing_q) = talk_core::matchq::near_match(q, &existing) else {
        return Ok(Some(q.to_string()));
    };
    use std::io::Write;
    print!("you've sat with \"{existing_q}\" before — continue that thread? [Y/n] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    let n = std::io::stdin().read_line(&mut line)?;
    // EOF (closed stdin) is no answer at all — start a new thread rather than
    // silently merging into one the user never confirmed.
    let continue_it = n > 0 && !matches!(line.trim(), "n" | "N");
    Ok(Some(if continue_it { existing_q.clone() } else { q.to_string() }))
}

/// First-run prompt: offer to fetch the Plan-5 models now (~219 MB is the stated
/// total — moonshine base + streaming zipformer). The returning-user copy explains
/// why the download grew and that the old caches are now dead weight. Reads one
/// stdin line; only `y`/`Y` accepts.
#[cfg(feature = "listen")]
fn offer_first_run_fetch() -> std::io::Result<bool> {
    use std::io::Write;
    print!(
        "talk's transcription engine changed (live streaming + a better model). \
new models: ~219 MB, one time. your old models are no longer used (left in place, \
~30 MB — harmless). download now? [y/N] "
    );
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y"))
}

/// Fetch every model artifact — shared by `talk download models` and the live
/// session's first-run fetch offer (one implementation, one behavior).
#[cfg(feature = "download")]
fn fetch_all_models() -> std::io::Result<()> {
    for art in download::models::MODELS {
        println!("fetching {} …", art.name);
        download::fetch(art, &paths::models_dir()).map_err(std::io::Error::other)?;
        println!("  ✓ {}", art.name);
    }
    Ok(())
}

/// True when every model the session will LOAD verifies against its pin: the
/// archives (download integrity) AND the extracted files (what the session
/// actually reads — an attacker swapping an extracted weight would slip past an
/// archive-only check). A verified archive with missing extracted files is
/// healed by re-extracting before giving up.
#[cfg(feature = "listen")]
fn models_ready() -> bool {
    let dir = paths::models_dir();
    let archives_ok = download::models::MODELS.iter().all(|art| {
        download::verify(&dir.join(art.name), art.sha256).unwrap_or(false)
    });
    if !archives_ok {
        return false;
    }
    let extracted_ok = || download::models::EXTRACTED.iter().all(|(rel, sha)| {
        download::verify(&dir.join(rel), sha).unwrap_or(false)
    });
    if extracted_ok() {
        return true;
    }
    let any_missing = download::models::EXTRACTED.iter().any(|(rel, _)| !dir.join(rel).exists());
    if any_missing {
        for art in download::models::MODELS.iter().filter(|a| a.name.ends_with(".tar.bz2")) {
            if download::extract(&dir.join(art.name), &dir).is_err() {
                return false;
            }
        }
        return extracted_ok();
    }
    false
}

/// Best-effort entry count from a freshly-written file's frontmatter (reflect) or
/// `## ` date headings (journal), for a simple `entry N` close-screen provenance.
#[cfg(feature = "listen")]
fn written_entry_count(path: &Path) -> usize {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if let Some((fm, _)) = talk_core::frontmatter::Frontmatter::parse(&text) {
        return fm.entries as usize;
    }
    text.lines().filter(|l| l.starts_with("## ")).count().max(1)
}

// config.toml always lives at the default ~/talk; base_dir only relocates where entries land.
fn config_path() -> PathBuf { paths::base_dir(None).join("config.toml") }

fn handle_config(action: Option<&str>) -> std::io::Result<()> {
    let p = config_path();
    match action {
        Some("init") => {
            if let Some(dir) = p.parent() { paths::ensure_base_dir(dir)?; }
            paths::write_private(&p, &config::Config::commented_template())?;
            println!("wrote {}", p.display());
        }
        Some("path") => println!("{}", p.display()),
        _ => print!("{}", config::Config::commented_template()),
    }
    Ok(())
}

/// `talk download` (no arg) — list what's installed. The pack list prints in
/// every build; the models section is only meaningful with the `download` feature.
fn list_installed() -> std::io::Result<()> {
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

/// `talk download` dispatch. No-arg lists installed packs + models status (in
/// every build); `talk download models` fetches the model artifacts (behind the
/// `download` feature — without it, this build can't fetch).
#[cfg(feature = "download")]
fn handle_download(target: Option<&str>) -> std::io::Result<()> {
    match target {
        None => list_installed(),
        Some("models") => fetch_all_models(),
        Some("verify") => {
            let mut bad = 0;
            let names = download::models::MODELS.iter().map(|art| (art.name, art.sha256));
            let extracted = download::models::EXTRACTED.iter().copied();
            for (name, sha) in names.chain(extracted) {
                let p = paths::models_dir().join(name);
                match download::verify(&p, sha) {
                    Ok(true) => println!("  ✓ {name}"),
                    _ => {
                        println!("  ✗ {name} (missing or hash mismatch)");
                        bad += 1;
                    }
                }
            }
            if bad > 0 {
                std::process::exit(1);
            }
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown download target: {other} (try `talk download models`)");
            std::process::exit(2);
        }
    }
}

#[cfg(not(feature = "download"))]
fn handle_download(target: Option<&str>) -> std::io::Result<()> {
    match target {
        None => list_installed(),
        Some(_) => {
            eprintln!("this build has no download support — rebuild with `--features download`");
            std::process::exit(2);
        }
    }
}

fn print_thread(base: &Path, question: Option<&str>) {
    match question {
        Some(q) => {
            let direct = base.join(format!("{}.md", talk_core::slug::derive_slug(q)));
            // Trust the direct hit only if it's actually THIS question's file;
            // a different BYO question may own the un-suffixed slug (the write
            // path suffixes collisions, so the read path must verify, not assume).
            let direct_ok = std::fs::read_to_string(&direct).ok()
                .and_then(|t| talk_core::frontmatter::Frontmatter::parse(&t).map(|(fm, _)| fm.question == q))
                .unwrap_or(false);
            let path = if direct_ok { Some(direct) } else { find_by_question(base, q) };
            match path.and_then(|p| std::fs::read_to_string(p).ok()) {
                Some(text) => print!("{}", text),
                None => println!("No thread yet for \"{}\".", q),
            }
        }
        None => {
            let mut rows = existing_threads(base);
            if rows.is_empty() {
                println!("No threads yet — run `talk` to start one.");
                return;
            }
            rows.sort_by(|a, b| b.last.cmp(&a.last)); // last-date desc (ISO dates sort lexically)
            for t in rows {
                println!("{} · {} {} · {}", t.slug, t.entries,
                    if t.entries == 1 { "entry" } else { "entries" }, t.last);
            }
        }
    }
}

/// One thread file's frontmatter plus its on-disk path, as scanned from the base
/// dir.
struct ThreadRow {
    path: PathBuf,
    slug: String,
    entries: u32,
    last: String,
    question: String,
    /// Read only by the live BYO near-match path (listen builds).
    #[cfg_attr(not(feature = "listen"), allow(dead_code))]
    pack: String,
}

/// Scan the base dir for every thread file (frontmatter-bearing `.md`), parsing
/// each into a `ThreadRow`. The `.md` filter excludes the `.raw/` subdir (a dir
/// has no extension); journal date files have no frontmatter and drop out. Shared
/// by `print_thread`'s list view and `find_by_question`.
fn existing_threads(base: &Path) -> Vec<ThreadRow> {
    std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let Some((fm, _)) = talk_core::frontmatter::Frontmatter::parse(&text) else {
                // Journal date files legitimately have no frontmatter; anything
                // else is a corrupt thread that would otherwise silently vanish
                // from the list (and fork on the next near-match).
                let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                if !is_journal_date_slug(stem) {
                    eprintln!("warning: skipping {} — unparseable frontmatter", p.display());
                }
                return None;
            };
            Some(ThreadRow { path: p, slug: fm.slug, entries: fm.entries, last: fm.last, question: fm.question, pack: fm.pack })
        })
        .collect()
}

/// Find a reflect file whose frontmatter question matches `q` (covers spine
/// id-named and collision-suffixed files). Returns its on-disk path.
fn find_by_question(base: &Path, q: &str) -> Option<PathBuf> {
    existing_threads(base).into_iter().find(|t| t.question == q).map(|t| t.path)
}

fn load_config() -> std::io::Result<config::Config> {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    Ok(config::Config::load(&text).unwrap_or_default())
}

fn state_path(base: &Path) -> PathBuf {
    base.join(".state.json") // dot-prefixed so vault sync / indexing skip it
}

/// The first-run privacy note (spec §8) — printed to STDOUT once, since it's a
/// feature of talk (everything stays local), not an error.
const DISCLOSURE: &str = "talk keeps everything local — your words land only in ~/talk on this machine.\n\
one honest note: the raw transcript is plaintext. if ~/talk lives in a\n\
cloud-synced folder (iCloud, Dropbox), that cloud sees your words.\n\
`keep_raw = false` stores only the cleaned text; `talk unburden` keeps\n\
nothing at all. clipboard recovery, if you ever use it, passes through the\n\
system clipboard. scrollback and OS swap are beyond any app's reach.";

/// Print the first-run disclosure once, then record it in state. Must be called
/// only when a non-ephemeral session actually proceeds, and BEFORE any other
/// `State` load (reflect_choice's included), so reflect's later save doesn't
/// clobber the flag with a pre-disclosure copy. Ephemeral never discloses.
fn disclose_once(base: &Path) -> std::io::Result<()> {
    let mut st = state::State::load(&std::fs::read_to_string(state_path(base)).unwrap_or_default());
    if !st.disclosed {
        println!("{DISCLOSURE}");
        st.disclosed = true;
        paths::write_private(&state_path(base), &st.save())?;
    }
    Ok(())
}

/// True if a file at `path` exists AND is not this question's own reflect file —
/// either it stores a DIFFERENT question, or it has no frontmatter at all (e.g.
/// a journal date file the BYO slug happens to collide with).
fn slug_taken_by_other(path: &Path, q: &str) -> bool {
    match std::fs::read_to_string(path) {
        Ok(t) => match talk_core::frontmatter::Frontmatter::parse(&t) {
            Some((fm, _)) => fm.question != q,
            None => true,
        },
        Err(_) => false,
    }
}

/// True if `slug` is shaped like a journal filename (`YYYY-MM-DD`), so a BYO
/// question never claims a slug in the journal date-file namespace. Hand-rolled
/// (len 10: four digits, '-', two digits, '-', two digits) to avoid a regex dep.
fn is_journal_date_slug(slug: &str) -> bool {
    let b = slug.as_bytes();
    b.len() == 10
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

fn require_text(from: &Option<String>) -> String {
    match from {
        Some(t) if !t.trim().is_empty() => t.clone(),
        _ => {
            eprintln!("no --from-text given and this build has no microphone support — rebuild with --features listen, or pass --from-text <text>");
            std::process::exit(2);
        }
    }
}

fn hour_of(hm: &str) -> u32 {
    hm.split(':').next().and_then(|h| h.parse().ok()).unwrap_or(12)
}

// A real (UTC) system clock so files aren't epoch-dated; tests pass --date/--time
// for determinism.
fn system_date() -> String {
    let (y, m, d) = streak::civil_from_days((unix_secs() / 86_400) as i64);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_date_slug_accepts_only_yyyy_mm_dd_shapes() {
        assert!(is_journal_date_slug("2026-06-09"));
        assert!(!is_journal_date_slug("2026-6-9"));
        assert!(!is_journal_date_slug("abcd-ef-gh"));
        assert!(!is_journal_date_slug("2026-06-0"));
        assert!(!is_journal_date_slug("abcdefghij"));
    }
}
