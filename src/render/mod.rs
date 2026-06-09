#![allow(dead_code)] // Wired into the live session loop in Plan-2 Task 11.

use std::io::{self, Write};
use crossterm::{cursor, execute, queue, style, terminal};
use talk_core::palette::{palette, Rgb};
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

/// Paint a full frame. Clears, then writes each composed line in its tone:
/// Settled → core (bright), Edge → dim, Chrome → edge (dimmest).
pub fn paint(view: &View) -> io::Result<()> {
    let p = palette();
    let mut out = io::stdout();
    queue!(out, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
    for (line, kind) in compose(view) {
        let tone = match kind {
            LineKind::Settled => p.core,
            LineKind::Edge => p.dim,
            LineKind::Chrome => p.edge,
        };
        queue!(out, style::SetForegroundColor(rust(tone)), style::Print(line), cursor::MoveToNextLine(1))?;
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
