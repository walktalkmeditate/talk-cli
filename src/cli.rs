use clap::{Parser, Subcommand};

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
