use super::blur::{AxisKernel, gaussian_blur};
use super::*;
use crate::frame::Frame;
use crate::geometry::{Lens, Mat3};
use crate::rng::Rng;

const H_INV: Mat3 = [
    [1.03, 0.04, -0.6],
    [0.02, 0.97, 0.4],
    [1.1e-4, -0.8e-4, 1.0],
];

const REFLECTION: ReflectionStyle = ReflectionStyle {
    gradient: 1.15,
    base: 0.55,
    tint: [0.84, 0.93, 1.12],
    blur: 0.004,
};

const REFLECTION_STREAM: u64 = 4;

fn plane_with_lens(
    width: usize,
    height: usize,
    value: f32,
    defocus: f32,
    tilt_defocus: f32,
    distortion: f32,
) -> Frame {
    let mut frame = Frame::zeros(width, height);
    frame.pixels_mut().fill([value; 3]);
    display_sampling(
        &mut frame,
        &H_INV,
        DisplayStyle {
            moire: 0.65,
            pitch: 2.06,
            aperture: 0.8,
            row_modulation: 0.65,
            defocus,
            tilt_defocus,
        },
        Sampling {
            pitch: 2.06,
            lens: Lens {
                distortion,
                center: (0.5, 0.5),
            },
            phase: (0.17, 0.41),
        },
    );
    frame
}

fn plane(width: usize, height: usize, value: f32, defocus: f32) -> Frame {
    plane_with_lens(width, height, value, defocus, 0.0, 0.035)
}

fn axis_variance(frame: &Frame, axis: usize) -> f64 {
    let (width, height) = (frame.width, frame.height);
    let length = if axis == 0 { height } else { width };
    let mut moments = vec![0.0_f64; length];
    for y in 0..height {
        for x in 0..width {
            let position = if axis == 0 { y } else { x };
            moments[position] += frame.pixels()[y * width + x][0] as f64;
        }
    }
    let center = (if axis == 0 { height } else { width }) as f64 / 2.0 - 0.5;
    let mut variance = 0.0;
    for (position, mass) in moments.iter().enumerate() {
        let distance = position as f64 - center;
        variance += mass * distance * distance;
    }
    variance
}

fn sinusoid(width: usize, height: usize, fx: f64, fy: f64, amplitude: f32) -> Frame {
    let mut frame = Frame::zeros(width, height);
    for y in 0..height {
        for x in 0..width {
            let phase = std::f64::consts::TAU * (fx * (x as f64 + 0.5) + fy * (y as f64 + 0.5));
            frame.pixels_mut()[y * width + x] = [0.5 + amplitude * phase.cos() as f32; 3];
        }
    }
    frame
}

/// Signed amplitude of the `0.4 * cos` test carrier left in `frame`.
fn projected_response(frame: &Frame, fx: f64, fy: f64, margin: usize) -> f64 {
    let (width, height) = (frame.width, frame.height);
    let mut signal = 0.0_f64;
    let mut carrier = 0.0_f64;
    for y in margin..height - margin {
        for x in margin..width - margin {
            let phase = std::f64::consts::TAU * (fx * (x as f64 + 0.5) + fy * (y as f64 + 0.5));
            let wave = phase.cos();
            signal += (frame.pixels()[y * width + x][0] as f64 - 0.5) * wave;
            carrier += wave * wave;
        }
    }
    signal / carrier / 0.4
}

fn shared_response(sigma: f32, fx: f64, fy: f64) -> f64 {
    match AxisKernel::new(sigma) {
        Some(kernel) => kernel.response(fx) * kernel.response(fy),
        None => 1.0,
    }
}

#[test]
fn blur_preserves_energy_and_requested_variance() {
    for sigma in [0.24_f32, 0.52, 1.5, 3.2] {
        let mut impulse = Frame::zeros(65, 65);
        impulse.pixels_mut()[32 * 65 + 32][0] = 1.0;
        let blurred = gaussian_blur(impulse, sigma);
        let energy: f64 = blurred.pixels().iter().map(|pixel| pixel[0] as f64).sum();
        assert!(
            (energy - 1.0).abs() < 2e-5,
            "sigma {sigma}: energy {energy}"
        );
        let expected = (sigma * sigma) as f64;
        let tolerance = (expected * 2e-4).max(2e-5);
        for axis in 0..2 {
            let variance = axis_variance(&blurred, axis);
            assert!(
                (variance - expected).abs() < tolerance,
                "sigma {sigma} axis {axis}: variance {variance} expected {expected}"
            );
        }
    }
}

#[test]
fn blur_response_matches_the_shared_carrier_kernel() {
    let (width, height) = (129, 129);
    let directions = [
        (0.08, 0.0),
        (0.0, 0.08),
        (0.06, 0.053),
        (-0.08, 0.04),
        (0.2, 0.0),
    ];
    for sigma in [0.02_f32, 0.0201, 0.24, 0.7, 0.7001, 1.5, 3.2] {
        for (fx, fy) in directions {
            let blurred = gaussian_blur(sinusoid(width, height, fx, fy, 0.4), sigma);
            let measured = projected_response(&blurred, fx, fy, 20);
            let expected = shared_response(sigma, fx, fy);
            assert!(
                (measured - expected).abs() < 2e-4,
                "sigma {sigma} direction ({fx}, {fy}): measured {measured} expected {expected}"
            );
        }
    }
}

