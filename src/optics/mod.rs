//! Optics: the camera chain between a clean screenshot and a photo.
//!
//! Defocus/vignette -> display carriers -> reflection/bloom -> sensor noise ->
//! CFA reconstruction -> white balance -> tone/sharpening. Float stages only
//! quantise at `to_uint8`. Display modulation preserves black emission;
//! reflection and sensor noise may lift it.
//!
//! Colour-domain stages (`auto_exposure`, `white_balance_shift`,
//! `tone_encode`) live in `crate::color`; this facade is the spatial/sensor
//! chain.

mod blur;
mod display;
mod resample;
mod sensor;
mod sharpen;

pub(crate) use blur::{VignetteStyle, defocus, vignette};
pub(crate) use display::{
    DisplayStyle, ReflectionStyle, Sampling, bloom_layer, display_sampling, sky_reflection,
};
pub(crate) use resample::resize;
pub(crate) use sensor::{SensorStyle, bayer_artifacts, sensor_noise};
pub(crate) use sharpen::{sharpen, to_uint8};

fn lowres_factor(width: usize, height: usize) -> usize {
    if width.min(height) >= 600 { 2 } else { 1 }
}

#[cfg(test)]
mod tests;
