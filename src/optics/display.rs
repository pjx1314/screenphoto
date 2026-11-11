//! Display carriers, glass reflection and bloom.

use crate::color::luma;
use crate::frame::Frame;
use crate::geometry::{Lens, Mat3, lens_coords, lens_jacobian};
use crate::rng::Rng;

use super::blur::{AxisKernel, blur_lowres, focal_weight, gaussian_blur};
use super::resample::resize;

/// Display carrier coefficients owned by the style. The defocus kernel is
/// shared with the spatial stage because it also sets the carrier response.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DisplayStyle {
    pub moire: f32,
    pub pitch: f32,
    pub aperture: f64,
    pub row_modulation: f64,
    pub defocus: f32,
    pub tilt_defocus: f32,
}

/// Per-capture display sampling resolved from the style and the `sampling`
/// RNG stream: jittered pitch, lens and phase only.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sampling {
    pub pitch: f32,
    pub lens: Lens,
    pub phase: (f32, f32),
}

/// Glass-reflection falloff, tint and blur.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReflectionStyle {
    pub gradient: f32,
    pub base: f32,
    pub tint: [f32; 3],
    pub blur: f32,
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let pix = std::f64::consts::PI * x;
        pix.sin() / pix
    }
}

/// Signed 2D response of the separable axis kernel at a frame-space frequency.
#[inline]
fn carrier_response(kernel: Option<AxisKernel>, fx: f64, fy: f64) -> f64 {
    match kernel {
        Some(kernel) => kernel.response(fx) * kernel.response(fy),
        None => 1.0,
    }
}

/// Sample a virtual RGB-stripe display before CFA reconstruction.
///
/// The two Fourier carriers (row gaps plus three RGB stripes) are attenuated
/// by the same discrete defocus kernel as the spatial stage, mixed by the same
/// focal-plane weight, and by the pixel-aperture MTF. Both the screenshots and
/// the carriers use the same inverse lens field, so colour beats land on the
/// sensor lattice instead of being painted on the JPEG.
pub(crate) fn display_sampling(
    src: &mut Frame,
    h_inv: &Mat3,
    style: DisplayStyle,
    settings: Sampling,
) {
    if style.moire <= 1e-4 || src.width == 0 || src.height == 0 {
        return;
    }
    let (width, height) = (src.width, src.height);
    let center = settings.lens.center;
    let cx = center.0 as f64 * width as f64;
    let cy = center.1 as f64 * height as f64;
    let m = *h_inv;
    let den0 = m[2][0] * cx + m[2][1] * cy + m[2][2];
    if den0 == 0.0 || !den0.is_finite() {
        return;
    }
    let u0 = (m[0][0] * cx + m[0][1] * cy + m[0][2]) / den0;
    let du0x = (m[0][0] - u0 * m[2][0]) / den0;
    let du0y = (m[0][1] - u0 * m[2][1]) / den0;
    let period = (du0x * du0x + du0y * du0y).sqrt().max(1e-6) * settings.pitch as f64;
    let inv_period = 1.0 / period;
    let half = width.max(height) as f32 * 0.5;
    let focal_center = (center.0 * width as f32, center.1 * height as f32);
    let (phase_u, phase_v) = (settings.phase.0 as f64, settings.phase.1 as f64);
    let moire = style.moire as f64;
    let base_kernel = AxisKernel::new(style.defocus);
    let tilted_kernel = if style.tilt_defocus > 1e-4 {
        AxisKernel::new(style.defocus + style.tilt_defocus)
    } else {
        None
    };
    // The default lens is distortion-free: skip the radial map and its
    // Jacobian (both identities) instead of evaluating them per pixel.
    let radial = settings.lens.distortion != 0.0;
    let pixels = src.pixels_mut();
    for y in 0..height {
        let ys = y as f64 + 0.5;
        let dy = ((y as f32 + 0.5) - focal_center.1) / half;
        for x in 0..width {
            let xs = x as f64 + 0.5;
            let (lx, ly, jxx, jxy, jyy) = if radial {
                let (lx, ly) = lens_coords(xs, ys, width, height, settings.lens);
                let jacobian = lens_jacobian(xs, ys, width, height, settings.lens);
                (lx, ly, jacobian[0][0], jacobian[0][1], jacobian[1][1])
            } else {
                (xs, ys, 1.0, 0.0, 1.0)
            };
            let den = m[2][0] * lx + m[2][1] * ly + m[2][2];
            if den == 0.0 || !den.is_finite() {
                continue;
            }
            let inv_den = 1.0 / den;
            let inv_den_period = inv_den * inv_period;
            let u = (m[0][0] * lx + m[0][1] * ly + m[0][2]) * inv_den;
            let v = (m[1][0] * lx + m[1][1] * ly + m[1][2]) * inv_den;
            let ux = (m[0][0] - u * m[2][0]) * inv_den_period;
            let uy = (m[0][1] - u * m[2][1]) * inv_den_period;
            let vx = (m[1][0] - v * m[2][0]) * inv_den_period;
            let vy = (m[1][1] - v * m[2][1]) * inv_den_period;
            let (fu_x, fu_y, fv_x, fv_y) = if radial {
                (
                    ux * jxx + uy * jxy,
                    ux * jxy + uy * jyy,
                    vx * jxx + vy * jxy,
                    vx * jxy + vy * jyy,
                )
            } else {
                (ux, uy, vx, vy)
            };
            let fu = (fu_x * fu_x + fu_y * fu_y).sqrt();
            let fv = (fv_x * fv_x + fv_y * fv_y).sqrt();
            let (mut au, mut av) = if let Some(tilted) = tilted_kernel {
                let dx = ((x as f32 + 0.5) - focal_center.0) / half;
                let weight = focal_weight(dx, dy) as f64;
                let response = |fx, fy| {
                    let (base, edge) = match base_kernel {
                        Some(base) => base.paired_response(tilted, fx, fy),
                        None => (1.0, carrier_response(Some(tilted), fx, fy)),
                    };
                    base * (1.0 - weight) + edge * weight
                };
                (response(fu_x, fu_y), response(fv_x, fv_y))
            } else {
                (
                    carrier_response(base_kernel, fu_x, fu_y),
                    carrier_response(base_kernel, fv_x, fv_y),
                )
            };
            au *= moire;
            av *= moire;
            au *= sinc(fu * style.aperture).clamp(0.0, 1.0);
            av *= sinc(fv * style.aperture).clamp(0.0, 1.0);
            let pu = std::f64::consts::TAU * (u * inv_period + phase_u);
            let pv = std::f64::consts::TAU * (v * inv_period + phase_v);
            let (sin_u, cos_u) = pu.sin_cos();
            let rows = 1.0 + style.row_modulation * av * pv.cos();
            let pixel = &mut pixels[y * width + x];
            pixel[0] *= (rows * (1.0 + au * cos_u)) as f32;
            pixel[1] *= (rows * (1.0 - au * (0.5 * cos_u + 0.866_025_4 * sin_u))) as f32;
            pixel[2] *= (rows * (1.0 - au * (0.5 * cos_u - 0.866_025_4 * sin_u))) as f32;
        }
    }
}

