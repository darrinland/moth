//! CPU chaos-game renderer: iterate, accumulate a density histogram, tone-map.

use rand::{Rng, RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use super::genome::{Affine, Genome, Xform};
use super::variations::Variation;
use crate::generator::{Image, RenderSpec};

const BURN_IN: usize = 20;
/// Fixed work split, so output does not depend on how many cores ran it.
const CHUNKS: usize = 256;
/// Color is accumulated in 0..=COLOR_SCALE fixed point.
const COLOR_SCALE: f32 = 255.0;

struct CompiledXform {
    cumulative: f64,
    color: f64,
    color_speed: f64,
    affine: Affine,
    post: Option<Affine>,
    variations: Vec<(Variation, f64)>,
}

impl CompiledXform {
    fn new(x: &Xform, cumulative: f64) -> Self {
        Self {
            cumulative,
            color: x.color,
            color_speed: x.color_speed,
            affine: x.affine,
            post: (!x.post.is_identity()).then_some(x.post),
            variations: x.variations.clone(),
        }
    }

    #[inline]
    fn step<R: Rng + ?Sized>(&self, x: f64, y: f64, rng: &mut R) -> (f64, f64) {
        let (px, py) = self.affine.apply(x, y);
        let (mut ox, mut oy) = (0.0, 0.0);
        for &(v, w) in &self.variations {
            let (vx, vy) = v.apply(px, py, &self.affine, rng);
            ox += w * vx;
            oy += w * vy;
        }
        match &self.post {
            Some(p) => p.apply(ox, oy),
            None => (ox, oy),
        }
    }
}

/// The iteration function system, ready to run.
pub(crate) struct Chaos {
    xforms: Vec<CompiledXform>,
    total_weight: f64,
    final_xform: Option<CompiledXform>,
}

pub(crate) struct Walker {
    x: f64,
    y: f64,
    c: f64,
}

/// A point to plot: position and palette coordinate, after the final transform.
pub(crate) struct Plot {
    pub x: f64,
    pub y: f64,
    pub c: f64,
}

impl Chaos {
    pub(crate) fn new(g: &Genome) -> Self {
        let mut total = 0.0;
        let xforms = g
            .xforms
            .iter()
            .filter(|x| x.weight > 0.0)
            .map(|x| {
                total += x.weight;
                CompiledXform::new(x, total)
            })
            .collect();
        Self {
            xforms,
            total_weight: total,
            final_xform: g.final_xform.as_ref().map(|x| CompiledXform::new(x, 0.0)),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.xforms.is_empty()
    }

    pub(crate) fn walker<R: Rng + ?Sized>(&self, rng: &mut R) -> Walker {
        Walker {
            x: rng.random_range(-1.0..1.0),
            y: rng.random_range(-1.0..1.0),
            c: rng.random(),
        }
    }

    /// Advances one iteration. `None` means the point escaped and was re-seeded.
    #[inline]
    pub(crate) fn step<R: Rng + ?Sized>(&self, w: &mut Walker, rng: &mut R) -> Option<Plot> {
        let pick = rng.random::<f64>() * self.total_weight;
        let xf = self
            .xforms
            .iter()
            .find(|x| pick < x.cumulative)
            .unwrap_or(&self.xforms[self.xforms.len() - 1]);
        let (x, y) = xf.step(w.x, w.y, rng);
        if !(x.is_finite() && y.is_finite()) || x.abs() > 1e9 || y.abs() > 1e9 {
            *w = self.walker(rng);
            return None;
        }
        w.x = x;
        w.y = y;
        w.c += (xf.color - w.c) * xf.color_speed;
        match &self.final_xform {
            Some(f) => {
                let (fx, fy) = f.step(x, y, rng);
                let fc = w.c + (f.color - w.c) * f.color_speed;
                (fx.is_finite() && fy.is_finite()).then_some(Plot {
                    x: fx,
                    y: fy,
                    c: fc,
                })
            }
            None => Some(Plot { x, y, c: w.c }),
        }
    }

    /// Runs the burn-in and returns a walker that is on the attractor.
    pub(crate) fn warm_walker<R: Rng + ?Sized>(&self, rng: &mut R) -> Walker {
        let mut w = self.walker(rng);
        for _ in 0..BURN_IN {
            self.step(&mut w, rng);
        }
        w
    }
}

struct View {
    cx: f64,
    cy: f64,
    ppu: f64,
    cos: f64,
    sin: f64,
    half_w: f64,
    half_h: f64,
    w: usize,
    h: usize,
    /// Only rows `row0..row1` are kept (a horizontal strip of the full frame).
    row0: usize,
    row1: usize,
}

impl View {
    fn new(g: &Genome, spec: &RenderSpec, row0: usize, row1: usize) -> Self {
        let (sin, cos) = g.camera.rotate.to_radians().sin_cos();
        Self {
            cx: g.camera.center.0,
            cy: g.camera.center.1,
            ppu: g.camera.scale * spec.width as f64 / g.camera.size.0.max(1) as f64,
            cos,
            sin,
            half_w: spec.width as f64 / 2.0,
            half_h: spec.height as f64 / 2.0,
            w: spec.width as usize,
            h: spec.height as usize,
            row0,
            row1,
        }
    }

    #[inline]
    fn pixel(&self, x: f64, y: f64) -> Option<usize> {
        let (dx, dy) = (x - self.cx, y - self.cy);
        let px = (dx * self.cos - dy * self.sin) * self.ppu + self.half_w;
        let py = (dx * self.sin + dy * self.cos) * self.ppu + self.half_h;
        if px >= 0.0 && py >= 0.0 && px < self.w as f64 && py < self.h as f64 {
            let row = py as usize;
            (row >= self.row0 && row < self.row1).then(|| (row - self.row0) * self.w + px as usize)
        } else {
            None
        }
    }
}

pub fn render(g: &Genome, spec: &RenderSpec) -> Image {
    render_rows(g, spec, 0, spec.height as usize)
}

/// Raw per-pixel `[count, r, g, b]` sums of the chaos game, before any filtering or tone
/// mapping, at `spec`'s size with no oversampling. Other renderers (the GPU) are checked
/// against this.
pub fn raw_histogram(g: &Genome, spec: &RenderSpec) -> Vec<f32> {
    let total = spec.quality.max(0.0) as f64 * (spec.width * spec.height) as f64;
    accumulate(g, spec, total, 0, spec.height as usize)
}

/// Renders output rows `y0..y1` of the full frame. Strips of one frame match the
/// full render, so huge images can be produced a band at a time.
pub fn render_rows(g: &Genome, spec: &RenderSpec, y0: usize, y1: usize) -> Image {
    let os = spec.oversample.max(1) as usize;
    let (w, h) = (spec.width as usize, spec.height as usize);
    let y1 = y1.min(h);
    if Chaos::new(g).is_empty() || w == 0 || h == 0 || y0 >= y1 {
        return blank_rows(g, spec, y1.saturating_sub(y0));
    }
    let (hw, hh) = (w * os, h * os);
    let hi = RenderSpec {
        width: hw as u32,
        height: hh as u32,
        ..spec.clone()
    };
    let max_sigma = 2.5 * spec.denoise * os as f32;
    // Rows beyond the strip that the filter can pull light from.
    let halo = if spec.denoise > 0.0 {
        (max_sigma * 3.0).ceil() as usize + 2
    } else {
        0
    };
    let (r0, r1) = ((y0 * os).saturating_sub(halo), (y1 * os + halo).min(hh));

    // Total samples follow the output size, so oversampling costs memory, not time.
    let mut density = accumulate(
        g,
        &hi,
        spec.quality.max(0.0) as f64 * (w * h) as f64,
        r0,
        r1,
    );
    if spec.denoise > 0.0 {
        density = density_filter(&density, hw, r1 - r0, max_sigma);
    }
    let spp = (spec.quality / (os * os) as f32).max(1e-3);
    let mut premul = tone_map(g, &density, spp, hw, r1 - r0, spec.transparent);
    drop(density);
    let (c0, c1) = ((y0 * os - r0) * hw * 4, (y1 * os - r0) * hw * 4);
    premul.truncate(c1);
    premul.drain(..c0);
    downsample(premul, hw, (y1 - y0) * os, os, spec.transparent)
}

/// Renders the frame in horizontal strips of at most `max_strip_pixels` (oversampled)
/// pixels, handing each finished strip to `sink` top to bottom. Memory stays bounded;
/// time grows with the strip count because every strip replays the same sample stream.
pub fn render_strips<E>(
    g: &Genome,
    spec: &RenderSpec,
    max_strip_pixels: usize,
    mut sink: impl FnMut(Image) -> Result<(), E>,
) -> Result<(), E> {
    let os = spec.oversample.max(1) as usize;
    let rows = (max_strip_pixels / (spec.width as usize * os * os).max(1))
        .clamp(1, spec.height.max(1) as usize);
    let mut y = 0;
    while y < spec.height as usize {
        let y1 = (y + rows).min(spec.height as usize);
        sink(render_rows(g, spec, y, y1))?;
        y = y1;
    }
    Ok(())
}

/// Runs the chaos game and returns per-pixel `[count, r, g, b]` sums.
fn accumulate(g: &Genome, spec: &RenderSpec, total: f64, row0: usize, row1: usize) -> Vec<f32> {
    let chaos = Chaos::new(g);
    let view = View::new(g, spec, row0, row1);
    let palette: Vec<[f32; 3]> = (0..256)
        .map(|i| g.palette.sample(i as f64 / 255.0))
        .collect();
    let pixels = view.w * (row1 - row0);

    // Integer adds commute, so the result is deterministic even though
    // threads race on the same pixels.
    let accum: Vec<AtomicU32> = (0..pixels * 4).map(|_| AtomicU32::new(0)).collect();
    let per_chunk = (total as u64).div_ceil(CHUNKS as u64);

    // Fixed-point palette, so a flush only does integer math.
    let palette_fx: Vec<[u32; 3]> = palette
        .iter()
        .map(|c| c.map(|v| (v * COLOR_SCALE) as u32))
        .collect();

    (0..CHUNKS).into_par_iter().for_each(|chunk| {
        let mut rng = ChaCha8Rng::seed_from_u64(
            spec.seed ^ (chunk as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
        );
        let mut walker = chaos.warm_walker(&mut rng);
        // Bright pixels are hit by every thread, so hits are merged in a small private
        // cache first and only evicted entries touch the shared buffer.
        let mut cache = HitCache::new();
        for _ in 0..per_chunk {
            let Some(p) = chaos.step(&mut walker, &mut rng) else {
                continue;
            };
            let Some(i) = view.pixel(p.x, p.y) else {
                continue;
            };
            cache.add(
                i as u32,
                palette_fx[(p.c.clamp(0.0, 1.0) * 255.0) as usize],
                &accum,
            );
        }
        cache.flush(&accum);
    });

    accum
        .into_par_iter()
        .map(|a| a.into_inner() as f32)
        .collect()
}

const CACHE_BITS: u32 = 12;
const EMPTY: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct Hits {
    pixel: u32,
    n: u32,
    rgb: [u32; 3],
}

/// Direct-mapped cache of pending pixel hits for one thread.
struct HitCache(Vec<Hits>);

impl HitCache {
    fn new() -> Self {
        Self(vec![
            Hits {
                pixel: EMPTY,
                n: 0,
                rgb: [0; 3]
            };
            1 << CACHE_BITS
        ])
    }

    #[inline]
    fn add(&mut self, pixel: u32, rgb: [u32; 3], accum: &[AtomicU32]) {
        let slot = &mut self.0[(pixel.wrapping_mul(0x9E37_79B1) >> (32 - CACHE_BITS)) as usize];
        if slot.pixel != pixel {
            commit(slot, accum);
            slot.pixel = pixel;
        }
        slot.n += 1;
        slot.rgb[0] += rgb[0];
        slot.rgb[1] += rgb[1];
        slot.rgb[2] += rgb[2];
    }

    fn flush(&mut self, accum: &[AtomicU32]) {
        for slot in &mut self.0 {
            commit(slot, accum);
        }
    }
}

#[inline]
fn commit(slot: &mut Hits, accum: &[AtomicU32]) {
    if slot.pixel != EMPTY {
        let a = &accum[slot.pixel as usize * 4..slot.pixel as usize * 4 + 4];
        a[0].fetch_add(slot.n, Relaxed);
        a[1].fetch_add(slot.rgb[0], Relaxed);
        a[2].fetch_add(slot.rgb[1], Relaxed);
        a[3].fetch_add(slot.rgb[2], Relaxed);
    }
    *slot = Hits {
        pixel: EMPTY,
        n: 0,
        rgb: [0; 3],
    };
}

fn blank_rows(g: &Genome, spec: &RenderSpec, rows: usize) -> Image {
    let bg = if spec.transparent {
        [0.0; 4]
    } else {
        [g.background[0], g.background[1], g.background[2], 1.0]
    };
    Image {
        width: spec.width,
        height: rows as u32,
        rgba: bg
            .iter()
            .copied()
            .cycle()
            .take(spec.width as usize * rows * 4)
            .collect(),
    }
}

/// Adaptive density estimation. A pixel with few hits is noisy, so its light is spread
/// over a wide kernel; a dense pixel keeps its detail. Each pixel's contribution is
/// split between two neighbouring blur levels *before* blurring, which conserves the
/// total light (the blur is equivalent to scattering a kernel from every pixel).
fn density_filter(raw: &[f32], w: usize, h: usize, max_sigma: f32) -> Vec<f32> {
    const N_REF: f32 = 6.0;
    let sigmas = [
        0.0,
        max_sigma / 8.0,
        max_sigma / 4.0,
        max_sigma / 2.0,
        max_sigma,
    ];
    let top = sigmas.len() - 1;

    // How much of a pixel with `n` hits goes to blur level `k`.
    let level_weight = |n: f32, k: usize| -> f32 {
        let s = max_sigma * N_REF / (N_REF + n);
        let j = sigmas.iter().rposition(|&v| v <= s).unwrap_or(0);
        if j == top {
            return (k == j) as u8 as f32;
        }
        let f = (s - sigmas[j]) / (sigmas[j + 1] - sigmas[j]);
        if k == j {
            1.0 - f
        } else if k == j + 1 {
            f
        } else {
            0.0
        }
    };

    let mut out = vec![0.0f32; raw.len()];
    for k in 0..sigmas.len() {
        let mut part = vec![0.0f32; raw.len()];
        part.par_chunks_mut(4)
            .zip(raw.par_chunks(4))
            .for_each(|(o, r)| {
                let wt = level_weight(r[0], k);
                if wt > 0.0 {
                    for c in 0..4 {
                        o[c] = r[c] * wt;
                    }
                }
            });
        let spread = if k == 0 {
            part
        } else {
            gaussian_blur(&part, w, h, sigmas[k])
        };
        out.par_iter_mut()
            .zip(spread.par_iter())
            .for_each(|(o, v)| *o += v);
    }
    out
}

/// Separable Gaussian blur of an interleaved 4-channel buffer; edges clamp.
fn gaussian_blur(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let radius = (sigma * 3.0).ceil().max(1.0) as usize;
    let mut kernel: Vec<f32> = (0..=2 * radius)
        .map(|i| (-((i as f32 - radius as f32).powi(2)) / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f32 = kernel.iter().sum();
    kernel.iter_mut().for_each(|k| *k /= sum);

    let mut tmp = vec![0.0f32; src.len()];
    tmp.par_chunks_mut(w * 4)
        .zip(src.par_chunks(w * 4))
        .for_each(|(out_row, row)| {
            for x in 0..w {
                let mut acc = [0.0f32; 4];
                for (i, k) in kernel.iter().enumerate() {
                    let sx = (x + i).saturating_sub(radius).min(w - 1);
                    for c in 0..4 {
                        acc[c] += k * row[sx * 4 + c];
                    }
                }
                out_row[x * 4..x * 4 + 4].copy_from_slice(&acc);
            }
        });
    let mut out = vec![0.0f32; src.len()];
    out.par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, out_row)| {
            for (i, k) in kernel.iter().enumerate() {
                let sy = (y + i).saturating_sub(radius).min(h - 1);
                let row = &tmp[sy * w * 4..(sy + 1) * w * 4];
                for (o, v) in out_row.iter_mut().zip(row) {
                    *o += k * v;
                }
            }
        });
    out
}

/// Returns premultiplied RGBA. Opaque renders are composited over the background (alpha 1).
fn tone_map(
    g: &Genome,
    density: &[f32],
    spp: f32,
    w: usize,
    h: usize,
    transparent: bool,
) -> Vec<f32> {
    let inv_gamma = (1.0 / g.gamma.max(0.1)) as f32;
    let vibrancy = g.vibrancy.clamp(0.0, 1.0) as f32;
    let brightness = g.brightness as f32;
    let bg = g.background;
    let mut rgba = vec![0.0f32; w * h * 4];

    rgba.par_chunks_mut(4)
        .zip(density.par_chunks(4))
        .for_each(|(px, d)| {
            let nf = d[0];
            if nf <= 1e-6 {
                px.copy_from_slice(&if transparent {
                    [0.0; 4]
                } else {
                    [bg[0], bg[1], bg[2], 1.0]
                });
                return;
            }
            let avg = [
                d[1] / (COLOR_SCALE * nf),
                d[2] / (COLOR_SCALE * nf),
                d[3] / (COLOR_SCALE * nf),
            ];
            // Log density: a pixel sampled at the average rate lands mid-range.
            let level = (brightness * (1.0 + nf / spp).ln() / 8.0).clamp(0.0, 1.0);
            let alpha = level.powf(inv_gamma);
            let mut rgb = [0.0f32; 3];
            for k in 0..3 {
                let per_channel = (avg[k] * level).powf(inv_gamma);
                rgb[k] = vibrancy * avg[k] * alpha + (1.0 - vibrancy) * per_channel;
            }
            if transparent {
                px.copy_from_slice(&[
                    rgb[0].min(alpha),
                    rgb[1].min(alpha),
                    rgb[2].min(alpha),
                    alpha,
                ]);
            } else {
                for k in 0..3 {
                    px[k] = (rgb[k] + bg[k] * (1.0 - alpha)).clamp(0.0, 1.0);
                }
                px[3] = 1.0;
            }
        });
    rgba
}

/// Averages `os`x`os` blocks of premultiplied RGBA and converts to straight alpha.
fn downsample(premul: Vec<f32>, hw: usize, hh: usize, os: usize, transparent: bool) -> Image {
    let (w, h) = (hw / os, hh / os);
    let mut rgba = vec![0.0f32; w * h * 4];
    let inv = 1.0 / (os * os) as f32;
    rgba.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let mut acc = [0.0f32; 4];
            for dy in 0..os {
                for dx in 0..os {
                    let i = ((y * os + dy) * hw + x * os + dx) * 4;
                    for c in 0..4 {
                        acc[c] += premul[i + c];
                    }
                }
            }
            let px = &mut row[x * 4..x * 4 + 4];
            for c in 0..4 {
                acc[c] *= inv;
            }
            if transparent && acc[3] > 0.0 {
                px.copy_from_slice(&[
                    (acc[0] / acc[3]).min(1.0),
                    (acc[1] / acc[3]).min(1.0),
                    (acc[2] / acc[3]).min(1.0),
                    acc[3],
                ]);
            } else {
                px.copy_from_slice(&acc);
            }
        }
    });
    Image {
        width: w as u32,
        height: h as u32,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flame::random::{RandomSettings, randomize};

    fn small(seed: u64) -> RenderSpec {
        RenderSpec {
            width: 64,
            height: 48,
            quality: 20.0,
            seed,
            oversample: 2,
            ..Default::default()
        }
    }

    #[test]
    fn same_seed_same_image() {
        let g = randomize(&RandomSettings::default(), 11);
        assert_eq!(render(&g, &small(5)).rgba, render(&g, &small(5)).rgba);
        assert_ne!(render(&g, &small(5)).rgba, render(&g, &small(6)).rgba);
    }

    #[test]
    fn strips_match_the_full_render() {
        let g = randomize(&RandomSettings::default(), 21);
        let spec = RenderSpec {
            width: 90,
            height: 70,
            quality: 25.0,
            seed: 4,
            oversample: 2,
            ..Default::default()
        };
        let full = render(&g, &spec);
        let mut stitched = Vec::new();
        render_strips::<()>(&g, &spec, 90 * 4 * 13, |strip| {
            stitched.extend(strip.rgba);
            Ok(())
        })
        .unwrap();
        assert_eq!(stitched.len(), full.rgba.len());
        let worst = stitched
            .iter()
            .zip(&full.rgba)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < 1e-3,
            "strips differ from the full render by {worst}"
        );
    }

    #[test]
    fn density_filter_conserves_total_light() {
        let (w, h) = (40, 30);
        let mut raw = vec![0.0f32; w * h * 4];
        for (i, v) in [
            (3 * w + 4, 1.0),
            (15 * w + 20, 40.0),
            (25 * w + 33, 3.0),
            (10 * w + 10, 500.0),
        ] {
            raw[i * 4..i * 4 + 4].copy_from_slice(&[v, 2.0 * v, 3.0 * v, 4.0 * v]);
        }
        let out = density_filter(&raw, w, h, 3.0);
        for c in 0..4 {
            let (a, b): (f32, f32) = (
                raw.iter().skip(c).step_by(4).sum(),
                out.iter().skip(c).step_by(4).sum(),
            );
            // Edge clamping in the blur may shift a little light; interior points here are far from the edge.
            assert!((a - b).abs() / a < 0.02, "channel {c}: {a} vs {b}");
        }
    }

    #[test]
    fn random_flames_render_something_and_mutations_differ() {
        let settings = RandomSettings::default();
        for seed in 1..=6 {
            let g = randomize(&settings, seed);
            let spec = RenderSpec {
                width: 160,
                height: 120,
                quality: 30.0,
                transparent: true,
                denoise: 0.0,
                ..Default::default()
            };
            assert!(
                render(&g, &spec).rgba.chunks_exact(4).any(|p| p[3] > 0.2),
                "seed {seed}: empty image"
            );
            let m = crate::flame::random::mutate(&g, &settings, 0.5, seed);
            assert_ne!(m.xforms, g.xforms, "seed {seed}: mutation changed nothing");
        }
    }
}
