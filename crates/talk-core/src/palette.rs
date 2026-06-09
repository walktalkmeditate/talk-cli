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
