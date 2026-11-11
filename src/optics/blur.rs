//! Defocus, gaussian primitives and vignetting.

use crate::frame::Frame;

use super::resample::resize;

/// One separable box pass with edge replication. `r == 1` is the 3-tap box
/// used by the subpixel branch; larger radii use exact prefix sums.
pub(super) fn box_pass(
    src: &[[f32; 3]],
    dst: &mut [[f32; 3]],
    width: usize,
    height: usize,
    radius: usize,
    vertical: bool,
    prefix: &mut Vec<[f64; 3]>,
) {
    let (length, lines) = if vertical {
        (height, width)
    } else {
        (width, height)
    };
    if radius == 0 || length == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let index = |line: usize, i: usize| {
        if vertical {
            i * width + line
        } else {
            line * width + i
        }
    };
    if radius == 1 {
        let last = length - 1;
        for line in 0..lines {
            for i in 0..length {
                let below = index(line, i.saturating_sub(1));
                let here = index(line, i);
                let above = index(line, (i + 1).min(last));
                for c in 0..3 {
                    dst[here][c] = (src[below][c] + src[here][c] + src[above][c]) * (1.0 / 3.0);
                }
            }
        }
        return;
    }
    let span = (2 * radius + 1) as f64;
    for line in 0..lines {
        prefix.clear();
        prefix.push([0.0; 3]);
        for i in 0..length {
            let sample = src[index(line, i)];
            let previous = prefix[i];
            prefix.push([
                previous[0] + sample[0] as f64,
                previous[1] + sample[1] as f64,
                previous[2] + sample[2] as f64,
            ]);
        }
        let first = src[index(line, 0)];
        let last = src[index(line, length - 1)];
        for i in 0..length {
            let low = i.saturating_sub(radius);
            let high = (i + radius).min(length - 1);
            let below = (radius + low - i) as f64;
            let above = (i + radius - high) as f64;
            let here = index(line, i);
            for c in 0..3 {
                let window = prefix[high + 1][c] - prefix[low][c];
                let sum = window + below * first[c] as f64 + above * last[c] as f64;
                dst[here][c] = (sum / span) as f32;
            }
        }
    }
}

/// One separable axis of the defocus kernel, shared by the spatial filter and
/// the display-carrier frequency response so both consume the same coefficients.
///
/// `Subpixel` is `(1 - weight) * identity + weight * box3`; `Boxes` is three
/// passes of `(1 - frac) * box(radius) + frac * box(radius + 1)`. The plan is
/// derived once per sigma; the pixel loops only consume it.
#[derive(Clone, Copy, Debug)]
pub(super) enum AxisKernel {
    Subpixel { weight: f32 },
    Boxes { radius: usize, frac: f32 },
}

