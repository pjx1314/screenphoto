//! One owner for the input byte/pixel caps and the output long-edge cap.

/// Encoded input cap.
pub const MAX_INPUT_BYTES: usize = 40 * 1024 * 1024;

/// Decoded input cap.
pub const MAX_INPUT_PIXELS: u64 = 24_000_000;

/// Optical and delivery cap; bounds memory and output size.
pub const LONG_EDGE_CAP: u32 = 1920;

/// Why an input geometry was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitError {
    Empty,
    TooLarge,
}

/// Validate an input geometry against the pixel cap. Returns the pixel count
/// so callers can size their buffers.
pub fn checked_pixels(width: u32, height: u32) -> std::result::Result<u64, LimitError> {
    if width == 0 || height == 0 {
        return Err(LimitError::Empty);
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_INPUT_PIXELS {
        return Err(LimitError::TooLarge);
    }
    Ok(pixels)
}
