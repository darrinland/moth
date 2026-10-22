//! Random flames and mutation. A candidate is kept only if it renders something
//! worth looking at; otherwise the seed is advanced deterministically and retried.

use rand::{Rng, RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::f64::consts::TAU;

use super::genome::{Affine, Genome, Xform};
use super::palette::Palette;
use super::render::{Chaos, render};
use super::variations::Variation;
use crate::generator::RenderSpec;

const MAX_ATTEMPTS: u64 = 60;
const PROBE_SAMPLES: usize = 20_000;
const PROBE_WALKERS: usize = 8;

#[derive(Clone, Debug)]
pub struct RandomSettings {
    /// Variations the randomizer may pick from.
    pub allowed: Vec<Variation>,
    /// Inclusive range for the number of transforms.
    pub xforms: (usize, usize),
    /// Inclusive range for variations mixed into each transform.
    pub variations_per_xform: (usize, usize),
    /// 0..=1: how far transforms may stretch, shear and wander.
    pub wildness: f64,
    pub final_xform_chance: f64,
    /// Reference size stored in new flames; the camera is fitted to it.
    pub size: (u32, u32),
}

impl Default for RandomSettings {
    fn default() -> Self {
        Self {
            allowed: Variation::ALL.to_vec(),
            xforms: (2, 5),
            variations_per_xform: (1, 3),
            wildness: 0.5,
            final_xform_chance: 0.15,
            size: (1024, 768),
        }
    }
}

pub fn randomize(s: &RandomSettings, seed: u64) -> Genome {
    let mut last = None;
    for attempt in 0..MAX_ATTEMPTS {
        let mut rng = attempt_rng(seed, attempt);
        let mut g = candidate(s, &mut rng);
        if fit_camera(&mut g, &mut rng) {
            if worth_keeping(&g) {
                return g;
            }
            last = Some(g);
        }
    }
    last.unwrap_or_else(Genome::blank)
}

pub fn mutate(g: &Genome, s: &RandomSettings, amount: f32, seed: u64) -> Genome {
    let amount = amount.clamp(0.0, 1.0) as f64;
    let mut fallback = None;
    for attempt in 0..MAX_ATTEMPTS {
        let mut rng = attempt_rng(seed, attempt);
        let mut m = g.clone();
        for x in &mut m.xforms {
            nudge_affine(&mut x.affine, amount, &mut rng);
            for (_, w) in &mut x.variations {
                *w = (*w * (1.0 + rng.random_range(-1.0..1.0) * amount * 0.5)).max(0.01);
            }
            if rng.random_bool(amount * 0.3) {
                swap_variation(x, s, &mut rng);
            }
        }
        m.palette
            .shift_hue((rng.random_range(-1.0..1.0) * amount * 0.25) as f32);
        if fit_camera(&mut m, &mut rng) {
            if worth_keeping(&m) {
                return m;
            }
            fallback = Some(m);
        }
    }
    fallback.unwrap_or_else(|| g.clone())
}

fn attempt_rng(seed: u64, attempt: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed.wrapping_add(attempt.wrapping_mul(0xD1B5_4A32_D192_ED03)))
}

fn candidate<R: Rng + ?Sized>(s: &RandomSettings, rng: &mut R) -> Genome {
    let n = rng.random_range(s.xforms.0.max(1)..=s.xforms.1.max(s.xforms.0.max(1)));
    let mut g = Genome::blank();
    g.camera.size = s.size;
    g.palette = Palette::random(rng);
    g.xforms = (0..n)
        .map(|i| {
            let mut x = Xform::linear();
            x.weight = rng.random_range(0.2..1.0);
            x.color = if n > 1 {
                i as f64 / (n - 1) as f64
            } else {
                0.0
            };
            x.color_speed = rng.random_range(0.2..0.8);
            x.affine = random_affine(s.wildness, rng);
            x.variations.clear();
            let k = rng.random_range(
                s.variations_per_xform.0.max(1)
                    ..=s.variations_per_xform
                        .1
                        .max(s.variations_per_xform.0.max(1)),
            );
            for _ in 0..k {
                add_variation(&mut x, s, rng);
            }
            normalize_weights(&mut x);
            x
        })
        .collect();
    if rng.random_bool(s.final_xform_chance.clamp(0.0, 1.0)) {
        let mut f = Xform::linear();
        f.color_speed = 0.0;
        f.affine = random_affine(s.wildness * 0.5, rng);
        f.variations.clear();
        add_variation(&mut f, s, rng);
        g.final_xform = Some(f);
    }
    g
}

