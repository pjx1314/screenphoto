//! One native screenshot-to-photograph renderer.
//!
//! The core renderer (`render_bytes`/`render_rgb`) performs no filesystem or
//! network IO. The optional `dev-tools` feature adds maintenance utilities
//! that do.
mod calibration;
mod color;
mod frame;
mod geometry;
pub mod limits;
mod optics;
mod pipeline;
mod render;
mod rng;

#[cfg(feature = "dev-tools")]
#[doc(hidden)]
pub mod devtools;

pub use limits::{LONG_EDGE_CAP, LimitError, MAX_INPUT_BYTES, MAX_INPUT_PIXELS, checked_pixels};
pub use render::{render_bytes, render_rgb};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    InvalidInput(String),
    Processing(String),
}

impl Error {
    pub fn is_input(&self) -> bool {
        matches!(self, Self::InvalidInput(_))
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(message) | Self::Processing(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {}