impl AxisKernel {
    /// `None` is the identity kernel (`sigma <= 0.02`).
    pub(super) fn new(sigma: f32) -> Option<Self> {
        if sigma <= 0.02 {
            None
        } else if sigma <= 0.7 {
            Some(Self::Subpixel {
                weight: 1.5 * sigma * sigma,
            })
        } else {
            let exact = ((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0;
            let radius = exact.floor() as usize;
            let frac =
                (sigma * sigma - (radius * (radius + 1)) as f32) / (2.0 * (radius + 1) as f32);
            Some(Self::Boxes { radius, frac })
        }
    }

    /// Exact one-axis frequency response at `frequency` cycles per pixel. The
    /// 2D response of the separable kernel is the product of both axes.
    #[inline]
    pub(super) fn response(&self, frequency: f64) -> f64 {
        match *self {
            Self::Subpixel { weight } => subpixel_response(weight, box_response(1, frequency)),
            Self::Boxes { radius, frac } => {
                let frac = frac as f64;
                let pass = (1.0 - frac) * box_response(radius, frequency)
                    + frac * box_response(radius + 1, frequency);
                pass * pass * pass
            }
        }
    }

    /// The default two subpixel kernels share their trigonometric samples.
    #[inline]
    pub(super) fn paired_response(self, other: Self, fx: f64, fy: f64) -> (f64, f64) {
        match (self, other) {
            (Self::Subpixel { weight: a }, Self::Subpixel { weight: b }) => {
                let (bx, by) = (box_response(1, fx), box_response(1, fy));
                let response =
                    |weight| subpixel_response(weight, bx) * subpixel_response(weight, by);
                (response(a), response(b))
            }
            _ => (
                self.response(fx) * self.response(fy),
                other.response(fx) * other.response(fy),
            ),
        }
    }
}

#[inline]
fn subpixel_response(weight: f32, box_gain: f64) -> f64 {
    let weight = weight as f64;
    1.0 - weight + weight * box_gain
}

/// DTFT of a normalised box of `2 * radius + 1` taps.
#[inline]
fn box_response(radius: usize, frequency: f64) -> f64 {
    if radius == 0 {
        return 1.0;
    }
    if radius == 1 {
        return (1.0 + 2.0 * (std::f64::consts::TAU * frequency).cos()) / 3.0;
    }
    let span = (2 * radius + 1) as f64;
    let x = std::f64::consts::PI * frequency;
    let denominator = span * x.sin();
    if denominator.abs() < 1e-12 {
        return 1.0;
    }
    (span * x).sin() / denominator
}

#[inline]
pub(super) fn focal_weight(dx: f32, dy: f32) -> f32 {
    (dx * dx + dy * dy).clamp(0.0, 1.0)
}

/// Separable Gaussian approximation with variance `sigma**2` per axis.
/// Consumes and reuses the source buffer; a tiny sigma is the identity.
pub(super) fn gaussian_blur(src: Frame, sigma: f32) -> Frame {
    let (width, height, mut pixels) = src.into_parts();
    if width == 0 || height == 0 {
        return Frame::new(width, height, pixels);
    }
    let Some(kernel) = AxisKernel::new(sigma) else {
        return Frame::new(width, height, pixels);
    };
    let len = width * height;
    let mut buffer = vec![[0.0_f32; 3]; len];
    match kernel {
        AxisKernel::Subpixel { weight } => {
            let mut prefix = Vec::new();
            for vertical in [true, false] {
                box_pass(
                    &pixels,
                    &mut buffer,
                    width,
                    height,
                    1,
                    vertical,
                    &mut prefix,
                );
                for i in 0..len {
                    for c in 0..3 {
                        pixels[i][c] = pixels[i][c] * (1.0 - weight) + buffer[i][c] * weight;
                    }
                }
            }
        }
        AxisKernel::Boxes { radius, frac } => {
            let mut prefix = Vec::with_capacity(width.max(height) + 1);
            let mut low = vec![[0.0_f32; 3]; len];
            for vertical in [true, false] {
                for _ in 0..3 {
                    if radius >= 1 {
                        box_pass(
                            &pixels,
                            &mut low,
                            width,
                            height,
                            radius,
                            vertical,
                            &mut prefix,
                        );
                    } else {
                        low.copy_from_slice(&pixels);
                    }
                    box_pass(
                        &pixels,
                        &mut buffer,
                        width,
                        height,
                        radius + 1,
                        vertical,
                        &mut prefix,
                    );
                    for i in 0..len {
                        for c in 0..3 {
                            pixels[i][c] = low[i][c] * (1.0 - frac) + buffer[i][c] * frac;
                        }
                    }
                }
            }
        }
    }
    Frame::new(width, height, pixels)
}

pub(super) fn blur_lowres(src: Frame, sigma: f32) -> Frame {
    let factor = super::lowres_factor(src.width, src.height);
    if sigma <= 0.02 || factor <= 1 {
        return gaussian_blur(src, sigma);
    }
    let small_width = (src.width / factor).max(2);
    let small_height = (src.height / factor).max(2);
    let small = resize(&src, small_width, small_height);
    let small = gaussian_blur(small, sigma / factor as f32);
    resize(&small, src.width, src.height)
}

/// Defocus plus the tilted focal plane around a shared centre.
pub(crate) fn defocus(src: Frame, base: f32, tilt: f32, center: (f32, f32)) -> Frame {
    if tilt <= 1e-4 {
        return gaussian_blur(src, base);
    }
    let blurred = gaussian_blur(src.clone(), base + tilt);
    let mut focused = gaussian_blur(src, base);
    let (width, height) = (focused.width, focused.height);
    let half = width.max(height) as f32 * 0.5;
    let blurred_pixels = blurred.pixels();
    let focused_pixels = focused.pixels_mut();
    for y in 0..height {
        let dy = ((y as f32 + 0.5) - center.1 * height as f32) / half;
        for x in 0..width {
            let dx = ((x as f32 + 0.5) - center.0 * width as f32) / half;
            let weight = focal_weight(dx, dy);
            let index = y * width + x;
            for c in 0..3 {
                focused_pixels[index][c] =
                    focused_pixels[index][c] * (1.0 - weight) + blurred_pixels[index][c] * weight;
            }
        }
    }
    focused
}

/// Vignette falloff shape and strength.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VignetteStyle {
    pub amount: f32,
    pub power: f32,
    pub aspect: (f32, f32),
    pub floor: f32,
}

pub(crate) fn vignette(src: &mut Frame, style: VignetteStyle, center: (f32, f32)) {
    let (width, height) = (src.width, src.height);
    if width == 0 || height == 0 {
        return;
    }
    let (width_f, height_f) = (width as f32, height as f32);
    let pixels = src.pixels_mut();
    for (y, row) in pixels.chunks_mut(width).enumerate() {
        let ny = (y as f32 + 0.5) / height_f - center.1;
        for (x, pixel) in row.iter_mut().enumerate() {
            let nx = (x as f32 + 0.5) / width_f - center.0;
            let radius = (nx * style.aspect.0).hypot(ny * style.aspect.1);
            let fall = (1.0 - style.amount * radius.powf(style.power)).clamp(style.floor, 1.0);
            for value in pixel {
                *value *= fall;
            }
        }
    }
}
