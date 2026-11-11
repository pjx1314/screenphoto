//! Colour: the one place that knows the sRGB transfer function, the display
//! emission level and Rec.709 luminance.
//!
//! Every stage works in linear light; the photo is encoded to sRGB exactly
//! once, in [`tone_encode`]. Metering, white balance and the tone curve live
//! here so no other module owns colour constants.

use std::sync::LazyLock;

use crate::frame::Frame;
use crate::rng::Rng;

/// Emission of a lit white screen relative to a perfect white; black emission
/// stays at zero until reflection or sensor noise lifts it.
pub(crate) const DISPLAY_EMISSION: f32 = DISPLAY_EMISSION_F64 as f32;

/// Metering uses the same emission value in f64; rendering derives its f32 value.
const DISPLAY_EMISSION_F64: f64 = 0.92;

/// Rec.709 luminance weights, shared by metering, bloom and sensor noise.
const LUMA_WEIGHTS: [f32; 3] = [0.2126, 0.7152, 0.0722];

#[inline]
pub(crate) fn luma(pixel: [f32; 3]) -> f32 {
    LUMA_WEIGHTS[0] * pixel[0] + LUMA_WEIGHTS[1] * pixel[1] + LUMA_WEIGHTS[2] * pixel[2]
}

/// Composite decoded alpha onto white, packing RGB in the existing buffer.
pub(crate) fn alpha_on_white(rgba: image::RgbaImage) -> image::RgbImage {
    let (width, height) = rgba.dimensions();
    let mut pixels = rgba.into_raw();
    for index in 0..width as usize * height as usize {
        let offset = index * 4;
        let alpha = u32::from(pixels[offset + 3]);
        let rgb = [pixels[offset], pixels[offset + 1], pixels[offset + 2]];
        for channel in 0..3 {
            pixels[index * 3 + channel] =
                ((u32::from(rgb[channel]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
        }
    }
    pixels.truncate(width as usize * height as usize * 3);
    image::RgbImage::from_raw(width, height, pixels).expect("packed decoded RGB dimensions")
}

static SRGB_LUT: LazyLock<[f32; 256]> = LazyLock::new(|| {
    std::array::from_fn(|code| {
        let value = code as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    })
});

/// sRGB byte code -> linear light, through the exact piecewise transfer
/// function.
pub(crate) fn srgb_lut() -> &'static [f32; 256] {
    &SRGB_LUT
}

const TONE_STEPS: usize = 4096;
const TONE_SCALE: f32 = (TONE_STEPS as f32 - 1.0) / 4.0;

fn linear_to_srgb(x: f32) -> f32 {
    let a = x.clamp(0.0, 1.0);
    if a <= 0.003_130_8 {
        a * 12.92
    } else {
        1.055 * a.powf(1.0 / 2.4) - 0.055
    }
}

fn filmic(x: f32) -> f32 {
    let shoulder = 0.82_f32;
    let y = if x < shoulder {
        x
    } else {
        shoulder + (1.0 - shoulder) * (1.0 - (-(x - shoulder) / (1.0 - shoulder)).exp())
    };
    y.clamp(0.0, 1.0)
}

static TONE_LUT: LazyLock<[f32; TONE_STEPS]> = LazyLock::new(|| {
    std::array::from_fn(|index| {
        let x = 4.0 * index as f32 / (TONE_STEPS - 1) as f32;
        linear_to_srgb(filmic(x))
    })
});

/// Filmic shoulder and sRGB encode through one fixed lookup table.
pub(crate) fn tone_encode(src: &mut Frame) {
    let lut = &*TONE_LUT;
    for pixel in src.pixels_mut() {
        for value in pixel {
            let scaled = (*value * TONE_SCALE).clamp(0.0, (TONE_STEPS - 1) as f32);
            *value = lut[scaled as usize];
        }
    }
}

/// Decode every pixel through per-channel code histograms in linear light.
/// The result depends on the colour multiset rather than on pixel phase.
/// Returns scene factor and black lift.
pub(crate) fn auto_exposure(src: &image::RgbImage) -> (f32, f32) {
    let pixels = u64::from(src.width()) * u64::from(src.height());
    if pixels == 0 {
        return (1.0, 0.0);
    }
    // Luminance is a linear sum, so separate channel bins avoid a joint RGB table.
    let mut histogram = [[0_u32; 256]; 3];
    for pixel in src.pixels() {
        for (bins, code) in histogram.iter_mut().zip(pixel.0) {
            bins[usize::from(code)] += 1;
        }
    }
    let lut = srgb_lut();
    let mut sum = 0.0_f64;
    for (bins, weight) in histogram.iter().zip(LUMA_WEIGHTS) {
        let channel: f64 = bins
            .iter()
            .zip(lut)
            .map(|(&count, &linear)| f64::from(count) * f64::from(linear))
            .sum();
        sum += channel * f64::from(weight);
    }
    let lum = (DISPLAY_EMISSION_F64 * sum / pixels as f64) as f32;
    let factor = ((0.28 / lum.max(1e-4)).powf(0.45)).clamp(0.82, 1.70);
    let lift = ((0.20 - lum) * 0.03).clamp(0.0, 0.008);
    (factor, lift)
}

/// Random per-channel white-balance gains, the small colour cast of a phone
/// ISP under fixed lighting.
pub(crate) fn white_balance_shift(src: &mut Frame, rng: &mut Rng, amount: f32) {
    let gains = [
        1.0 + rng.normal() * amount,
        1.0 + rng.normal() * (amount * 0.6),
        1.0 + rng.normal() * amount,
    ];
    for pixel in src.pixels_mut() {
        for c in 0..3 {
            pixel[c] *= gains[c];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::auto_exposure;

    fn cyclic_shift(image: &image::RgbImage, dx: u32, dy: u32) -> image::RgbImage {
        let (width, height) = image.dimensions();
        image::RgbImage::from_fn(width, height, |x, y| {
            *image.get_pixel((x + width - dx) % width, (y + height - dy) % height)
        })
    }

    fn stripes(horizontal: bool) -> image::RgbImage {
        let shades = [[0, 0, 0], [128, 128, 128], [255, 255, 255]];
        image::RgbImage::from_fn(16, 16, |x, y| {
            let axis = if horizontal { x } else { y };
            image::Rgb(shades[axis as usize % shades.len()])
        })
    }

    #[test]
    fn metering_ignores_horizontal_and_vertical_phase() {
        for horizontal in [true, false] {
            let image = stripes(horizontal);
            let expected = auto_exposure(&image);
            for shift in 1..=2 {
                let moved = if horizontal {
                    cyclic_shift(&image, shift, 0)
                } else {
                    cyclic_shift(&image, 0, shift)
                };
                assert_eq!(
                    auto_exposure(&moved),
                    expected,
                    "{} shift by {shift} changed metering",
                    if horizontal { "horizontal" } else { "vertical" }
                );
            }
        }
    }

    #[test]
    fn metering_counts_pixels_on_the_far_edges() {
        let plain = image::RgbImage::from_pixel(19, 17, image::Rgb([128, 128, 128]));
        let mut edged = plain.clone();
        edged.put_pixel(18, 1, image::Rgb([255, 255, 255]));
        edged.put_pixel(1, 16, image::Rgb([255, 255, 255]));
        let plain_exposure = auto_exposure(&plain);
        let edged_exposure = auto_exposure(&edged);
        assert!(
            edged_exposure.0 < plain_exposure.0 - 1e-3,
            "edge pixels did not lower the exposure factor: {plain_exposure:?} vs {edged_exposure:?}"
        );
    }

    #[test]
    fn metering_averages_light_not_srgb_codes() {
        let mixed = image::RgbImage::from_fn(2, 1, |x, _| {
            image::Rgb(if x == 0 { [0; 3] } else { [255; 3] })
        });
        let grey = image::RgbImage::from_pixel(2, 1, image::Rgb([128; 3]));
        assert!(
            auto_exposure(&mixed).0 < auto_exposure(&grey).0,
            "black/white light average must meter brighter than mid-code grey"
        );
    }
}
