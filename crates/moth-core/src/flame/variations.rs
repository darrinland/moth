//! Variation functions, following the flam3 / Apophysis definitions.
//!
//! `theta` uses the flam3 convention `atan2(x, y)`.

use rand::{Rng, RngExt};
use std::f64::consts::PI;

use super::genome::Affine;

const EPS: f64 = 1e-10;

macro_rules! variations {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Variation { $($variant),+ }

        impl Variation {
            pub const ALL: &'static [Variation] = &[$(Variation::$variant),+];

            pub fn name(self) -> &'static str {
                match self { $(Variation::$variant => $name),+ }
            }
        }
    };
}

variations! {
    Linear => "linear",
    Sinusoidal => "sinusoidal",
    Spherical => "spherical",
    Swirl => "swirl",
    Horseshoe => "horseshoe",
    Polar => "polar",
    Handkerchief => "handkerchief",
    Heart => "heart",
    Disc => "disc",
    Spiral => "spiral",
    Hyperbolic => "hyperbolic",
    Diamond => "diamond",
    Ex => "ex",
    Julia => "julia",
    Bent => "bent",
    Waves => "waves",
    Fisheye => "fisheye",
    Popcorn => "popcorn",
    Exponential => "exponential",
    Cosine => "cosine",
    Eyefish => "eyefish",
    Bubble => "bubble",
    Cylinder => "cylinder",
    Tangent => "tangent",
    Cross => "cross",
    Rings => "rings",
}

impl Variation {
    pub fn from_name(name: &str) -> Option<Variation> {
        Self::ALL.iter().copied().find(|v| v.name() == name)
    }

