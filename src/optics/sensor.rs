//! Sensor lattice: shot/chroma/band noise and CFA reconstruction.

use crate::color::luma;
use crate::frame::Frame;
use crate::rng::Rng;

use super::blur::gaussian_blur;
use super::resample::resize;

/// Sensor noise coefficients owned by the style.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SensorStyle {
    pub shot_scale: f32,
    pub gain_power: f32,
    pub floor: f32,
    pub chroma_scale: f32,
    pub chroma_gain_power: f32,
    pub chroma_blur: f32,
    pub band_scale: f32,
    pub band_blur: f32,
}

fn smooth_pass(src: &[f32], dst: &mut [f32], width: usize, height: usize, vertical: bool) {
    if vertical {
        for x in 0..width {
            for y in 0..height {
                let above = if y == 0 { 0 } else { y - 1 };
                let below = if y + 1 >= height { height - 1 } else { y + 1 };
                dst[y * width + x] =
                    (src[above * width + x] + 2.0 * src[y * width + x] + src[below * width + x])
                        * 0.25;
            }
        }
    } else {
        for y in 0..height {
            let row = y * width;
            for x in 0..width {
                let left = if x == 0 { 0 } else { x - 1 };
                let right = if x + 1 >= width { width - 1 } else { x + 1 };
                dst[row + x] = (src[row + left] + 2.0 * src[row + x] + src[row + right]) * 0.25;
            }
        }
    }
}

fn cfa_gather(
    src: &[[f32; 3]],
    width: usize,
    height: usize,
    phases: &[(usize, usize)],
    channel: usize,
    mosaic: &mut [f32],
) {
    mosaic.fill(0.0);
    for &(start_y, start_x) in phases {
        let mut y = start_y;
        while y < height {
            let mut x = start_x;
            while x < width {
                mosaic[y * width + x] = src[y * width + x][channel];
                x += 2;
            }
            y += 2;
        }
    }
}

fn smooth_phase(mosaic: &mut [f32], scratch: &mut [f32], width: usize, height: usize) {
    smooth_pass(mosaic, scratch, width, height, true);
    smooth_pass(scratch, mosaic, width, height, true);
    smooth_pass(mosaic, scratch, width, height, false);
    smooth_pass(scratch, mosaic, width, height, false);
}

fn parity_response(length: usize) -> (Vec<f32>, Vec<f32>) {
    let mut even = vec![0.0_f32; length];
    let mut odd = vec![0.0_f32; length];
    for i in 0..length {
        if i % 2 == 0 {
            even[i] = 1.0;
        } else {
            odd[i] = 1.0;
        }
    }
    for response in [&mut even, &mut odd] {
        let mut scratch = vec![0.0_f32; length];
        for _ in 0..2 {
            smooth_pass(response.as_slice(), &mut scratch, length, 1, false);
            std::mem::swap(&mut *response, &mut scratch);
        }
    }
    (even, odd)
}

/// RGGB mosaic and bilinear demosaic on the capture sensor grid.
///
/// The separable filter is evaluated per CFA phase, so the cost is one gather
/// and two blend passes instead of four full-size masks. Incomplete cells
/// (a one-pixel axis) keep their input colours rather than inventing channels.
pub(crate) fn bayer_artifacts(src: &mut Frame, strength: f32) {
    let (width, height) = (src.width, src.height);
    if strength <= 0.01 || width.min(height) < 2 {
        return;
    }
    let (even_y, odd_y) = parity_response(height);
    let (even_x, odd_x) = parity_response(width);
    let len = width * height;
    let mut mosaic = vec![0.0_f32; len];
    let mut scratch = vec![0.0_f32; len];

    cfa_gather(src.pixels(), width, height, &[(0, 0)], 0, &mut mosaic);
    smooth_phase(&mut mosaic, &mut scratch, width, height);
    {
        let pixels = src.pixels_mut();
        for (y, &y_response) in even_y.iter().enumerate() {
            for (x, &x_response) in even_x.iter().enumerate() {
                let index = y * width + x;
                let denominator = y_response * x_response;
                let recovered = if denominator > 0.0 {
                    (mosaic[index] / denominator).clamp(0.0, 4.0)
                } else {
                    0.0
                };
                pixels[index][0] = pixels[index][0] * (1.0 - strength) + recovered * strength;
            }
        }
    }

    cfa_gather(
        src.pixels(),
        width,
        height,
        &[(0, 1), (1, 0)],
        1,
        &mut mosaic,
    );
    smooth_phase(&mut mosaic, &mut scratch, width, height);
    {
        let pixels = src.pixels_mut();
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let denominator = even_y[y] * odd_x[x] + odd_y[y] * even_x[x];
                let recovered = if denominator > 0.0 {
                    (mosaic[index] / denominator).clamp(0.0, 4.0)
                } else {
                    0.0
                };
                pixels[index][1] = pixels[index][1] * (1.0 - strength) + recovered * strength;
            }
        }
    }

    cfa_gather(src.pixels(), width, height, &[(1, 1)], 2, &mut mosaic);
    smooth_phase(&mut mosaic, &mut scratch, width, height);
    {
        let pixels = src.pixels_mut();
        for (y, &y_response) in odd_y.iter().enumerate() {
            for (x, &x_response) in odd_x.iter().enumerate() {
                let index = y * width + x;
                let denominator = y_response * x_response;
                let recovered = if denominator > 0.0 {
                    (mosaic[index] / denominator).clamp(0.0, 4.0)
                } else {
                    0.0
                };
                pixels[index][2] = pixels[index][2] * (1.0 - strength) + recovered * strength;
            }
        }
    }
}

pub(crate) fn sensor_noise(
    src: &mut Frame,
    rng: &mut Rng,
    iso: f32,
    chroma: f32,
    style: SensorStyle,
) {
    let (width, height) = (src.width, src.height);
    if width == 0 || height == 0 {
        return;
    }
    let gain = iso / 100.0;
    let shot_scale = style.shot_scale * gain.powf(style.gain_power);
    for pixel in src.pixels_mut() {
        let lum = luma(*pixel).clamp(0.0, 1.0);
        let sigma = shot_scale * (lum + style.floor).sqrt();
        let shot = rng.normal() * sigma;
        for value in pixel {
            *value += shot;
        }
    }

    if chroma > 0.0 {
        let factor = super::lowres_factor(width, height);
        let small_width = (width / factor).max(1);
        let small_height = (height / factor).max(1);
        let mut noise = Frame::zeros(small_width, small_height);
        for pixel in noise.pixels_mut() {
            *pixel = [rng.normal(), rng.normal(), rng.normal()];
        }
        let noise = gaussian_blur(noise, style.chroma_blur / factor as f32);
        let noise = if factor > 1 {
            resize(&noise, width, height)
        } else {
            noise
        };
        let scale = style.chroma_scale * gain.powf(style.chroma_gain_power) * chroma;
        for (pixel, tint) in src.pixels_mut().iter_mut().zip(noise.pixels()) {
            for c in 0..3 {
                pixel[c] += tint[c] * scale;
            }
        }
    }

    let mut bands = Frame::zeros(1, height);
    for y in 0..height {
        let value = rng.normal();
        bands.pixels_mut()[y] = [value; 3];
    }
    let bands = gaussian_blur(bands, style.band_blur);
    let pixels = src.pixels_mut();
    for y in 0..height {
        let value = bands.pixels()[y][0] * style.band_scale;
        for x in 0..width {
            let pixel = &mut pixels[y * width + x];
            for channel in pixel {
                *channel += value;
            }
        }
    }
}
