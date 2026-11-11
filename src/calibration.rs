use crate::optics::{DisplayStyle, ReflectionStyle, SensorStyle, VignetteStyle};

/// Internal calibration; deliberately absent from the public contract.
///
/// Single owner of the visual style: pose, display carriers, glass, lens,
/// sensor and delivery coefficients. The optics stages receive their composed
/// style structs directly instead of field-by-field copies.
#[derive(Clone, Copy)]
pub(crate) struct Calibration {
    /// Camera yaw in degrees.
    pub yaw_deg: f32,
    /// Camera pitch in degrees.
    pub pitch_deg: f32,
    /// Camera roll in degrees.
    pub roll_deg: f32,
    /// Uniform pose jitter half-width in degrees.
    pub pose_jitter_deg: f32,
    /// Camera distance in source-screen widths; positive.
    pub camera_distance: f64,
    /// Maximum centred crop per axis, combining both sides.
    pub crop_limit: f64,
    pub display: DisplayStyle,
    /// Residual radial distortion after camera processing; zero keeps lines straight.
    pub lens_distortion: f32,
    /// Linear-light reflection multiplier; non-negative.
    pub reflection_amount: f32,
    pub reflection: ReflectionStyle,
    pub vignette: VignetteStyle,
    /// Linear-light bloom multiplier; non-negative.
    pub bloom_strength: f32,
    /// Linear-light luminance threshold for bloom.
    pub bloom_threshold: f32,
    /// Bloom blur radius as a fraction of the auxiliary short edge.
    pub bloom_radius: f32,
    /// Output-to-auxiliary edge divisor; at least one.
    pub auxiliary_divisor: usize,
    /// Unsharp-mask blend strength; non-negative.
    pub sharpen: f32,
    pub auto_exposure: bool,
    /// Exposure compensation in stops, applied as `2^EV`.
    pub exposure_ev: f32,
    /// Sensor sensitivity, with ISO 100 as the base gain.
    pub iso: f32,
    /// Sensitivity multiplier; zero disables sensor noise.
    pub noise_scale: f32,
    /// Relative chroma-noise strength; non-negative.
    pub chroma_noise: f32,
    pub sensor: SensorStyle,
    /// Bayer reconstruction blend strength in 0..=1.
    pub cfa: f32,
    /// Standard deviation of per-channel white-balance gain jitter.
    pub white_balance: f32,
    /// JPEG encoder quality in 1..=100.
    pub jpeg_quality: u8,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            yaw_deg: 4.0,
            pitch_deg: -3.0,
            roll_deg: 0.6,
            pose_jitter_deg: 1.5,
            camera_distance: 2.4,
            crop_limit: 0.10,
            display: DisplayStyle {
                moire: 0.30,
                pitch: 2.06,
                aperture: 0.8,
                row_modulation: 0.65,
                defocus: 0.24,
                tilt_defocus: 0.28,
            },
            lens_distortion: 0.0,
            reflection_amount: 0.035,
            reflection: ReflectionStyle {
                gradient: 1.15,
                base: 0.55,
                tint: [0.84, 0.93, 1.12],
                blur: 0.004,
            },
            vignette: VignetteStyle {
                amount: 0.24,
                power: 1.8,
                aspect: (1.7, 1.8),
                floor: 0.25,
            },
            bloom_strength: 0.0,
            bloom_threshold: 0.90,
            bloom_radius: 0.028,
            auxiliary_divisor: 4,
            sharpen: 0.10,
            auto_exposure: true,
            exposure_ev: -0.6,
            iso: 320.0,
            noise_scale: 1.0,
            chroma_noise: 0.14,
            sensor: SensorStyle {
                shot_scale: 0.008,
                gain_power: 0.55,
                floor: 0.015,
                chroma_scale: 0.005,
                chroma_gain_power: 0.4,
                chroma_blur: 1.1,
                band_scale: 0.0018,
                band_blur: 1.5,
            },
            cfa: 0.65,
            white_balance: 0.012,
            jpeg_quality: 92,
        }
    }
}
