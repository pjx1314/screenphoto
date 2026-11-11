//! Encoded-image sharpening and quantisation.

use crate::frame::Frame;

use super::blur::box_pass;

/// Modest unsharp mask on the encoded image: the halo a phone ISP leaves
/// around its own fonts.
pub(crate) fn sharpen(src: Frame, amount: f32) -> Frame {
    let (width, height, mut pixels) = src.into_parts();
    if amount <= 1e-4 || width == 0 || height == 0 {
        return Frame::new(width, height, pixels);
    }
    let len = width * height;
    let mut first = vec![[0.0_f32; 3]; len];
    let mut blurred = vec![[0.0_f32; 3]; len];
    let mut prefix = Vec::new();
    box_pass(&pixels, &mut first, width, height, 1, true, &mut prefix);
    box_pass(&first, &mut blurred, width, height, 1, false, &mut prefix);
    for i in 0..len {
        for c in 0..3 {
            let value = pixels[i][c];
            pixels[i][c] = (value + (value - blurred[i][c]) * amount).clamp(0.0, 1.0);
        }
    }
    Frame::new(width, height, pixels)
}

pub(crate) fn to_uint8(src: &Frame) -> image::RgbImage {
    let mut bytes = Vec::with_capacity(src.pixels().len() * 3);
    for pixel in src.pixels() {
        for value in pixel {
            bytes.push((value * 255.0 + 0.5).clamp(0.0, 255.0) as u8);
        }
    }
    image::RgbImage::from_raw(src.width as u32, src.height as u32, bytes)
        .expect("frame dimensions fit an RgbImage")
}