#[test]
fn display_carriers_match_spatial_defocus() {
    let (width, height) = (129, 97);
    let center = (0.43, 0.29);
    for (base, tilt, pitch) in [
        (0.01_f32, 0.0, 2.06_f32),
        (0.24, 0.28, 2.06),
        (0.68, 0.1, 2.06),
        (0.72, 0.3, 2.06),
        (1.5, 0.0, 2.06),
        (3.2, 0.0, 5.0),
        (0.24, 5e-5, 2.06),
    ] {
        for (a, b) in [(1.0_f64, 0.0_f64), (0.6, 0.8)] {
            let mut stripes = Frame::zeros(width, height);
            let mut gaps = Frame::zeros(width, height);
            let phase = (0.13_f32, 0.27_f32);
            for y in 0..height {
                for x in 0..width {
                    let (x, y) = (x as f64 + 0.5, y as f64 + 0.5);
                    let pu =
                        std::f64::consts::TAU * ((a * x + b * y) / pitch as f64 + phase.0 as f64);
                    let pv =
                        std::f64::consts::TAU * ((-b * x + a * y) / pitch as f64 + phase.1 as f64);
                    let (sin, cos) = pu.sin_cos();
                    let index = (y as usize) * width + x as usize;
                    stripes.pixels_mut()[index] = [
                        (0.5 * (1.0 + 0.3 * cos)) as f32,
                        (0.5 * (1.0 - 0.3 * (0.5 * cos + 0.866_025_4 * sin))) as f32,
                        (0.5 * (1.0 - 0.3 * (0.5 * cos - 0.866_025_4 * sin))) as f32,
                    ];
                    gaps.pixels_mut()[index] = [(0.5 * (1.0 + 0.3 * 0.65 * pv.cos())) as f32; 3];
                }
            }
            let stripes = defocus(stripes, base, tilt, center);
            let gaps = defocus(gaps, base, tilt, center);
            let mut sampled = Frame::zeros(width, height);
            sampled.pixels_mut().fill([0.5; 3]);
            display_sampling(
                &mut sampled,
                &[[a, b, 0.0], [-b, a, 0.0], [0.0, 0.0, 1.0]],
                DisplayStyle {
                    moire: 0.3,
                    pitch,
                    aperture: 0.0,
                    row_modulation: 0.65,
                    defocus: base,
                    tilt_defocus: tilt,
                },
                Sampling {
                    pitch,
                    lens: Lens {
                        distortion: 0.0,
                        center,
                    },
                    phase,
                },
            );
            let mut error = 0.0_f32;
            for y in 20..height - 20 {
                for x in 20..width - 20 {
                    let index = y * width + x;
                    let pixel = sampled.pixels()[index];
                    let mean = pixel.iter().sum::<f32>() / 3.0;
                    error = error.max((mean - gaps.pixels()[index][0]).abs());
                    for (channel, value) in pixel.iter().enumerate() {
                        error = error
                            .max((0.5 * value / mean - stripes.pixels()[index][channel]).abs());
                    }
                }
            }
            assert!(
                error < 5e-5,
                "base {base} tilt {tilt} direction ({a}, {b}): error {error}"
            );
        }
    }
}

#[test]
fn display_sampling_cannot_light_black() {
    let sampled = plane(31, 47, 0.0, 0.24);
    assert!(sampled.pixels().iter().all(|pixel| *pixel == [0.0; 3]));
}

#[test]
fn defocus_attenuates_display_interference() {
    let sharp = plane(61, 47, 0.42, 0.0);
    let soft = plane(61, 47, 0.42, 1.0);
    let spread = |frame: &Frame| {
        let mut sum = 0.0_f64;
        let mut squares = 0.0_f64;
        for pixel in frame.pixels() {
            for value in pixel {
                let delta = *value as f64 - 0.42;
                sum += delta;
                squares += delta * delta;
            }
        }
        let count = (frame.pixels().len() * 3) as f64;
        (squares / count - (sum / count).powi(2)).sqrt()
    };
    let (sharp_spread, soft_spread) = (spread(&sharp), spread(&soft));
    assert!(sharp_spread > 0.0);
    assert!(
        soft_spread < 0.1 * sharp_spread,
        "sharp {sharp_spread} soft {soft_spread}"
    );
}

#[test]
fn display_sampling_survives_tiny_and_odd_frames() {
    for distortion in [0.0, 0.035] {
        for (width, height) in [(9, 7), (7, 1), (1, 7), (1, 1)] {
            let sampled = plane_with_lens(width, height, 0.42, 0.24, 0.0, distortion);
            assert_eq!((sampled.width, sampled.height), (width, height));
            for pixel in sampled.pixels() {
                for value in pixel {
                    assert!(value.is_finite());
                    assert!(*value > 0.0 && *value < 0.84);
                }
            }
        }
    }
}

