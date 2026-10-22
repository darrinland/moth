use rand::{Rng, RngExt};

pub const PALETTE_SIZE: usize = 256;

/// 256 RGB entries, 0..=1 per channel.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette(pub Vec<[f32; 3]>);

impl Palette {
    pub fn grayscale() -> Self {
        Self((0..PALETTE_SIZE).map(|i| [i as f32 / 255.0; 3]).collect())
    }

    /// Looks up `t` in 0..=1 with linear interpolation.
    #[inline]
    pub fn sample(&self, t: f64) -> [f32; 3] {
        let p = t.clamp(0.0, 1.0) * (PALETTE_SIZE - 1) as f64;
        let i = p as usize;
        let j = (i + 1).min(PALETTE_SIZE - 1);
        let f = (p - i as f64) as f32;
        let (a, b) = (self.0[i], self.0[j]);
        [
            a[0] + (b[0] - a[0]) * f,
            a[1] + (b[1] - a[1]) * f,
            a[2] + (b[2] - a[2]) * f,
        ]
    }

    /// A smooth looping gradient through a few random hues.
    pub fn random<R: Rng + ?Sized>(rng: &mut R) -> Self {
        let keys = rng.random_range(3..=6usize);
        let base_hue = rng.random_range(0.0..1.0f32);
        let spread = rng.random_range(0.1..0.6f32);
        let colors: Vec<[f32; 3]> = (0..keys)
            .map(|_| {
                let h = base_hue + rng.random_range(-spread..spread);
                let s = rng.random_range(0.35..1.0f32);
                let v = rng.random_range(0.45..1.0f32);
                hsv_to_rgb(h.rem_euclid(1.0), s, v)
            })
            .collect();
        // Cosine-interpolate around the loop so the palette wraps seamlessly.
        let entries = (0..PALETTE_SIZE)
            .map(|i| {
                let p = i as f32 / PALETTE_SIZE as f32 * keys as f32;
                let k = p.floor() as usize;
                let t = p - k as f32;
                let t = 0.5 - 0.5 * (t * std::f32::consts::PI).cos();
                let (a, b) = (colors[k % keys], colors[(k + 1) % keys]);
                [
                    a[0] + (b[0] - a[0]) * t,
                    a[1] + (b[1] - a[1]) * t,
                    a[2] + (b[2] - a[2]) * t,
                ]
            })
            .collect();
        Self(entries)
    }

    /// Rotates every color's hue by `turns` (1.0 is a full circle).
    pub fn shift_hue(&mut self, turns: f32) {
        for c in &mut self.0 {
            let (h, s, v) = rgb_to_hsv(*c);
            *c = hsv_to_rgb((h + turns).rem_euclid(1.0), s, v);
        }
    }
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

pub fn rgb_to_hsv([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, if max == 0.0 { 0.0 } else { d / max }, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip() {
        for &c in &[
            [0.2, 0.5, 0.9],
            [1.0, 0.0, 0.0],
            [0.3, 0.3, 0.3],
            [0.0, 0.8, 0.1],
        ] {
            let (h, s, v) = rgb_to_hsv(c);
            let back = hsv_to_rgb(h, s, v);
            for k in 0..3 {
                assert!((c[k] - back[k]).abs() < 1e-5, "{c:?} -> {back:?}");
            }
        }
    }
}
