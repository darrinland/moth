//! Rendering a flame straight to an image file, a band at a time so memory stays bounded.
//! Shared by the CLI and the app.

use std::path::Path;

use crate::flame::render::render_strips;
use crate::flame::{Genome, io};
use crate::generator::RenderSpec;
use crate::output::{self, BitDepth, OutputError};

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub spec: RenderSpec,
    pub depth: BitDepth,
    /// 1-100; JPEG only.
    pub jpeg_quality: u8,
    /// Largest band, in oversampled pixels. Huge images take longer in more bands.
    pub strip_pixels: usize,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            spec: RenderSpec {
                oversample: 2,
                ..RenderSpec::default()
            },
            depth: BitDepth::Eight,
            jpeg_quality: 92,
            strip_pixels: 16_000_000,
        }
    }
}

/// Renders `genome` to `path` as PNG or JPEG (by extension), embedding the recipe so the
/// file can be opened later and re-rendered at higher quality.
pub fn export(genome: &Genome, opts: &ExportOptions, path: &Path) -> Result<(), OutputError> {
    let spec = &opts.spec;
    let recipe = io::to_xml(genome);
    match output::extension(path).as_str() {
        "png" => {
            let mut png = output::PngStream::create(
                path,
                spec.width,
                spec.height,
                opts.depth,
                Some(&recipe),
            )?;
            render_strips(genome, spec, opts.strip_pixels, |rows| {
                png.write_rows(&rows)
            })?;
            png.finish()
        }
        "jpg" | "jpeg" => {
            let mut rgb = Vec::with_capacity(spec.width as usize * spec.height as usize * 3);
            render_strips(
                genome,
                spec,
                opts.strip_pixels,
                |rows| -> Result<(), OutputError> {
                    output::append_rgb8(&rows, genome.background, &mut rgb);
                    Ok(())
                },
            )?;
            output::write_jpeg(
                path,
                spec.width,
                spec.height,
                &rgb,
                opts.jpeg_quality,
                Some(&recipe),
            )
        }
        other => Err(OutputError::Format(format!(
            "unsupported output format {other:?}: use .png or .jpg"
        ))),
    }
}

/// Loads every flame from a `.flame` file, or the one embedded in a PNG/JPEG.
/// Unsupported items are returned as warnings next to each flame.
pub fn load_flames(path: &Path) -> Result<Vec<io::Imported>, LoadError> {
    let xml = match output::extension(path).as_str() {
        "png" | "jpg" | "jpeg" => output::read_recipe(path)?.ok_or(LoadError::NoRecipe)?,
        _ => std::fs::read_to_string(path)?,
    };
    Ok(io::from_xml(&xml)?)
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("this image has no embedded Moth recipe")]
    NoRecipe,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Output(#[from] OutputError),
    #[error(transparent)]
    Flame(#[from] io::FlameError),
}
