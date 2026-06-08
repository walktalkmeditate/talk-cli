#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb { pub r: u8, pub g: u8, pub b: u8 }

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self { Rgb { r, g, b } }
}

/// The talk pillar base tone — `rust`, from pilgrim-ios rust.colorset (light).
/// Plan 1 needs only the base constant; the `palette()` synthesis (edge/dim
/// variants + season/time tinting) is deferred to Plan 2, where the renderer
/// that consumes it is built (YAGNI — no Plan-1 consumer exists).
pub const RUST: Rgb = Rgb::new(160, 99, 75);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_is_the_talk_base_tone() {
        assert_eq!(RUST, Rgb::new(160, 99, 75));
    }
}