fn random_affine<R: Rng + ?Sized>(wildness: f64, rng: &mut R) -> Affine {
    let w = wildness.clamp(0.0, 1.0);
    // R(theta) * diag(s1, s2) * R(phi): mostly contractive, with some stretch and shear.
    let (s1, s2) = (
        rng.random_range(0.25..0.75 + 0.25 * w),
        rng.random_range(0.25..0.75 + 0.25 * w),
    );
    let (t, p) = (
        rng.random_range(0.0..TAU),
        rng.random_range(-0.5..0.5) * w * std::f64::consts::PI,
    );
    let (st, ct) = t.sin_cos();
    let (sp, cp) = p.sin_cos();
    let (a, b, d, e) = (
        ct * s1 * cp - st * s2 * sp,
        -ct * s1 * sp - st * s2 * cp,
        st * s1 * cp + ct * s2 * sp,
        -st * s1 * sp + ct * s2 * cp,
    );
    let reach = 0.3 + 1.2 * w;
    Affine {
        a,
        b,
        c: rng.random_range(-reach..reach),
        d,
        e,
        f: rng.random_range(-reach..reach),
    }
}

fn add_variation<R: Rng + ?Sized>(x: &mut Xform, s: &RandomSettings, rng: &mut R) {
    let pool = if s.allowed.is_empty() {
        &[Variation::Linear][..]
    } else {
        &s.allowed[..]
    };
    let v = pool[rng.random_range(0..pool.len())];
    let w = rng.random_range(0.2..1.0);
    match x.variations.iter_mut().find(|(have, _)| *have == v) {
        Some((_, existing)) => *existing += w,
        None => x.variations.push((v, w)),
    }
}

fn swap_variation<R: Rng + ?Sized>(x: &mut Xform, s: &RandomSettings, rng: &mut R) {
    if x.variations.len() > 1 {
        x.variations.remove(rng.random_range(0..x.variations.len()));
    }
    add_variation(x, s, rng);
    normalize_weights(x);
}

fn normalize_weights(x: &mut Xform) {
    let sum: f64 = x.variations.iter().map(|(_, w)| w).sum();
    if sum > 0.0 {
        for (_, w) in &mut x.variations {
            *w /= sum;
        }
    }
}

fn nudge_affine<R: Rng + ?Sized>(a: &mut Affine, amount: f64, rng: &mut R) {
    let mut j = |v: &mut f64, scale: f64| *v += rng.random_range(-1.0..1.0) * amount * scale;
    for v in [&mut a.a, &mut a.b, &mut a.d, &mut a.e] {
        j(v, 0.3);
    }
    j(&mut a.c, 0.5);
    j(&mut a.f, 0.5);
}

