use std::fmt::{self, Write as _};
use std::path::Path;

use font8x8::{BASIC_FONTS, UnicodeFonts};
use image::imageops::FilterType;
use image::{DynamicImage, Rgb, RgbImage};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Command-line usage mistake versus a failure while doing the requested work.
#[derive(Debug)]
pub enum Error {
    Usage(String),
    Internal(Box<dyn std::error::Error>),
}

impl Error {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    pub fn internal(error: impl Into<Box<dyn std::error::Error>>) -> Self {
        Self::Internal(error.into())
    }

    pub fn is_usage(&self) -> bool {
        matches!(self, Self::Usage(_))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) => formatter.write_str(message),
            Self::Internal(error) => error.fmt(formatter),
        }
    }
}

impl<E: std::error::Error + 'static> From<E> for Error {
    fn from(error: E) -> Self {
        Self::Internal(Box::new(error))
    }
}

/// Fixed corpus manifest, shared by gallery, benchmark and verify-corpus.
#[derive(Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub image_count: usize,
    pub images: Vec<ManifestImage>,
}

#[derive(Deserialize)]
pub struct ManifestImage {
    pub file: String,
    pub format: String,
    pub mode: String,
    pub size: [u32; 2],
    pub key_regions: Vec<KeyRegion>,
    pub sha256: String,
}

#[derive(Deserialize)]
pub struct KeyRegion {
    pub name: String,
    #[serde(rename = "box")]
    pub bounds: [u32; 4],
}

pub fn read_manifest(directory: &Path) -> Result<Manifest> {
    let path = directory.join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| Error::internal(format!("{}: {error}", path.display())))?;
    Ok(serde_json::from_str(&text)?)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

pub fn sha256_file(path: &Path) -> Result<String> {
    Ok(sha256_hex(&std::fs::read(path)?))
}

pub fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    Ok(image::load_from_memory(bytes)?)
}

/// RGB8 view of a decoded image; alpha is composited onto white like the
/// converter contract, so transparent fixtures stay readable in sheets.
pub fn to_rgb(image: &DynamicImage) -> RgbImage {
    if image.color().has_alpha() {
        crate::color::alpha_on_white(image.to_rgba8())
    } else {
        image.to_rgb8()
    }
}

pub fn encode_jpeg(image: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality);
    encoder.encode_image(image)?;
    Ok(bytes)
}

/// Downscale-only fit into the given bounds, matching the old gallery panels.
pub fn fit(image: &RgbImage, max_width: u32, max_height: u32) -> RgbImage {
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return RgbImage::new(1, 1);
    }
    let scale = (max_width as f64 / width as f64)
        .min(max_height as f64 / height as f64)
        .min(1.0);
    let target_width = ((width as f64 * scale).round() as u32).max(1);
    let target_height = ((height as f64 * scale).round() as u32).max(1);
    if (target_width, target_height) == (width, height) {
        image.clone()
    } else {
        image::imageops::resize(image, target_width, target_height, FilterType::Lanczos3)
    }
}

/// Bitmap label using font8x8; the Chinese sample content is already rasterized.
pub fn draw_text(canvas: &mut RgbImage, x: i32, y: i32, text: &str, color: [u8; 3]) {
    let (width, height) = canvas.dimensions();
    let mut cursor = x;
    for character in text.chars() {
        let glyph = BASIC_FONTS.get(character).or_else(|| BASIC_FONTS.get('?'));
        if let Some(glyph) = glyph {
            for (row, bits) in glyph.iter().enumerate() {
                for column in 0..8u8 {
                    if *bits & (1u8 << column) == 0 {
                        continue;
                    }
                    let pixel_x = cursor + column as i32;
                    let pixel_y = y + row as i32;
                    if pixel_x >= 0
                        && pixel_y >= 0
                        && (pixel_x as u32) < width
                        && (pixel_y as u32) < height
                    {
                        canvas.put_pixel(pixel_x as u32, pixel_y as u32, Rgb(color));
                    }
                }
            }
        }
        cursor += 8;
    }
}
