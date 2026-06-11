#![allow(dead_code)] // Wired into the live session loop in Plan-2 Task 11.

use std::io::{self, Write};
use crossterm::{cursor, execute, queue, style, terminal};
use talk_core::palette::{palette, Rgb, Theme, Tone};
use talk_core::render_model::{compose, LineKind, View};

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
            LineKind::Question => p.dim,
            LineKind::Edge => p.dim,
            LineKind::Chrome => p.edge,
        };
        apply_tone(&mut out, tone)?;
        queue!(out, style::Print(line), cursor::MoveToNextLine(1))?;
    }
    queue!(out, style::ResetColor, style::SetAttribute(style::Attribute::Reset))?;
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
