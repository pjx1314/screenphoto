use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbImage};
use jpeg_encoder::{ChromaSubsamplingMethod, ColorType, Encoder, SamplingFactor};

use crate::calibration::Calibration;
use crate::limits::{LONG_EDGE_CAP, MAX_INPUT_BYTES, MAX_INPUT_PIXELS, checked_pixels};
use crate::pipeline;
use crate::{Error, Result};

fn decode_error(error: impl std::fmt::Display) -> Error {
    Error::InvalidInput(format!("cannot decode image: {error}"))
}

fn decode_image(data: &[u8]) -> Result<RgbImage> {
    if data.is_empty() || data.len() > MAX_INPUT_BYTES {
        return Err(Error::InvalidInput(format!(
            "image must contain 1 to {} MiB of encoded data",
            MAX_INPUT_BYTES / (1024 * 1024)
        )));
    }
    let format = image::guess_format(data).map_err(decode_error)?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) {
        return Err(Error::InvalidInput(
            "only PNG and JPEG input is supported".into(),
        ));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_INPUT_PIXELS as u32);
    limits.max_image_height = Some(MAX_INPUT_PIXELS as u32);
    limits.max_alloc = Some(MAX_INPUT_PIXELS * 16);
    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(decode_error)?;
    let (width, height) = decoder.dimensions();
    checked_pixels(width, height).map_err(|_| {
        Error::InvalidInput(format!(
            "image exceeds {MAX_INPUT_PIXELS} input pixels or has a zero dimension"
        ))
    })?;
    let orientation = decoder.orientation().map_err(decode_error)?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    decoded.apply_orientation(orientation);
    if !decoded.color().has_alpha() {
        return Ok(decoded.into_rgb8());
    }
    Ok(crate::color::alpha_on_white(decoded.into_rgba8()))
}

fn capped_size(target: u32, width: u32, height: u32) -> (u32, u32) {
    let factor = (f64::from(target) / f64::from(width.max(height))).min(1.0);
    (
        (f64::from(width) * factor).round_ties_even().max(1.0) as u32,
        (f64::from(height) * factor).round_ties_even().max(1.0) as u32,
    )
}

fn encode_jpeg(image: &RgbImage, quality: u8) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    {
        let mut encoder = Encoder::new(&mut output, quality);
        encoder.set_sampling_factor(SamplingFactor::F_2_2);
        encoder.set_chroma_subsampling_method(ChromaSubsamplingMethod::Average);
        encoder
            .encode(
                image.as_raw(),
                image.width() as u16,
                image.height() as u16,
                ColorType::Rgb,
            )
            .map_err(|error| Error::Processing(format!("cannot encode JPEG: {error}")))?;
    }
    Ok(output)
}

fn system_seed() -> Result<u64> {
    let mut entropy = [0; 4];
    getrandom::fill(&mut entropy)
        .map_err(|error| Error::Processing(format!("cannot obtain random seed: {error}")))?;
    Ok(u64::from(u32::from_le_bytes(entropy) & 0x7fff_ffff))
}

fn render_screen(
    screen: RgbImage,
    seed: Option<u64>,
    style: &Calibration,
) -> Result<(Vec<u8>, u64)> {
    render_screen_with_seed_source(screen, seed, style, system_seed)
}

/// [`render_screen`] with the entropy source injected, so tests can pin and
/// fail seed generation without touching the OS.
fn render_screen_with_seed_source(
    screen: RgbImage,
    seed: Option<u64>,
    style: &Calibration,
    seed_source: impl FnOnce() -> Result<u64>,
) -> Result<(Vec<u8>, u64)> {
    let seed = match seed {
        Some(seed) => seed,
        None => seed_source()?,
    };
    let (source_w, source_h) = screen.dimensions();
    let (w, h) = capped_size(LONG_EDGE_CAP, source_w, source_h);
    let photo = pipeline::capture(screen, w as usize, h as usize, seed, style);
    let jpeg = encode_jpeg(&photo, style.jpeg_quality)?;
    Ok((jpeg, seed))
}

/// PNG/JPEG bytes in, JPEG bytes and the resolved seed out. No files, network
/// or subprocesses. The output long edge is `min(source long edge, 1920)`.
///
/// An omitted seed is drawn once and returned. Reusing it reproduces the
/// capture on the same build/platform.
pub fn render_bytes(data: &[u8], seed: Option<u64>) -> Result<(Vec<u8>, u64)> {
    let screen = decode_image(data)?;
    render_screen(screen, seed, &Calibration::default())
}

