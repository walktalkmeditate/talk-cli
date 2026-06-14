//! The pure talk-cli engine. No I/O, no audio, no ML.

pub mod cleanup;
pub mod clock;
pub mod close;
pub mod entry;
pub mod eval;
pub mod format;
pub mod frontmatter;
pub mod lexicon;
pub mod matchq;
pub mod pairing;
pub mod palette;
pub mod questions;
pub mod render_model;
pub mod selection;
pub mod settle;
pub mod slug;
#[cfg(test)]
mod test_support;