/// The soft sky patch reflected by the glass, in screen space.
///
/// The patch is a low-resolution cubic falloff whose direction stays within
/// 60° of up, with a bounded horizontal offset. The stream draws the tilt
/// angle, then the offset, before the pixel loop, so the layer is finite and
/// reproducible.
pub(crate) fn sky_reflection(
    width: usize,
    height: usize,
    intensity: f32,
    style: ReflectionStyle,
    rng: &mut Rng,
) -> Frame {
    if intensity <= 0.0 || width == 0 || height == 0 {
        return Frame::zeros(width, height);
    }
    let angle = rng.uniform(-60.0, 60.0).to_radians();
    let (direction_x, direction_y) = angle.sin_cos();
    let offset_x = rng.uniform(-0.25, 0.25);
    let quarter = 4;
    let small_width = (width / quarter).max(2);
    let small_height = (height / quarter).max(2);
    let mut small = Frame::zeros(small_width, small_height);
    let x_scale = small_width.saturating_sub(1).max(1) as f32;
    let y_scale = small_height.saturating_sub(1).max(1) as f32;
    let x_step = 1.0 / x_scale;
    for (y, row) in small.pixels_mut().chunks_mut(small_width).enumerate() {
        let yn = y as f32 / y_scale;
        let row_t = 0.5 + (yn - 0.5) * direction_y;
        for (x, pixel) in row.iter_mut().enumerate() {
            let xn = x as f32 * x_step;
            let t = row_t + (xn - 0.5 - offset_x) * direction_x;
            let grad = (1.0 - t * style.gradient).clamp(0.0, 1.0).powi(3) * style.base;
            *pixel = [
                grad * style.tint[0],
                grad * style.tint[1],
                grad * style.tint[2],
            ];
        }
    }
    let sigma = (style.blur * width.min(height) as f32 / quarter as f32).max(1.0);
    let blurred = gaussian_blur(small, sigma);
    let mut up = resize(&blurred, width, height);
    for pixel in up.pixels_mut() {
        for value in pixel {
            *value = (*value * intensity).max(0.0);
        }
    }
    up
}

pub(crate) fn bloom_layer(src: &Frame, threshold: f32, radius: f32, strength: f32) -> Frame {
    if strength <= 0.0 || src.width == 0 || src.height == 0 {
        return Frame::zeros(src.width, src.height);
    }
    let mut high = Frame::zeros(src.width, src.height);
    for (index, pixel) in src.pixels().iter().enumerate() {
        let lum = luma(*pixel);
        let weight = (lum - threshold).max(0.0);
        if weight > 0.0 {
            high.pixels_mut()[index] = [pixel[0] * weight, pixel[1] * weight, pixel[2] * weight];
        }
    }
    let mut high = blur_lowres(high, radius);
    for pixel in high.pixels_mut() {
        for value in pixel {
            *value *= strength;
        }
    }
    high
}
