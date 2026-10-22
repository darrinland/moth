//! Moth core: generators, genome models, renderers and file formats.
//!
//! Everything here is UI-independent. A *generator* (the flame is the first one)
//! turns a serializable *genome* into an [`Image`]; the gallery, randomizer,
//! save files and export pipeline are all written against [`Generator`].

pub mod export;
pub mod flame;
pub mod generator;
pub mod output;

pub use generator::{Generator, Image, RenderSpec};
