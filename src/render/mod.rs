#![allow(dead_code)] // Wired into the live session loop in Plan-2 Task 11.

use std::io::{self, Write};
use crossterm::{cursor, execute, queue, style, terminal};
use talk_core::palette::{Palette, Rgb, Tone};
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

/// (foreground, intensity) for a tone — pure, no I/O.
fn style_for(tone: Tone) -> (style::Color, style::Attribute) {
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

/// The tone each line paints in. `Question` shares the mid `dim` tone with the live
/// edge — readable but a step below the settled words, and off the dimmest `edge`.
fn tone_for(kind: LineKind, palette: Palette) -> Tone {
    match kind {
        LineKind::Settled => palette.core,
        LineKind::Question | LineKind::Edge => palette.dim,
        LineKind::Chrome => palette.edge,
    }
}

/// Paint a full frame. Clears, then writes each composed line in its tone.
pub fn paint(view: &View, palette: Palette) -> io::Result<()> {
    let mut out = io::stdout();
    queue!(out, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))?;
    for (line, kind) in compose(view) {
        apply_tone(&mut out, tone_for(kind, palette))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::palette::{palette, Theme};

    #[test]
    fn style_for_maps_each_tone_to_crossterm() {
        assert_eq!(
            style_for(Tone::Color(Rgb::new(210, 146, 118))),
            (style::Color::Rgb { r: 210, g: 146, b: 118 }, style::Attribute::NormalIntensity)
        );
        assert_eq!(style_for(Tone::Terminal), (style::Color::Reset, style::Attribute::NormalIntensity));
        assert_eq!(style_for(Tone::TerminalFaint), (style::Color::Reset, style::Attribute::Dim));
    }

    #[test]
    fn question_paints_at_the_mid_tone_not_the_dimmest() {
        let p = palette(Theme::Rust);
        assert_eq!(tone_for(LineKind::Settled, p), p.core);
        assert_eq!(tone_for(LineKind::Question, p), p.dim);
        assert_eq!(tone_for(LineKind::Edge, p), p.dim);
        assert_eq!(tone_for(LineKind::Chrome, p), p.edge);
        assert_ne!(tone_for(LineKind::Question, p), tone_for(LineKind::Chrome, p));
    }
}
