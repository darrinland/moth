/// How to render a genome: output size and quality, independent of the genome itself,
/// so one saved recipe can be re-rendered at any resolution.
#[derive(Clone, Debug)]
pub struct RenderSpec {
    pub width: u32,
    pub height: u32,
    /// Average samples per pixel. Higher is smoother and slower.
    pub quality: f32,
    /// Leave the background transparent instead of filling it.
    pub transparent: bool,
    /// Seeds the chaos game; the same seed gives the same image.
    pub seed: u64,
    /// Renders at this many times the resolution per axis, then averages down.
    pub oversample: u32,
    /// Strength of the adaptive density filter: 0 is off, 1 is the default.
    pub denoise: f32,
}

impl Default for RenderSpec {
    fn default() -> Self {
        Self {
            width: 1024,
            height: 768,
            quality: 200.0,
            transparent: false,
            seed: 0,
            oversample: 1,
            denoise: 1.0,
        }
    }
}

/// A finished, display-referred image: straight (unpremultiplied) RGBA, 0..=1 floats.
/// Encoders choose the bit depth, so 8-bit, 16-bit and HDR outputs share one pipeline.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<f32>,
}

/// A source of images that can be randomized, evolved and rendered.
///
/// The flame is one implementation. Anything else that can produce an image from
/// a small serializable recipe (attractors, escape-time fractals, ...) plugs in here.
pub trait Generator {
    /// The recipe: everything needed to reproduce an image.
    type Genome: Clone + Send + Sync;
    /// User-facing knobs that steer [`Generator::randomize`] and [`Generator::mutate`].
    type Settings: Default;

    /// Stable identifier written into save files.
    const ID: &'static str;

    fn randomize(&self, settings: &Self::Settings, seed: u64) -> Self::Genome;

    /// `amount` runs from 0 (no change) to 1 (almost unrelated).
    fn mutate(
        &self,
        genome: &Self::Genome,
        settings: &Self::Settings,
        amount: f32,
        seed: u64,
    ) -> Self::Genome;

    fn render(&self, genome: &Self::Genome, spec: &RenderSpec) -> Image;
}
