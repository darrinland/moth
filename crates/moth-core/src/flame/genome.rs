use super::palette::Palette;
use super::variations::Variation;

/// `x' = a*x + b*y + c`, `y' = d*x + e*y + f`.
///
/// flam3 files store these as `coefs="a d b e c f"`; the conversion lives in [`super::io`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 0.0,
        e: 1.0,
        f: 0.0,
    };

    #[inline]
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.b * y + self.c,
            self.d * x + self.e * y + self.f,
        )
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Xform {
    /// Relative probability of being picked on each iteration.
    pub weight: f64,
    /// Position in the palette, 0..=1.
    pub color: f64,
    /// How far the running color moves toward `color` on each use, 0..=1.
    pub color_speed: f64,
    pub affine: Affine,
    pub post: Affine,
    pub variations: Vec<(Variation, f64)>,
}

impl Xform {
    pub fn linear() -> Self {
        Self {
            weight: 1.0,
            color: 0.0,
            color_speed: 0.5,
            affine: Affine::IDENTITY,
            post: Affine::IDENTITY,
            variations: vec![(Variation::Linear, 1.0)],
        }
    }
}

/// Maps flame space to pixels. Stored against a reference `size` so the same recipe
/// re-renders at any resolution (scale follows the output width).
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub center: (f64, f64),
    /// Pixels per unit at the reference `size`.
    pub scale: f64,
    /// Degrees, counter-clockwise.
    pub rotate: f64,
    pub size: (u32, u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Genome {
    pub name: String,
    pub xforms: Vec<Xform>,
    /// Applied to each plotted point but not fed back into the iteration.
    pub final_xform: Option<Xform>,
    pub palette: Palette,
    pub camera: Camera,
    pub brightness: f64,
    pub gamma: f64,
    pub vibrancy: f64,
    pub background: [f32; 3],
}

impl Genome {
    pub fn blank() -> Self {
        Self {
            name: String::new(),
            xforms: vec![Xform::linear()],
            final_xform: None,
            palette: Palette::grayscale(),
            camera: Camera {
                center: (0.0, 0.0),
                scale: 256.0,
                rotate: 0.0,
                size: (1024, 768),
            },
            brightness: 4.0,
            gamma: 4.0,
            vibrancy: 1.0,
            background: [0.0; 3],
        }
    }
}