#[test]
fn zero_distortion_path_matches_the_radial_limit() {
    let fast = plane_with_lens(61, 47, 0.42, 0.24, 0.28, 0.0);
    let limit = plane_with_lens(61, 47, 0.42, 0.24, 0.28, 1e-9);
    for (fast_pixel, limit_pixel) in fast.pixels().iter().zip(limit.pixels()) {
        for channel in 0..3 {
            let delta = (fast_pixel[channel] - limit_pixel[channel]).abs();
            assert!(delta < 1e-5, "zero-distortion path drifted by {delta}");
        }
    }
}

#[test]
fn bayer_keeps_colour_when_a_cfa_cell_is_incomplete() {
    for (width, height) in [(7, 1), (1, 7), (1, 1)] {
        let mut frame = Frame::zeros(width, height);
        let count = (frame.pixels().len() * 3) as f32;
        for (index, pixel) in frame.pixels_mut().iter_mut().enumerate() {
            let base = index as f32 / count;
            *pixel = [base, 0.4 + base, 0.8 + base];
        }
        let before = frame.pixels().to_vec();
        bayer_artifacts(&mut frame, 1.0);
        assert_eq!(frame.pixels(), before.as_slice());
    }
}

#[test]
fn bayer_reconstructs_a_neutral_constant() {
    let mut frame = Frame::zeros(5, 4);
    frame.pixels_mut().fill([0.42; 3]);
    bayer_artifacts(&mut frame, 1.0);
    for pixel in frame.pixels() {
        for value in pixel {
            assert!((value - 0.42).abs() < 1e-5);
        }
    }
}

fn difference_spread(frame: &Frame) -> f64 {
    let mut sum = 0.0_f64;
    let mut squares = 0.0_f64;
    let mut count = 0_u64;
    for y in 2..frame.height.saturating_sub(2) {
        for x in 2..frame.width.saturating_sub(2) {
            let pixel = frame.pixels()[y * frame.width + x];
            let delta = (pixel[0] - pixel[2]) as f64;
            sum += delta;
            squares += delta * delta;
            count += 1;
        }
    }
    if count == 0 {
        return 0.0;
    }
    let count = count as f64;
    (squares / count - (sum / count).powi(2)).max(0.0).sqrt()
}

#[test]
fn cfa_reconstruction_produces_coloured_beats_that_survive_reduction() {
    let raw = plane(256, 192, 0.42, 0.24);
    let mut photo = raw.clone();
    bayer_artifacts(&mut photo, 1.0);
    let small = resize(&photo, 64, 48);
    let unsampled = resize(&raw, 64, 48);
    let colour = difference_spread(&small);
    let raw_colour = difference_spread(&unsampled);
    assert!(colour > 0.01, "colour {colour}");
    assert!(
        colour > 3.0 * raw_colour,
        "colour {colour} raw {raw_colour}"
    );
}

#[test]
fn sky_reflection_layout_follows_the_seeded_stream() {
    let (width, height) = (96, 72);
    let intensity = 0.035_f32;
    let model_peak =
        intensity * REFLECTION.base * REFLECTION.tint.iter().copied().fold(0.0, f32::max);
    // Lanczos upsampling may ring a few percent above the source peak.
    let bound = model_peak * 1.3;
    let mut left_bright = false;
    let mut right_bright = false;
    for seed in 0..64_u64 {
        let mut rng = Rng::stream(seed, REFLECTION_STREAM);
        let layer = sky_reflection(width, height, intensity, REFLECTION, &mut rng);
        let (mut left, mut right, mut top, mut bottom) = (0.0_f64, 0.0, 0.0, 0.0);
        let mut peak = 0.0_f32;
        for y in 0..height {
            for x in 0..width {
                let pixel = layer.pixels()[y * width + x];
                for value in pixel {
                    assert!(value.is_finite() && value >= 0.0, "seed {seed}: {value}");
                    peak = peak.max(value);
                }
                let lum = (pixel[0] + pixel[1] + pixel[2]) as f64;
                if x < width / 2 {
                    left += lum;
                } else {
                    right += lum;
                }
                if y < height / 2 {
                    top += lum;
                } else {
                    bottom += lum;
                }
            }
        }
        assert!(peak <= bound, "seed {seed}: peak {peak} above {bound}");
        assert!(
            peak > 0.5 * model_peak,
            "seed {seed}: peak {peak} below half the intensity model"
        );
        assert!(top > bottom, "seed {seed}: top {top} bottom {bottom}");
        if left > right {
            left_bright = true;
        } else if right > left {
            right_bright = true;
        }
    }
    assert!(
        left_bright && right_bright,
        "seeded streams never flipped the bright side"
    );
}

#[test]
fn sky_reflection_is_reproducible_per_stream() {
    let mut first = Rng::stream(20261111, REFLECTION_STREAM);
    let mut second = Rng::stream(20261111, REFLECTION_STREAM);
    let a = sky_reflection(64, 48, 0.035, REFLECTION, &mut first);
    let b = sky_reflection(64, 48, 0.035, REFLECTION, &mut second);
    assert_eq!(a.pixels(), b.pixels());
}
