pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const TARGET: &str = env!("SCREENPHOTO_TARGET");

/// Fixed seed for every checked-in gallery and benchmark sample.
pub const SAMPLE_SEED: u64 = 20261107;

pub mod benchmark;
pub mod corpus;
pub mod gallery;
pub mod package;
pub mod util;
