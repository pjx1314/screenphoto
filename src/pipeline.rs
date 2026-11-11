//! The ordered capture pipeline: RNG stream allocation and every optical
//! stage between a decoded screenshot and an RGB photo.

use image::RgbImage;

use crate::calibration::Calibration;
use crate::color;
use crate::frame::Frame;
use crate::geometry::{self, Lens, Projection};
use crate::optics;
use crate::optics::Sampling;
use crate::rng::Rng;

/// Fixed RNG streams and draw order; both are part of the seed contract.
///
/// Stream 0 draws pose yaw, pitch and roll. Stream 1 draws display pitch, lens
/// centre (x, y), lens distortion, then sampling phase (x, y). Stream 2 is
/// consumed by sensor noise, stream 3 by white balance and stream 4 by reflection.
/// Never reorder draws within a stream.
struct Streams {
    pose: Rng,
    sampling: Rng,
    sensor: Rng,
    white_balance: Rng,
    reflection: Rng,
}

impl Streams {
    fn new(seed: u64) -> Self {
        Self {
            pose: Rng::stream(seed, 0),
            sampling: Rng::stream(seed, 1),
            sensor: Rng::stream(seed, 2),
            white_balance: Rng::stream(seed, 3),
            reflection: Rng::stream(seed, 4),
        }
    }

    /// Draw every per-capture value in the documented order.
    fn draw(&mut self, style: &Calibration, pose_scale: f32) -> ((f64, f64, f64), Sampling) {
        let jitter = style.pose_jitter_deg;
        let angles = (
            f64::from((style.yaw_deg + self.pose.uniform(-jitter, jitter)) * pose_scale),
            f64::from((style.pitch_deg + self.pose.uniform(-jitter, jitter)) * pose_scale),
            f64::from((style.roll_deg + self.pose.uniform(-jitter, jitter)) * pose_scale),
        );
        let pitch = style.display.pitch * self.sampling.uniform(0.92, 1.08);
        let center = (
            self.sampling.uniform(0.35, 0.55),
            self.sampling.uniform(0.25, 0.45),
        );
        let lens = Lens {
            distortion: style.lens_distortion * self.sampling.uniform(0.85, 1.15),
            center,
        };
        let phase = (
            self.sampling.uniform(0.0, 1.0),
            self.sampling.uniform(0.0, 1.0),
        );
        (angles, Sampling { pitch, lens, phase })
    }
}

