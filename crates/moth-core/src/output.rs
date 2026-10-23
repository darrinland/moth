//! Image encoders. A PNG carries its recipe in an iTXt chunk, so any exported
//! image can be opened again and re-rendered at higher quality.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use crate::generator::Image;

/// iTXt keyword holding the flame XML.
pub const RECIPE_KEY: &str = "moth.flame";

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Encode(#[from] png::EncodingError),
    #[error(transparent)]
    Decode(#[from] png::DecodingError),
    #[error(transparent)]
    Jpeg(#[from] jpeg_encoder::EncodingError),
    #[error("{0}")]
    Format(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitDepth {
    Eight,
    Sixteen,
}

fn encode_rows(img: &Image, depth: BitDepth) -> Vec<u8> {
    match depth {
        BitDepth::Eight => img
            .rgba
            .iter()
            .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect(),
        BitDepth::Sixteen => img
            .rgba
            .iter()
            .flat_map(|v| ((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes())
            .collect(),
    }
}

pub fn write_png(
    path: &Path,
    img: &Image,
    depth: BitDepth,
    recipe: Option<&str>,
) -> Result<(), OutputError> {
    let mut out = PngStream::create(path, img.width, img.height, depth, recipe)?;
    out.write_rows(img)?;
    out.finish()
}

/// Writes a PNG a band of rows at a time, so huge images never sit in memory whole.
pub struct PngStream {
    writer: png::StreamWriter<'static, BufWriter<File>>,
    depth: BitDepth,
}

impl PngStream {
    pub fn create(
        path: &Path,
        width: u32,
        height: u32,
        depth: BitDepth,
        recipe: Option<&str>,
    ) -> Result<Self, OutputError> {
        let mut enc = png::Encoder::new(BufWriter::new(File::create(path)?), width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(match depth {
            BitDepth::Eight => png::BitDepth::Eight,
            BitDepth::Sixteen => png::BitDepth::Sixteen,
        });
        if let Some(xml) = recipe {
            enc.add_itxt_chunk(RECIPE_KEY.to_string(), xml.to_string())?;
        }
        let writer = enc.write_header()?.into_stream_writer()?;
        Ok(Self { writer, depth })
    }

    /// Appends rows; strips must arrive top to bottom and add up to the full height.
    pub fn write_rows(&mut self, rows: &Image) -> Result<(), OutputError> {
        self.writer.write_all(&encode_rows(rows, self.depth))?;
        Ok(())
    }

    pub fn finish(self) -> Result<(), OutputError> {
        self.writer.finish()?;
        Ok(())
    }
}

/// Returns the flame XML embedded in a PNG written by [`write_png`], if any.
pub fn read_png_recipe(path: &Path) -> Result<Option<String>, OutputError> {
    let reader = png::Decoder::new(std::io::BufReader::new(File::open(path)?)).read_info()?;
    Ok(reader
        .info()
        .utf8_text
        .iter()
        .find(|c| c.keyword == RECIPE_KEY)
        .and_then(|c| c.get_text().ok()))
}

/// Reads an 8-bit PNG written by [`write_png`] back as RGBA, for thumbnails.
pub fn read_png_rgba(path: &Path) -> Result<(u32, u32, Vec<u8>), OutputError> {
    let mut reader = png::Decoder::new(std::io::BufReader::new(File::open(path)?)).read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let frame = reader.next_frame(&mut buf)?;
    let pixels = &buf[..frame.buffer_size()];
    let rgba = match frame.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => {
            return Err(OutputError::Format(format!(
                "unexpected thumbnail colour {other:?}"
            )));
        }
    };
    Ok((frame.width, frame.height, rgba))
}

/// JPEG APP segment that carries the recipe: `APP15`, prefixed with the key and a NUL.
const JPEG_RECIPE_SEGMENT: u8 = 15;

/// Appends the pixels of `rows` to `rgb` as 8-bit RGB, compositing over `background`
/// (JPEG has no alpha channel).
pub fn append_rgb8(rows: &Image, background: [f32; 3], rgb: &mut Vec<u8>) {
    rgb.reserve(rows.rgba.len() / 4 * 3);
    for p in rows.rgba.chunks_exact(4) {
        for c in 0..3 {
            let v = p[c] * p[3] + background[c] * (1.0 - p[3]);
            rgb.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
}

pub fn write_jpeg(
    path: &Path,
    width: u32,
    height: u32,
    rgb: &[u8],
    quality: u8,
    recipe: Option<&str>,
) -> Result<(), OutputError> {
    let (w16, h16) = (u16::try_from(width), u16::try_from(height));
    let (Ok(w16), Ok(h16)) = (w16, h16) else {
        return Err(OutputError::Format(
            "JPEG is limited to 65535 pixels per side; use PNG for larger images".into(),
        ));
    };
    let mut enc = jpeg_encoder::Encoder::new_file(path, quality.clamp(1, 100))?;
    if let Some(xml) = recipe {
        let mut data = format!("{RECIPE_KEY}\0").into_bytes();
        data.extend_from_slice(xml.as_bytes());
        enc.add_app_segment(JPEG_RECIPE_SEGMENT, data)?;
    }
    enc.encode(rgb, w16, h16, jpeg_encoder::ColorType::Rgb)?;
    Ok(())
}

/// Returns the flame XML embedded in a JPEG written by [`write_jpeg`], if any.
pub fn read_jpeg_recipe(path: &Path) -> Result<Option<String>, OutputError> {
    let bytes = std::fs::read(path)?;
    let prefix = format!("{RECIPE_KEY}\0").into_bytes();
    let mut i = 2;
    while bytes.len() >= 2 && bytes.starts_with(&[0xFF, 0xD8]) && i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            break;
        }
        let marker = bytes[i + 1];
        if marker == 0xDA {
            break; // start of scan: no more headers
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let body = bytes.get(i + 4..i + 2 + len).unwrap_or(&[]);
        if marker == 0xE0 + JPEG_RECIPE_SEGMENT && body.starts_with(&prefix) {
            return Ok(String::from_utf8(body[prefix.len()..].to_vec()).ok());
        }
        i += 2 + len;
    }
    Ok(None)
}

/// Reads the embedded recipe from a PNG or JPEG, chosen by file extension.
pub fn read_recipe(path: &Path) -> Result<Option<String>, OutputError> {
    match extension(path).as_str() {
        "png" => read_png_recipe(path),
        "jpg" | "jpeg" => read_jpeg_recipe(path),
        _ => Ok(None),
    }
}

/// Lower-cased file extension, or an empty string.
pub fn extension(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("moth-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn recipe_survives_png_and_jpeg() {
        let xml = "<flame name=\"t &amp; ü\"/>";
        let img = Image {
            width: 4,
            height: 2,
            rgba: vec![0.5; 4 * 2 * 4],
        };

        let png = temp("a.png");
        write_png(&png, &img, BitDepth::Sixteen, Some(xml)).unwrap();
        assert_eq!(read_recipe(&png).unwrap().as_deref(), Some(xml));

        let jpg = temp("a.jpg");
        let mut rgb = Vec::new();
        append_rgb8(&img, [0.0; 3], &mut rgb);
        write_jpeg(&jpg, 4, 2, &rgb, 90, Some(xml)).unwrap();
        assert_eq!(read_recipe(&jpg).unwrap().as_deref(), Some(xml));

        let plain = temp("b.png");
        write_png(&plain, &img, BitDepth::Eight, None).unwrap();
        assert_eq!(read_recipe(&plain).unwrap(), None);
        for f in [png, jpg, plain] {
            let _ = std::fs::remove_file(f);
        }
    }
}
