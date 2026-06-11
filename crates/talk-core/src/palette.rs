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
