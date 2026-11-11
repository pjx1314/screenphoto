//! One source/plan abstraction shared by `shot` and `capture`: where the pixels
//! come from, where the JPEG goes, and which refusals apply before rendering.

use std::io;
use std::path::{Path, PathBuf};

use screenphoto::limits::MAX_INPUT_BYTES;

use super::io::{output_aliases_source, read_capped};

/// Captures the live screen (or one monitor) as RGB. Production passes
/// `crate::capture::grab`; tests pass a fake so the screen path stays
/// exercisable without a desktop.
pub(crate) type Grabber = fn(Option<u32>) -> screenphoto::Result<image::RgbImage>;

/// Where the screenshot comes from.
pub(crate) enum Source {
    /// A PNG or JPEG on disk.
    File(PathBuf),
    /// The live screen; `Some(n)` picks monitor `n`.
    Screen(Option<u32>),
}

/// Why a [`Plan`] could not produce pixels. Wording and exit codes belong to
/// the reporting layer.
#[derive(Debug)]
pub(crate) enum PlanError {
    /// The source file does not exist.
    NotFound,
    /// The output path would replace the source file.
    Alias,
    /// The encoded input exceeds `MAX_INPUT_BYTES`.
    TooLarge,
    /// The screen could not be captured.
    CaptureFailed(screenphoto::Error),
    /// The source file could not be read.
    ReadFailed(io::Error),
}

/// Pixels loaded from a [`Source`], dispatched to the matching core entry point.
pub(crate) enum LoadedInput {
    Encoded(Vec<u8>),
    Rgb {
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    },
}

/// A resolved command: source, target path, and the label used in messages.
pub(crate) struct Plan {
    source: Source,
    output: PathBuf,
    label: String,
    grab: Grabber,
}

impl Plan {
    /// Resolve `--out` against the source-specific default. An empty path counts
    /// as not given.
    pub(crate) fn new(source: Source, requested: Option<&Path>, grab: Grabber) -> Self {
        let output = requested
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| source.default_output());
        let label = source.label();
        Self {
            source,
            output,
            label,
            grab,
        }
    }

    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn output(&self) -> &Path {
        &self.output
    }

    /// Validate the source-specific constraints, then load pixels.
    pub(crate) fn load(&self) -> Result<LoadedInput, PlanError> {
        match &self.source {
            Source::File(path) => {
                if !path.is_file() {
                    return Err(PlanError::NotFound);
                }
                if output_aliases_source(path, &self.output) {
                    return Err(PlanError::Alias);
                }
                let data = read_capped(path, MAX_INPUT_BYTES).map_err(PlanError::ReadFailed)?;
                if data.len() > MAX_INPUT_BYTES {
                    return Err(PlanError::TooLarge);
                }
                Ok(LoadedInput::Encoded(data))
            }
            Source::Screen(monitor) => {
                // The grabber owns the desktop pixel cap; re-checking it here
                // would duplicate `capture::checked_geometry` and is unreachable.
                let image = (self.grab)(*monitor).map_err(PlanError::CaptureFailed)?;
                let (width, height) = image.dimensions();
                Ok(LoadedInput::Rgb {
                    width,
                    height,
                    pixels: image.into_raw(),
                })
            }
        }
    }
}

impl Source {
    fn label(&self) -> String {
        match self {
            Self::File(path) => path.to_string_lossy().into_owned(),
            Self::Screen(_) => "screen capture".to_owned(),
        }
    }

    fn default_output(&self) -> PathBuf {
        match self {
            Self::File(path) => default_shot_output(path),
            Self::Screen(_) => PathBuf::from("capture-photo.jpg"),
        }
    }
}

impl LoadedInput {
    pub(crate) fn render(self, seed: Option<u64>) -> screenphoto::Result<(Vec<u8>, u64)> {
        match self {
            Self::Encoded(data) => screenphoto::render_bytes(&data, seed),
            Self::Rgb {
                width,
                height,
                pixels,
            } => screenphoto::render_rgb(width, height, pixels, seed),
        }
    }
}

fn default_shot_output(source: &Path) -> PathBuf {
    let mut name = source.file_stem().unwrap_or_default().to_os_string();
    name.push("-photo.jpg");
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::{LoadedInput, Plan, PlanError, Source};
    use std::path::{Path, PathBuf};

    fn no_grab(_: Option<u32>) -> screenphoto::Result<image::RgbImage> {
        unreachable!("file sources never grab the screen")
    }

    /// Encodes the requested monitor into the width so the test can prove the
    /// argument reaches the grabber.
    fn fake_grab(monitor: Option<u32>) -> screenphoto::Result<image::RgbImage> {
        let width = 4 + monitor.unwrap_or(0);
        Ok(image::RgbImage::from_pixel(width, 3, image::Rgb([1, 2, 3])))
    }

    fn oversized_grab(_: Option<u32>) -> screenphoto::Result<image::RgbImage> {
        let width = (screenphoto::MAX_INPUT_PIXELS + 1) as u32;
        let pixels = vec![0u8; width as usize * 3];
        Ok(image::RgbImage::from_raw(width, 1, pixels).expect("oversized RGB buffer"))
    }

    #[test]
    fn defaults_are_resolved_per_source_and_empty_path_means_default() {
        let plan = Plan::new(Source::File(PathBuf::from("dir/截图.png")), None, no_grab);
        assert_eq!(plan.output(), Path::new("截图-photo.jpg"));
        assert_eq!(plan.label(), "dir/截图.png");

        let plan = Plan::new(Source::Screen(Some(1)), Some(Path::new("")), no_grab);
        assert_eq!(plan.output(), Path::new("capture-photo.jpg"));
        assert_eq!(plan.label(), "screen capture");
    }

    #[test]
    fn load_returns_encoded_bytes_for_a_readable_source() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join(file!());
        let plan = Plan::new(Source::File(source), None, no_grab);
        assert!(matches!(
            plan.load(),
            Ok(LoadedInput::Encoded(data)) if !data.is_empty()
        ));
    }

    #[test]
    fn missing_source_is_a_typed_not_found() {
        let missing = std::env::temp_dir().join(format!(
            "screenphoto-missing-{}-{}.png",
            std::process::id(),
            line!()
        ));
        let plan = Plan::new(Source::File(missing), None, no_grab);
        assert!(matches!(plan.load(), Err(PlanError::NotFound)));
    }

    #[test]
    fn screen_source_passes_the_monitor_to_the_injected_grabber() {
        let plan = Plan::new(Source::Screen(Some(2)), None, fake_grab);
        match plan.load().expect("fake capture") {
            LoadedInput::Rgb {
                width,
                height,
                pixels,
            } => {
                assert_eq!((width, height), (6, 3));
                assert_eq!(pixels.len(), 6 * 3 * 3);
                assert_eq!(&pixels[..3], &[1u8, 2, 3]);
            }
            LoadedInput::Encoded(_) => panic!("screen source must load raw RGB"),
        }
    }

    #[test]
    fn screen_source_leaves_the_pixel_cap_to_the_grabber() {
        let plan = Plan::new(Source::Screen(None), None, oversized_grab);
        let loaded = plan.load().expect("the plan no longer re-checks the cap");
        match loaded {
            LoadedInput::Rgb { width, height, .. } => {
                assert_eq!(height, 1);
                assert_eq!(width, (screenphoto::MAX_INPUT_PIXELS + 1) as u32);
            }
            LoadedInput::Encoded(_) => panic!("screen source must load raw RGB"),
        }
    }
}
