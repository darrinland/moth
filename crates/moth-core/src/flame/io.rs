//! Reading and writing flam3 / Apophysis style `.flame` XML.
//!
//! Variations Moth does not implement yet are skipped on import and reported, never
//! silently dropped: see [`Imported::ignored`].

use std::fmt::Write as _;

use super::genome::{Affine, Genome, Xform};
use super::palette::{PALETTE_SIZE, Palette};
use super::variations::Variation;

#[derive(Debug, thiserror::Error)]
pub enum FlameError {
    #[error("not valid XML: {0}")]
    Xml(#[from] roxmltree::Error),
    #[error("no <flame> element found")]
    NoFlame,
    #[error("bad value for `{attr}` in <{element}>: {value:?}")]
    BadValue {
        element: String,
        attr: String,
        value: String,
    },
}

#[derive(Debug)]
pub struct Imported {
    pub genome: Genome,
    /// Attributes that were present but not understood (unsupported variations and parameters).
    pub ignored: Vec<String>,
}

/// Attributes on `<xform>` that are not variations.
const XFORM_ATTRS: &[&str] = &[
    "weight",
    "color",
    "color_speed",
    "symmetry",
    "coefs",
    "post",
    "chaos",
    "opacity",
    "name",
    "animate",
    "var_color",
];

pub fn to_xml(g: &Genome) -> String {
    let mut s = String::new();
    let c = &g.camera;
    let _ = writeln!(
        s,
        "<flame name=\"{}\" size=\"{} {}\" center=\"{} {}\" scale=\"{}\" rotate=\"{}\" brightness=\"{}\" gamma=\"{}\" vibrancy=\"{}\" background=\"{} {} {}\">",
        escape(&g.name),
        c.size.0,
        c.size.1,
        c.center.0,
        c.center.1,
        c.scale,
        c.rotate,
        g.brightness,
        g.gamma,
        g.vibrancy,
        g.background[0],
        g.background[1],
        g.background[2],
    );
    for x in &g.xforms {
        write_xform(&mut s, "xform", x);
    }
    if let Some(f) = &g.final_xform {
        write_xform(&mut s, "finalxform", f);
    }
    let _ = write!(s, "  <palette count=\"{PALETTE_SIZE}\" format=\"RGB\">");
    for (i, rgb) in g.palette.0.iter().enumerate() {
        if i % 8 == 0 {
            s.push_str("\n    ");
        }
        for ch in rgb {
            let _ = write!(s, "{:02x}", (ch.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    s.push_str("\n  </palette>\n</flame>\n");
    s
}

fn write_xform(s: &mut String, tag: &str, x: &Xform) {
    let _ = write!(
        s,
        "  <{tag} weight=\"{}\" color=\"{}\" color_speed=\"{}\" coefs=\"{}\"",
        x.weight,
        x.color,
        x.color_speed,
        coefs(&x.affine)
    );
    if !x.post.is_identity() {
        let _ = write!(s, " post=\"{}\"", coefs(&x.post));
    }
    for (v, w) in &x.variations {
        let _ = write!(s, " {}=\"{}\"", v.name(), w);
    }
    s.push_str("/>\n");
}

/// flam3 order: `a d b e c f`.
fn coefs(a: &Affine) -> String {
    format!("{} {} {} {} {} {}", a.a, a.d, a.b, a.e, a.c, a.f)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Parses every `<flame>` in the document (a file may hold a whole collection).
pub fn from_xml(xml: &str) -> Result<Vec<Imported>, FlameError> {
    let doc = roxmltree::Document::parse(xml)?;
    let flames: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("flame"))
        .collect();
    if flames.is_empty() {
        return Err(FlameError::NoFlame);
    }
    flames.into_iter().map(read_flame).collect()
}

fn read_flame(node: roxmltree::Node) -> Result<Imported, FlameError> {
    let mut g = Genome::blank();
    let mut ignored = Vec::new();
    g.xforms.clear();
    g.name = node.attribute("name").unwrap_or("").to_string();
    if let Some(v) = nums(node, "size")? {
        if v.len() == 2 {
            g.camera.size = (v[0].max(1.0) as u32, v[1].max(1.0) as u32);
        }
    }
    if let Some(v) = nums(node, "center")? {
        if v.len() == 2 {
            g.camera.center = (v[0], v[1]);
        }
    }
    g.camera.scale = scalar(node, "scale")?.unwrap_or(g.camera.scale);
    g.camera.rotate = scalar(node, "rotate")?.unwrap_or(0.0);
    g.brightness = scalar(node, "brightness")?.unwrap_or(g.brightness);
    g.gamma = scalar(node, "gamma")?.unwrap_or(g.gamma);
    g.vibrancy = scalar(node, "vibrancy")?.unwrap_or(g.vibrancy);
    if let Some(v) = nums(node, "background")? {
        if v.len() == 3 {
            g.background = [v[0] as f32, v[1] as f32, v[2] as f32];
        }
    }

    let mut palette: Vec<[f32; 3]> = vec![[0.0; 3]; PALETTE_SIZE];
    let mut have_palette = false;
    for child in node.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "xform" => g.xforms.push(read_xform(child, &mut ignored)?),
            "finalxform" => g.final_xform = Some(read_xform(child, &mut ignored)?),
            "palette" => {
                if let Some(p) = read_hex_palette(child) {
                    palette = p;
                    have_palette = true;
                }
            }
            "color" => {
                if let (Some(i), Some(rgb)) = (scalar(child, "index")?, nums(child, "rgb")?) {
                    if (i as usize) < PALETTE_SIZE && rgb.len() == 3 {
                        palette[i as usize] = [
                            rgb[0] as f32 / 255.0,
                            rgb[1] as f32 / 255.0,
                            rgb[2] as f32 / 255.0,
                        ];
                        have_palette = true;
                    }
                }
            }
            other => ignored.push(format!("<{other}>")),
        }
    }
    if have_palette {
        g.palette = Palette(palette);
    }
    if g.xforms.is_empty() {
        g.xforms.push(Xform::linear());
    }
    ignored.sort();
    ignored.dedup();
    Ok(Imported { genome: g, ignored })
}

fn read_xform(node: roxmltree::Node, ignored: &mut Vec<String>) -> Result<Xform, FlameError> {
    let mut x = Xform::linear();
    x.variations.clear();
    x.weight = scalar(node, "weight")?.unwrap_or(1.0);
    x.color = scalar(node, "color")?.unwrap_or(0.0);
    x.color_speed = match (scalar(node, "color_speed")?, scalar(node, "symmetry")?) {
        (Some(cs), _) => cs,
        (None, Some(sym)) => (1.0 - sym) / 2.0,
        (None, None) => 0.5,
    };
    if let Some(v) = nums(node, "coefs")? {
        x.affine = affine_from(&v).ok_or_else(|| bad(node, "coefs"))?;
    }
    if let Some(v) = nums(node, "post")? {
        x.post = affine_from(&v).ok_or_else(|| bad(node, "post"))?;
    }
    for attr in node.attributes() {
        let name = attr.name();
        if XFORM_ATTRS.contains(&name) {
            continue;
        }
        match Variation::from_name(name) {
            Some(v) => {
                let w: f64 = attr.value().trim().parse().map_err(|_| bad(node, name))?;
                if w != 0.0 {
                    x.variations.push((v, w));
                }
            }
            None => ignored.push(name.to_string()),
        }
    }
    if x.variations.is_empty() {
        x.variations.push((Variation::Linear, 1.0));
    }
    Ok(x)
}

fn affine_from(v: &[f64]) -> Option<Affine> {
    (v.len() == 6).then(|| Affine {
        a: v[0],
        d: v[1],
        b: v[2],
        e: v[3],
        c: v[4],
        f: v[5],
    })
}

fn read_hex_palette(node: roxmltree::Node) -> Option<Vec<[f32; 3]>> {
    let hex: String = node
        .text()?
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let digits = match node.attribute("format") {
        Some("RGBA") => 8,
        _ => 6,
    };
    if hex.len() < PALETTE_SIZE * digits || !hex.is_ascii() {
        return None;
    }
    (0..PALETTE_SIZE)
        .map(|i| {
            let e = &hex[i * digits..(i + 1) * digits];
            let ch = |k: usize| {
                u8::from_str_radix(&e[k * 2..k * 2 + 2], 16)
                    .ok()
                    .map(|b| b as f32 / 255.0)
            };
            Some([ch(0)?, ch(1)?, ch(2)?])
        })
        .collect()
}

fn bad(node: roxmltree::Node, attr: &str) -> FlameError {
    FlameError::BadValue {
        element: node.tag_name().name().to_string(),
        attr: attr.to_string(),
        value: node.attribute(attr).unwrap_or("").to_string(),
    }
}

fn nums(node: roxmltree::Node, attr: &str) -> Result<Option<Vec<f64>>, FlameError> {
    let Some(raw) = node.attribute(attr) else {
        return Ok(None);
    };
    raw.split_whitespace()
        .map(|t| t.parse::<f64>().map_err(|_| bad(node, attr)))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn scalar(node: roxmltree::Node, attr: &str) -> Result<Option<f64>, FlameError> {
    match nums(node, attr)? {
        None => Ok(None),
        Some(v) if v.len() == 1 => Ok(Some(v[0])),
        Some(_) => Err(bad(node, attr)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flame::random::{RandomSettings, randomize};

    #[test]
    fn round_trips_a_random_flame() {
        let g = randomize(&RandomSettings::default(), 7);
        let back = from_xml(&to_xml(&g)).unwrap().remove(0);
        assert!(back.ignored.is_empty(), "{:?}", back.ignored);
        let b = back.genome;
        assert_eq!(b.xforms, g.xforms);
        assert_eq!(b.final_xform, g.final_xform);
        assert_eq!(b.camera, g.camera);
        for (p, q) in b.palette.0.iter().zip(&g.palette.0) {
            for k in 0..3 {
                assert!((p[k] - q[k]).abs() <= 1.0 / 255.0);
            }
        }
    }

    #[test]
    fn reads_flam3_coefs_order_and_reports_unknown_variations() {
        let xml = r#"<flame name="t" size="100 100" center="0 0" scale="50">
            <xform weight="1" color="0" coefs="1 2 3 4 5 6" linear="1" blob="0.5" blob_low="0.1"/>
        </flame>"#;
        let imp = from_xml(xml).unwrap().remove(0);
        let a = imp.genome.xforms[0].affine;
        assert_eq!(
            (a.a, a.d, a.b, a.e, a.c, a.f),
            (1.0, 2.0, 3.0, 4.0, 5.0, 6.0)
        );
        assert_eq!(imp.ignored, ["blob", "blob_low"]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(from_xml("<nope/>"), Err(FlameError::NoFlame)));
        assert!(from_xml("<flame scale=\"abc\"/>").is_err());
    }
}
