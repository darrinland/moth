//! The fractal flame generator (the Apophysis lineage): an iterated function system
//! with nonlinear "variation" functions, plotted by the chaos game.

pub mod genome;
pub mod io;
pub mod palette;
pub mod random;
pub mod render;
pub mod variations;

pub use genome::{Affine, Camera, Genome, Xform};
pub use palette::Palette;
pub use random::RandomSettings;
pub use variations::Variation;

use crate::generator::{Generator, Image, RenderSpec};

pub struct FlameGenerator;

impl Generator for FlameGenerator {
    type Genome = Genome;
    type Settings = RandomSettings;

    const ID: &'static str = "flame";

    fn randomize(&self, settings: &RandomSettings, seed: u64) -> Genome {
        random::randomize(settings, seed)
    }

    fn mutate(&self, genome: &Genome, settings: &RandomSettings, amount: f32, seed: u64) -> Genome {
        random::mutate(genome, settings, amount, seed)
    }

    fn render(&self, genome: &Genome, spec: &RenderSpec) -> Image {
        render::render(genome, spec)
    }
}