/// Raw RGB8 pixels in, JPEG bytes and the resolved seed out.
///
/// Intended for screen capture, where a PNG round-trip would only re-encode
/// the same pixels. `pixels` must be exactly `width * height * 3` bytes.
pub fn render_rgb(
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    seed: Option<u64>,
) -> Result<(Vec<u8>, u64)> {
    checked_pixels(width, height).map_err(|_| {
        Error::InvalidInput(format!(
            "image exceeds {MAX_INPUT_PIXELS} input pixels or has a zero dimension"
        ))
    })?;
    let expected = width as usize * height as usize * 3;
    if pixels.len() != expected {
        return Err(Error::InvalidInput(format!(
            "RGB buffer has {} bytes, expected {expected}",
            pixels.len()
        )));
    }
    let screen = RgbImage::from_raw(width, height, pixels)
        .ok_or_else(|| Error::InvalidInput("cannot build RGB image".into()))?;
    render_screen(screen, seed, &Calibration::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Rgb, Rgba, RgbaImage};

    fn png(image: DynamicImage) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn decodes_gray_and_composites_alpha_on_white() {
        let gray = GrayImage::from_pixel(3, 5, image::Luma([127]));
        assert!(
            decode_image(&png(gray.into()))
                .unwrap()
                .pixels()
                .all(|p| p.0 == [127; 3])
        );
        let rgba = RgbaImage::from_fn(3, 1, |x, _| match x {
            0 => Rgba([0, 0, 0, 0]),
            1 => Rgba([255, 0, 0, 255]),
            _ => Rgba([0, 0, 0, 128]),
        });
        let decoded = decode_image(&png(rgba.into())).unwrap();
        assert_eq!(decoded.get_pixel(0, 0).0, [255; 3]);
        assert_eq!(decoded.get_pixel(1, 0).0, [255, 0, 0]);
        assert_eq!(decoded.get_pixel(2, 0).0, [127; 3]);
    }

    #[test]
    fn rejects_corruption_encoded_limit_and_pixel_bombs() {
        for raw in [&[][..], &b"not an image"[..], &vec![0; MAX_INPUT_BYTES + 1]] {
            assert!(matches!(decode_image(raw), Err(Error::InvalidInput(_))));
        }
        let too_many = png(RgbImage::from_pixel(5001, 5000, Rgb([0; 3])).into());
        assert!(matches!(
            decode_image(&too_many),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn jpeg_orientation_is_applied_and_metadata_is_not_forwarded() {
        let source = RgbImage::from_fn(32, 16, |x, _| {
            if x < 16 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        });
        let raw = encode_jpeg(&source, 95).unwrap();
        let exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut oriented = raw[..2].to_vec();
        oriented.extend_from_slice(&[0xff, 0xe1]);
        oriented.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        oriented.extend_from_slice(exif);
        oriented.extend_from_slice(&raw[2..]);
        let decoded = decode_image(&oriented).unwrap();
        assert_eq!(decoded.dimensions(), (16, 32));
        let top = decoded.get_pixel(8, 4).0;
        let bottom = decoded.get_pixel(8, 28).0;
        assert!(top[0] > top[2].saturating_add(100));
        assert!(bottom[2] > bottom[0].saturating_add(100));
        let (jpeg, resolved) = render_bytes(&oriented, Some(7)).unwrap();
        assert_eq!(resolved, 7);
        let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(&jpeg)).unwrap();
        assert_eq!(decoder.dimensions(), (16, 32));
        assert!(decoder.exif_metadata().unwrap().is_none());
    }

    #[test]
    fn capped_size_fits_long_pages_and_does_not_enlarge() {
        for (cap, width, height, expected) in [
            (1920, 1920, 1080, (1920, 1080)),
            (1920, 1080, 1920, (1080, 1920)),
            (1920, 1080, 10000, (207, 1920)),
            (1920, 320, 181, (320, 181)),
            (64, 24000, 1, (64, 1)),
        ] {
            assert_eq!(capped_size(cap, width, height), expected);
        }
    }

    #[test]
    fn seed_source_is_used_only_when_the_seed_is_omitted() {
        let style = Calibration::default();
        let screen = || RgbImage::from_pixel(4, 3, Rgb([9, 8, 7]));
        let source = || Ok(11);
        let (_, seed) = render_screen_with_seed_source(screen(), None, &style, source).unwrap();
        assert_eq!(seed, 11);

        let source = || panic!("explicit seeds must not consult the entropy source");
        let (_, seed) = render_screen_with_seed_source(screen(), Some(7), &style, source).unwrap();
        assert_eq!(seed, 7);
    }

    #[test]
    fn seed_source_errors_are_processing_failures() {
        let screen = RgbImage::from_pixel(4, 3, Rgb([9, 8, 7]));
        let source = || Err(Error::Processing("entropy unavailable".to_owned()));
        let error = render_screen_with_seed_source(screen, None, &Calibration::default(), source)
            .expect_err("an erroring entropy source must surface");
        assert!(matches!(error, Error::Processing(message) if message == "entropy unavailable"));
    }
}
