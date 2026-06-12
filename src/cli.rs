use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "talk", version, about = "The terminal that listens.")]
pub struct Cli {
    /// A bring-your-own question (bare `talk "..."`).
    pub question: Option<String>,

    /// Drive the pipeline from text instead of a mic (Plan-1 testing seam).
    /// `global = true` so it parses AFTER a subcommand (e.g. `talk unburden --from-text ...`).
    #[arg(long, global = true)]
    pub from_text: Option<String>,

    /// Today's date override (tests pass this; real runs use the system date).
    #[arg(long, hide = true, global = true)]
    pub date: Option<String>,

    /// Time override (HH:MM).
    #[arg(long, hide = true, global = true)]
    pub time: Option<String>,

    /// Color palette: rust (default) · high-contrast · mono (terminal-native).
    #[arg(long, global = true)]
    pub palette: Option<PaletteArg>,

    /// Override cleanup intensity for this run: none · light · medium · high.
    #[arg(long, global = true)]
    pub clean: Option<CleanArg>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Reflect — asks a question, then listens (default).
    Reflect,
    /// Freeform daily journal — no prompt.
    Journal,
    /// Ephemeral — listens, shows, keeps nothing.
    Unburden,
    /// Alias for unburden.
    Vent,
    /// Print a question's accumulated file (no arg → list).
    Thread { question: Option<String> },
    /// Show your reflection streak.
    Streak,
    /// Config helpers.
    Config { action: Option<String> },
    /// Download models (and later, question packs). `talk download models`.
    Download { target: Option<String> },
}

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

/// The `--clean` flag's accepted values: none · light · medium · high.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum CleanArg { None, Light, Medium, High }

impl From<CleanArg> for talk_core::cleanup::Level {
    fn from(a: CleanArg) -> Self {
        use talk_core::cleanup::Level;
        match a {
            CleanArg::None => Level::None,
            CleanArg::Light => Level::Light,
            CleanArg::Medium => Level::Medium,
            CleanArg::High => Level::High,
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