    /// Applies the unweighted variation to a point that has already been through the
    /// transform's affine (`aff` is passed for the variations that read its coefficients).
    ///
    /// `r` and `theta` are computed inside the arms that need them: `atan2` is costly,
    /// and this runs billions of times per render.
    #[inline]
    pub fn apply<R: Rng + ?Sized>(self, x: f64, y: f64, aff: &Affine, rng: &mut R) -> (f64, f64) {
        use Variation::*;
        let r2 = x * x + y * y;
        match self {
            Linear => (x, y),
            Sinusoidal => (x.sin(), y.sin()),
            Spherical => {
                let k = 1.0 / (r2 + EPS);
                (x * k, y * k)
            }
            Swirl => {
                let (s, c) = r2.sin_cos();
                (s * x - c * y, c * x + s * y)
            }
            Horseshoe => {
                let k = 1.0 / (r2.sqrt() + EPS);
                ((x - y) * (x + y) * k, 2.0 * x * y * k)
            }
            Polar => (x.atan2(y) / PI, r2.sqrt() - 1.0),
            Handkerchief => {
                let (r, theta) = (r2.sqrt(), x.atan2(y));
                (r * (theta + r).sin(), r * (theta - r).cos())
            }
            Heart => {
                let (r, theta) = (r2.sqrt(), x.atan2(y));
                let a = theta * r;
                (r * a.sin(), -r * a.cos())
            }
            Disc => {
                let (r, theta) = (r2.sqrt(), x.atan2(y));
                let a = theta / PI;
                let pr = PI * r;
                (a * pr.sin(), a * pr.cos())
            }
            Spiral => {
                let (r, theta) = (r2.sqrt(), x.atan2(y));
                let k = 1.0 / (r + EPS);
                (k * (theta.cos() + r.sin()), k * (theta.sin() - r.cos()))
            }
            // sin(theta) = x / r and cos(theta) = y / r, so no atan2 is needed.
            Hyperbolic => {
                let r = r2.sqrt() + EPS;
                (x / (r * r), y)
            }
            Diamond => {
                let r = r2.sqrt();
                let k = 1.0 / (r + EPS);
                ((x * k) * r.cos(), (y * k) * r.sin())
            }
            Ex => {
                let (r, theta) = (r2.sqrt(), x.atan2(y));
                let n0 = (theta + r).sin();
                let n1 = (theta - r).cos();
                let (m0, m1) = (n0 * n0 * n0, n1 * n1 * n1);
                (r * (m0 + m1), r * (m0 - m1))
            }
            Julia => {
                let theta = x.atan2(y);
                let a = 0.5 * theta + if rng.random_bool(0.5) { PI } else { 0.0 };
                let k = r2.sqrt().sqrt();
                let (s, c) = a.sin_cos();
                (k * c, k * s)
            }
            Bent => match (x >= 0.0, y >= 0.0) {
                (true, true) => (x, y),
                (false, true) => (2.0 * x, y),
                (true, false) => (x, y / 2.0),
                (false, false) => (2.0 * x, y / 2.0),
            },
            Waves => {
                let dx = aff.c * aff.c + EPS;
                let dy = aff.f * aff.f + EPS;
                (x + aff.b * (y / dx).sin(), y + aff.e * (x / dy).sin())
            }
            Fisheye => {
                let k = 2.0 / (r2.sqrt() + 1.0);
                (k * y, k * x)
            }
            Popcorn => (
                x + aff.c * (3.0 * y).tan().sin(),
                y + aff.f * (3.0 * x).tan().sin(),
            ),
            Exponential => {
                let k = (x - 1.0).exp();
                let (s, c) = (PI * y).sin_cos();
                (k * c, k * s)
            }
            Cosine => ((PI * x).cos() * y.cosh(), -(PI * x).sin() * y.sinh()),
            Eyefish => {
                let k = 2.0 / (r2.sqrt() + 1.0);
                (k * x, k * y)
            }
            Bubble => {
                let k = 4.0 / (r2 + 4.0);
                (k * x, k * y)
            }
            Cylinder => (x.sin(), y),
            Tangent => (x.sin() / y.cos(), y.tan()),
            Cross => {
                let s = x * x - y * y;
                let k = (1.0 / (s * s + EPS)).sqrt();
                (k * x, k * y)
            }
            Rings => {
                let dx = aff.c * aff.c + EPS;
                let r = r2.sqrt();
                let m = 2.0 * dx;
                let t = r + dx;
                let rr = t - m * (t / m).floor() - dx + r * (1.0 - dx);
                let k = rr / (r + EPS);
                (k * x, k * y)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn names_round_trip_and_are_unique() {
        for v in Variation::ALL {
            assert_eq!(Variation::from_name(v.name()), Some(*v));
        }
        let mut names: Vec<_> = Variation::ALL.iter().map(|v| v.name()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), Variation::ALL.len());
    }

    #[test]
    fn finite_for_ordinary_input() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let aff = Affine {
            a: 0.5,
            b: 0.3,
            c: 0.2,
            d: -0.3,
            e: 0.5,
            f: 0.1,
        };
        for v in Variation::ALL {
            for &(x, y) in &[(0.3, 0.4), (-0.7, 0.2), (1.5, -1.2)] {
                let (ox, oy) = v.apply(x, y, &aff, &mut rng);
                assert!(
                    ox.is_finite() && oy.is_finite(),
                    "{} gave {ox},{oy}",
                    v.name()
                );
            }
        }
    }

    /// The optimized forms must equal the textbook definitions in terms of theta = atan2(x, y).
    #[test]
    fn shortcuts_match_the_definitions() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let aff = Affine::IDENTITY;
        for &(x, y) in &[(0.3f64, 0.4f64), (-0.7, 0.2), (1.5, -1.2), (-0.2, -0.9)] {
            let (r, theta) = ((x * x + y * y).sqrt(), x.atan2(y));
            let (hx, hy) = Variation::Hyperbolic.apply(x, y, &aff, &mut rng);
            assert!(
                (hx - theta.sin() / (r + EPS)).abs() < 1e-6 && (hy - r * theta.cos()).abs() < 1e-6
            );
            let (dx, dy) = Variation::Diamond.apply(x, y, &aff, &mut rng);
            assert!(
                (dx - theta.sin() * r.cos()).abs() < 1e-6
                    && (dy - theta.cos() * r.sin()).abs() < 1e-6
            );
        }
    }
}
