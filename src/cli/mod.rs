//! Command line surface: `shot` and `capture`.

mod args;
mod io;
mod plan;
mod report;

use std::ffi::OsString;
use std::io::Write;

use clap::Parser;

use args::{Cli, Command, RenderArgs};
use plan::{Grabber, Plan, Source};
use report::{Failure, Reporter};

/// Parse arguments and run one command, returning its exit code.
pub fn run() -> i32 {
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    run_with(std::env::args_os(), &mut out, &mut err)
}

/// Same as [`run`], but writes to the given streams so tests can capture the
/// output in process. Usage errors still go through clap to the real streams.
fn run_with<I, T>(argv: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(error) => {
            // Usage errors go to stderr and exit 2; help/version prints and exits 0.
            let _ = error.print();
            return error.exit_code();
        }
    };
    match cli.command {
        Command::Shot(args) => execute(Source::File(args.source), args.render, out, err),
        Command::Capture(args) => execute(Source::Screen(args.monitor), args.render, out, err),
    }
}

/// Shared execution: resolve the plan, load pixels, render, and emit.
fn execute(source: Source, render: RenderArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    execute_with_grabber(source, render, out, err, crate::capture::grab)
}

/// [`execute`] with the screen grabber injected, so tests can drive the
/// `capture` path without a desktop.
fn execute_with_grabber(
    source: Source,
    render: RenderArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
    grab: Grabber,
) -> i32 {
    let mut reporter = Reporter::new(render.json, out, err);
    let plan = Plan::new(source, render.out.as_deref(), grab);
    let loaded = plan
        .load()
        .map_err(|error| Failure::from_plan(error, plan.label(), plan.output()));
    let rendered = loaded.and_then(|input| {
        input
            .render(render.seed)
            .map_err(|error| Failure::render(&error, plan.label()))
    });
    match rendered {
        Ok((jpeg, seed)) => emit(&plan, jpeg, seed, &mut reporter),
        Err(failure) => reporter.failure(failure),
    }
}

/// The one emit path for both commands: probe the JPEG header, write the file
/// atomically, then report.
fn emit(plan: &Plan, jpeg: Vec<u8>, seed: u64, report: &mut Reporter<'_>) -> i32 {
    let Some((width, height)) = io::jpeg_dimensions(&jpeg) else {
        return report.failure(Failure::missing_dimensions(plan.label()));
    };
    if let Err(error) = io::write_output(plan.output(), &jpeg) {
        return report.failure(Failure::write_failed(plan.output(), &error, plan.label()));
    }
    match report.success(plan.label(), plan.output(), width, height, jpeg.len(), seed) {
        Ok(()) => 0,
        Err(error) => report.report_failed(&error),
    }
}

#[cfg(test)]
mod tests {
    use super::{RenderArgs, Source, execute_with_grabber, run_with};
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn small_screen(_: Option<u32>) -> screenphoto::Result<image::RgbImage> {
        Ok(image::RgbImage::from_pixel(
            24,
            18,
            image::Rgb([30, 60, 90]),
        ))
    }

    fn oversized_screen(_: Option<u32>) -> screenphoto::Result<image::RgbImage> {
        let width = (screenphoto::MAX_INPUT_PIXELS + 1) as u32;
        let pixels = vec![0u8; width as usize * 3];
        Ok(image::RgbImage::from_raw(width, 1, pixels).expect("oversized RGB buffer"))
    }

    fn temp_dir() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "screenphoto-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([10, 20, 30]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("encode PNG");
        bytes.into_inner()
    }

    #[test]
    fn shot_runs_in_process_and_reports_json_on_the_given_writer() {
        let dir = temp_dir();
        let source = dir.join("input.png");
        std::fs::write(&source, png(32, 24)).expect("source");
        let output = dir.join("photo.jpg");
        let mut out = Vec::new();
        let mut err = Vec::new();

        let code = run_with(
            [
                "screenphoto",
                "shot",
                source.to_str().expect("UTF-8 path"),
                "--json",
                "--seed",
                "7",
                "-o",
                output.to_str().expect("UTF-8 path"),
            ],
            &mut out,
            &mut err,
        );

        assert_eq!(code, 0);
        assert!(err.is_empty());
        let payload: serde_json::Value = serde_json::from_slice(&out).expect("JSON line");
        assert_eq!(payload["ok"].as_bool(), Some(true));
        assert_eq!(payload["seed"].as_u64(), Some(7));
        assert_eq!(
            payload["bytes"].as_u64(),
            Some(std::fs::read(&output).expect("written JPEG").len() as u64)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_input_reports_the_plan_error_as_json() {
        let dir = temp_dir();
        let source = dir.join("missing.png");
        let mut out = Vec::new();
        let mut err = Vec::new();

        let code = run_with(
            [
                "screenphoto",
                "shot",
                source.to_str().expect("UTF-8 path"),
                "--json",
            ],
            &mut out,
            &mut err,
        );

        assert_eq!(code, 2);
        assert!(err.is_empty());
        let payload: serde_json::Value = serde_json::from_slice(&out).expect("JSON line");
        assert_eq!(payload["ok"].as_bool(), Some(false));
        assert_eq!(payload["input"].as_str(), source.to_str());
        let expected = format!("找不到文件：{}", source.display());
        assert_eq!(payload["error"].as_str(), Some(expected.as_str()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn capture_renders_the_injected_grabber_output_in_process() {
        let dir = temp_dir();
        let output = dir.join("capture.jpg");
        let mut out = Vec::new();
        let mut err = Vec::new();

        let code = execute_with_grabber(
            Source::Screen(Some(1)),
            RenderArgs {
                seed: Some(9),
                out: Some(output.clone()),
                json: true,
            },
            &mut out,
            &mut err,
            small_screen,
        );

        assert_eq!(code, 0);
        assert!(err.is_empty());
        let payload: serde_json::Value = serde_json::from_slice(&out).expect("JSON line");
        assert_eq!(payload["ok"].as_bool(), Some(true));
        assert_eq!(payload["input"].as_str(), Some("screen capture"));
        assert_eq!(payload["seed"].as_u64(), Some(9));
        assert_eq!(payload["width"].as_u64(), Some(24));
        assert_eq!(payload["height"].as_u64(), Some(18));
        assert_eq!(
            payload["bytes"].as_u64(),
            Some(std::fs::read(&output).expect("written JPEG").len() as u64)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn oversized_grabber_output_reaches_the_renderer_without_a_plan_refusal() {
        let dir = temp_dir();
        let output = dir.join("capture.jpg");
        let mut out = Vec::new();
        let mut err = Vec::new();

        let code = execute_with_grabber(
            Source::Screen(None),
            RenderArgs {
                seed: Some(1),
                out: Some(output.clone()),
                json: true,
            },
            &mut out,
            &mut err,
            oversized_screen,
        );

        // The plan no longer duplicates the grabber's pixel cap, so a
        // contract-violating capture is rejected by the renderer as bad input.
        assert_eq!(code, 2);
        assert!(err.is_empty());
        let payload: serde_json::Value = serde_json::from_slice(&out).expect("JSON line");
        assert_eq!(payload["ok"].as_bool(), Some(false));
        let expected = format!(
            "image exceeds {} input pixels or has a zero dimension",
            screenphoto::MAX_INPUT_PIXELS
        );
        assert_eq!(payload["error"].as_str(), Some(expected.as_str()));
        assert!(!output.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
