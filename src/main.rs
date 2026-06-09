mod cli;
mod config;
#[cfg(feature = "download")]
mod download;
mod keymap;
#[cfg(feature = "listen")]
mod listen;
mod paths;
mod render;
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
        Some(Command::Config { action }) => return handle_config(action.as_deref()),
        Some(Command::Thread { question }) => print_thread(&base, question.as_deref()),
        Some(Command::Streak) => println!("streak: (Plan 4)"),
        Some(Command::Download { target }) => return handle_download(target.as_deref()),
        Some(Command::Reflect) => reflect(&base, &args.question, &date, &time, &require_text(&args.from_text), &cfg)?,
        // Bare `talk`: honor config.default_mode (journal) unless a BYO question was given.
        _ => {
            if args.question.is_none() && cfg.default_mode == "journal" {
                let text = require_text(&args.from_text);
                run_and_report(&base, Target::Journal, &date, &time, &text, cfg.keep_raw, false)?;
            } else {
                reflect(&base, &args.question, &date, &time, &require_text(&args.from_text), &cfg)?;
            }
        }
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
                slug_taken_by_other(&base.join(format!("{}.md", s)), q)
            });
            (slug.clone(), q.clone(), slug, "byo".to_string(), "self".to_string())
        }
        None => {
            let spine = talk_core::questions::Pack::from_toml(SPINE_TOML).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bundled spine.toml invalid: {e}"))
            })?;
            let chosen = match talk_core::selection::select(&spine, &st.selection_state(), hour_of(time)) {
                Some(q) => q.clone(),
                None => return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData, "spine pack has no questions")),
            };
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

/// `talk download models` — fetch + SHA-256-verify the model artifacts (behind
/// the `download` feature). Without the feature, this build can't fetch.
#[cfg(feature = "download")]
fn handle_download(target: Option<&str>) -> std::io::Result<()> {
    match target {
        Some("models") | None => {
            for art in download::models::MODELS {
                println!("fetching {} …", art.name);
                download::fetch(art, &paths::models_dir())
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                println!("  ✓ {}", art.name);
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
fn handle_download(_target: Option<&str>) -> std::io::Result<()> {
    eprintln!("this build has no download support — rebuild with `--features download`");
    std::process::exit(2);
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
        None => println!("(thread list — Plan 4)"),
    }
}

/// Scan the base dir for a reflect file whose frontmatter question matches `q`
/// (covers spine id-named and collision-suffixed files).
fn find_by_question(base: &Path, q: &str) -> Option<PathBuf> {
    std::fs::read_dir(base).ok()?.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .find(|p| std::fs::read_to_string(p).ok()
            .and_then(|t| talk_core::frontmatter::Frontmatter::parse(&t).map(|(fm, _)| fm.question == q))
            .unwrap_or(false))
}

fn load_config() -> std::io::Result<config::Config> {
    let text = std::fs::read_to_string(config_path()).unwrap_or_default();
    Ok(config::Config::load(&text).unwrap_or_default())
}

fn state_path(base: &Path) -> PathBuf {
    base.join(".state.json") // dot-prefixed so vault sync / indexing skip it
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