/// Frames the attractor in two passes: a statistical fit from sampled points, then a
/// check against an actual thumbnail render, zooming out if the picture touches the
/// edge and zooming in if it is lost in the middle. Returns false if the flame diverges.
fn fit_camera<R: Rng + ?Sized>(g: &mut Genome, rng: &mut R) -> bool {
    let chaos = Chaos::new(g);
    if chaos.is_empty() {
        return false;
    }
    // Several walkers, so one unlucky orbit cannot define the framing.
    let (mut xs, mut ys) = (
        Vec::with_capacity(PROBE_SAMPLES),
        Vec::with_capacity(PROBE_SAMPLES),
    );
    let mut walkers: Vec<_> = (0..PROBE_WALKERS).map(|_| chaos.warm_walker(rng)).collect();
    'outer: for _ in 0..PROBE_SAMPLES * 2 / PROBE_WALKERS {
        for w in &mut walkers {
            if let Some(p) = chaos.step(w, rng) {
                xs.push(p.x);
                ys.push(p.y);
                if xs.len() == PROBE_SAMPLES {
                    break 'outer;
                }
            }
        }
    }
    if xs.len() < PROBE_SAMPLES / 2 {
        return false;
    }
    let range = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        (v[v.len() / 100], v[v.len() - 1 - v.len() / 100])
    };
    let ((x0, x1), (y0, y1)) = (range(&mut xs), range(&mut ys));
    let (ex, ey) = ((x1 - x0).max(1e-6), (y1 - y0).max(1e-6));
    if ex > 1e6 || ey > 1e6 {
        return false;
    }
    let (w, h) = (g.camera.size.0 as f64, g.camera.size.1 as f64);
    g.camera.center = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    g.camera.scale = 0.8 * (w / ex).min(h / ey);
    g.camera.rotate = 0.0;
    refine_camera(g);
    true
}

const THUMB_W: u32 = 160;
const THUMB_H: u32 = 120;

/// Bounding box (as fractions of the frame) of pixels bright enough to matter.
fn lit_bounds(g: &Genome) -> Option<(f64, f64, f64, f64)> {
    let spec = RenderSpec {
        width: THUMB_W,
        height: THUMB_H,
        quality: 30.0,
        transparent: true,
        seed: 3,
        denoise: 0.0,
        ..Default::default()
    };
    let img = render(g, &spec);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (i, p) in img.rgba.chunks_exact(4).enumerate() {
        if p[3] > 0.2 {
            let (x, y) = (i as u32 % THUMB_W, i as u32 / THUMB_W);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    (x0 != u32::MAX).then(|| {
        (
            x0 as f64 / THUMB_W as f64,
            y0 as f64 / THUMB_H as f64,
            (x1 + 1) as f64 / THUMB_W as f64,
            (y1 + 1) as f64 / THUMB_H as f64,
        )
    })
}

fn refine_camera(g: &mut Genome) {
    for _ in 0..4 {
        let Some((x0, y0, x1, y1)) = lit_bounds(g) else {
            return;
        };
        let (fw, fh) = (x1 - x0, y1 - y0);
        let touches = x0 <= 0.0 || y0 <= 0.0 || x1 >= 1.0 || y1 >= 1.0;
        let fill = fw.max(fh);
        if !touches && (0.55..=0.95).contains(&fill) {
            return;
        }
        // Re-center on the lit region, then rescale so it fills ~85% of the limiting side.
        let ppu = g.camera.scale * THUMB_W as f64 / g.camera.size.0 as f64;
        let (cx, cy) = ((x0 + x1) / 2.0 - 0.5, (y0 + y1) / 2.0 - 0.5);
        g.camera.center.0 += cx * THUMB_W as f64 / ppu;
        g.camera.center.1 += cy * THUMB_H as f64 / ppu;
        let factor = if touches { 0.8 } else { 0.85 / fill.max(0.05) };
        g.camera.scale *= factor.clamp(0.4, 2.5);
    }
}

/// Renders a thumbnail and rejects flames that are mostly empty or a solid blob.
fn worth_keeping(g: &Genome) -> bool {
    let spec = RenderSpec {
        width: 96,
        height: 72,
        quality: 12.0,
        transparent: true,
        seed: 1,
        denoise: 0.0,
        ..Default::default()
    };
    let img = render(g, &spec);
    let lit = img.rgba.chunks_exact(4).filter(|p| p[3] > 0.05).count() as f64;
    let coverage = lit / (img.width * img.height) as f64;
    (0.06..0.85).contains(&coverage)
}