/// Run the capture stages in order on an already-decoded screen image.
/// `(w, h)` is the capped output size chosen by the caller.
pub(crate) fn capture(
    screen: RgbImage,
    w: usize,
    h: usize,
    seed: u64,
    style: &Calibration,
) -> RgbImage {
    let (source_w, source_h) = screen.dimensions();
    let (sw, sh) = (source_w as usize, source_h as usize);
    let mut streams = Streams::new(seed);
    // Long pages need less tilt to stay within the crop budget.
    let pose_scale = sw.min(sh) as f32 / sw.max(sh) as f32;
    let (angles, mut settings) = streams.draw(style, pose_scale);
    let projection = Projection::camera(
        (sw, sh),
        (w, h),
        angles,
        style.camera_distance,
        settings.lens,
        style.crop_limit,
    );
    settings.lens = projection.lens();
    let center = settings.lens.center;
    let (exposure, lift) = if style.auto_exposure {
        color::auto_exposure(&screen)
    } else {
        (1.0, 0.0)
    };
    let mut frame =
        geometry::warp_screen(&screen, projection.screen_to_frame(), w, h, settings.lens);
    drop(screen);
    frame = optics::defocus(
        frame,
        style.display.defocus,
        style.display.tilt_defocus,
        center,
    );
    optics::vignette(&mut frame, style.vignette, center);
    optics::display_sampling(
        &mut frame,
        projection.frame_to_screen(),
        style.display,
        settings,
    );
    let divisor = style.auxiliary_divisor.max(1);
    let (qw, qh) = ((w / divisor).max(2), (h / divisor).max(2));
    if style.reflection_amount > 0.0 {
        // The sky is low-frequency and source-normalized; a capture-sized grid bounds even 1px axes.
        let reflection = optics::sky_reflection(
            qw,
            qh,
            style.reflection_amount,
            style.reflection,
            &mut streams.reflection,
        );
        let low_matrix = projection.auxiliary_matrix((qw, qh));
        let reflected = geometry::warp_edges(&reflection, &low_matrix, qw, qh, settings.lens);
        drop(reflection);
        let layer = optics::resize(&reflected, w, h);
        add_layer(&mut frame, &layer);
    }
    if style.bloom_strength > 0.0 {
        let base = optics::resize(&frame, qw, qh);
        let bloom = optics::bloom_layer(
            &base,
            style.bloom_threshold,
            (qw.min(qh) as f32 * style.bloom_radius).max(1.0),
            style.bloom_strength,
        );
        drop(base);
        let layer = optics::resize(&bloom, w, h);
        add_layer(&mut frame, &layer);
    }
    let multiplier = exposure * style.exposure_ev.exp2();
    for pixel in frame.pixels_mut() {
        for value in pixel {
            *value = *value * multiplier + lift;
        }
    }
    if style.noise_scale > 0.0 {
        optics::sensor_noise(
            &mut frame,
            &mut streams.sensor,
            style.iso * style.noise_scale,
            style.chroma_noise,
            style.sensor,
        );
    }
    optics::bayer_artifacts(&mut frame, style.cfa);
    color::white_balance_shift(&mut frame, &mut streams.white_balance, style.white_balance);
    color::tone_encode(&mut frame);
    frame = optics::sharpen(frame, style.sharpen);
    let image = optics::to_uint8(&frame);
    drop(frame);
    image
}

fn add_layer(frame: &mut Frame, layer: &Frame) {
    for (pixel, addition) in frame.pixels_mut().iter_mut().zip(layer.pixels()) {
        for channel in 0..3 {
            pixel[channel] += addition[channel];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_capture_keeps_lines_straight_without_flattening_the_perspective() {
        let style = Calibration::default();
        for (sw, sh, w, h) in [
            (240, 160, 240, 160),
            (160, 240, 160, 240),
            (60, 720, 60, 720),
            (1000, 333, 320, 107),
        ] {
            for seed in [0, 7, 42, 20261107, u64::MAX] {
                let (angles, settings) =
                    Streams::new(seed).draw(&style, sw.min(sh) as f32 / sw.max(sh) as f32);
                let projection = Projection::camera(
                    (sw, sh),
                    (w, h),
                    angles,
                    style.camera_distance,
                    settings.lens,
                    style.crop_limit,
                );
                let map = |x: f64, y: f64| {
                    let (lx, ly) =
                        geometry::lens_coords(x * w as f64, y * h as f64, w, h, projection.lens());
                    geometry::apply_h(projection.frame_to_screen(), lx, ly)
                };
                for (start, end) in [
                    ((0.1, 0.2), (0.9, 0.2)),
                    ((0.1, 0.8), (0.9, 0.8)),
                    ((0.2, 0.1), (0.2, 0.9)),
                    ((0.8, 0.1), (0.8, 0.9)),
                    ((0.1, 0.2), (0.9, 0.8)),
                    ((0.1, 0.8), (0.9, 0.2)),
                ] {
                    let a = map(start.0, start.1);
                    let b = map(end.0, end.1);
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    for step in 1..32 {
                        let t = f64::from(step) / 32.0;
                        let p = map(
                            start.0 + (end.0 - start.0) * t,
                            start.1 + (end.1 - start.1) * t,
                        );
                        let deviation = ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / dx.hypot(dy);
                        assert!(
                            deviation < 1e-7,
                            "line bent by {deviation} pixels in {sw}x{sh}, seed {seed}"
                        );
                    }
                }
                let perspective = projection.screen_to_frame()[2];
                assert!(
                    (perspective[0] * sw as f64).hypot(perspective[1] * sh as f64) > 1e-4,
                    "tilt was flattened in {sw}x{sh}, seed {seed}"
                );
            }
        }
    }
}
