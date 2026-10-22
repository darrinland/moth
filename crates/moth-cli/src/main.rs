use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use moth_core::export::{ExportOptions, export, load_flames};
use moth_core::flame::{FlameGenerator, Genome, RandomSettings, io};
use moth_core::output::BitDepth;
use moth_core::{Generator, RenderSpec};

#[derive(Parser)]
#[command(name = "moth", version, about = "Fractal flame explorer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate random flames, writing a PNG and a .flame for each.
    Random {
        /// Output directory.
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
        #[arg(short = 'n', long, default_value_t = 10)]
        count: u64,
        /// First seed; flame i uses seed + i.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[command(flatten)]
        render: RenderArgs,
        /// Smallest and largest number of transforms, e.g. 2-5.
        #[arg(long, value_parser = parse_range, default_value = "2-5")]
        xforms: (usize, usize),
        /// 0..1: how wild the transforms get.
        #[arg(long, default_value_t = 0.5)]
        wildness: f64,
    },
    /// Evolve a saved flame into several variations, writing a PNG and a .flame for each.
    Mutate {
        input: PathBuf,
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
        #[arg(short = 'n', long, default_value_t = 10)]
        count: u64,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// 0..1: how far each variation drifts from the original.
        #[arg(long, default_value_t = 0.3)]
        amount: f32,
        #[arg(long, default_value_t = 0)]
        index: usize,
        #[command(flatten)]
        render: RenderArgs,
    },
    /// Render a .flame file, or a PNG/JPEG that carries an embedded recipe.
    Render {
        input: PathBuf,
        /// Output path: .png or .jpg.
        #[arg(short, long)]
        out: PathBuf,
        /// Which flame to render when the file holds several (0-based).
        #[arg(long, default_value_t = 0)]
        index: usize,
        #[command(flatten)]
        render: RenderArgs,
    },
}

#[derive(clap::Args)]
struct RenderArgs {
    /// Output size, e.g. 1920x1080.
    #[arg(long, value_parser = parse_size, default_value = "1024x768")]
    size: (u32, u32),
    /// Samples per pixel.
    #[arg(short, long, default_value_t = 100.0)]
    quality: f32,
    #[arg(long)]
    transparent: bool,
    /// JPEG quality, 1-100.
    #[arg(long, default_value_t = 92)]
    jpeg_quality: u8,
    /// Write 16-bit PNGs.
    #[arg(long)]
    deep: bool,
    /// Render at this many times the resolution, then average down (smoother edges).
    #[arg(long, default_value_t = 2)]
    oversample: u32,
    /// Strength of the noise-reducing density filter; 0 turns it off.
    #[arg(long, default_value_t = 1.0)]
    denoise: f32,
    /// Render in bands of at most this many (oversampled) pixels to bound memory. Huge
    /// images take proportionally longer, since every band replays the same samples.
    #[arg(long, default_value_t = 16_000_000)]
    strip_pixels: usize,
    /// Chaos-game seed for the render itself.
    #[arg(long, default_value_t = 0)]
    render_seed: u64,
}

impl RenderArgs {
    fn spec(&self) -> RenderSpec {
        RenderSpec {
            width: self.size.0,
            height: self.size.1,
            quality: self.quality,
            transparent: self.transparent,
            seed: self.render_seed,
            oversample: self.oversample.max(1),
            denoise: self.denoise,
        }
    }

    fn depth(&self) -> BitDepth {
        if self.deep {
            BitDepth::Sixteen
        } else {
            BitDepth::Eight
        }
    }
}

impl RenderArgs {
    fn options(&self) -> ExportOptions {
        ExportOptions {
            spec: self.spec(),
            depth: self.depth(),
            jpeg_quality: self.jpeg_quality,
            strip_pixels: self.strip_pixels,
        }
    }

    fn render_image(&self, genome: &Genome, path: &Path) -> Result<()> {
        export(genome, &self.options(), path)?;
        Ok(())
    }
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once(['x', 'X']).ok_or("expected WIDTHxHEIGHT")?;
    let (w, h) = (
        w.parse::<u32>().map_err(|e| e.to_string())?,
        h.parse::<u32>().map_err(|e| e.to_string())?,
    );
    if w == 0 || h == 0 {
        return Err("size must be non-zero".into());
    }
    Ok((w, h))
}

fn parse_range(s: &str) -> Result<(usize, usize), String> {
    let (a, b) = s.split_once('-').ok_or("expected MIN-MAX")?;
    let (a, b) = (
        a.parse::<usize>().map_err(|e| e.to_string())?,
        b.parse::<usize>().map_err(|e| e.to_string())?,
    );
    if a == 0 || a > b {
        return Err("need 1 <= MIN <= MAX".into());
    }
    Ok((a, b))
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Random {
            out,
            count,
            seed,
            render,
            xforms,
            wildness,
        } => random(&out, count, seed, &render, xforms, wildness),
        Command::Mutate {
            input,
            out,
            count,
            seed,
            amount,
            index,
            render,
        } => mutate(&input, &out, count, seed, amount, index, &render),
        Command::Render {
            input,
            out,
            index,
            render,
        } => render_file(&input, &out, index, &render),
    }
}

fn random(
    out: &Path,
    count: u64,
    seed: u64,
    render: &RenderArgs,
    xforms: (usize, usize),
    wildness: f64,
) -> Result<()> {
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let settings = RandomSettings {
        xforms,
        wildness,
        size: render.size,
        ..Default::default()
    };
    let gen_ = FlameGenerator;
    for i in 0..count {
        let s = seed + i;
        let mut genome = gen_.randomize(&settings, s);
        genome.name = format!("moth-{s}");
        let xml = io::to_xml(&genome);
        fs::write(out.join(format!("moth-{s}.flame")), &xml)?;
        render.render_image(&genome, &out.join(format!("moth-{s}.png")))?;
        println!("moth-{s}");
    }
    Ok(())
}

fn mutate(
    input: &Path,
    out: &Path,
    count: u64,
    seed: u64,
    amount: f32,
    index: usize,
    render: &RenderArgs,
) -> Result<()> {
    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let parent = load(input, index)?;
    let settings = RandomSettings {
        size: parent.camera.size,
        ..Default::default()
    };
    let gen_ = FlameGenerator;
    let stem = if parent.name.is_empty() {
        "mutant".to_string()
    } else {
        parent.name.clone()
    };
    for i in 0..count {
        let mut child = gen_.mutate(&parent, &settings, amount, seed + i);
        child.name = format!("{stem}-m{}", seed + i);
        let xml = io::to_xml(&child);
        fs::write(out.join(format!("{}.flame", child.name)), &xml)?;
        render.render_image(&child, &out.join(format!("{}.png", child.name)))?;
        println!("{}", child.name);
    }
    Ok(())
}

fn load(input: &Path, index: usize) -> Result<Genome> {
    let mut flames = load_flames(input).with_context(|| format!("reading {}", input.display()))?;
    if index >= flames.len() {
        bail!(
            "index {index} out of range: file has {} flame(s)",
            flames.len()
        );
    }
    let imported = flames.swap_remove(index);
    if !imported.ignored.is_empty() {
        eprintln!(
            "warning: ignored unsupported items: {}",
            imported.ignored.join(", ")
        );
    }
    Ok(imported.genome)
}

fn render_file(input: &Path, out: &Path, index: usize, render: &RenderArgs) -> Result<()> {
    let genome = load(input, index)?;
    render.render_image(&genome, out)?;
    println!("wrote {}", out.display());
    Ok(())
}
