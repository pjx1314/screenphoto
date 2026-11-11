//! Reporting seam: one JSON line or human text, written to captured streams.

use std::io::Write;
use std::path::Path;

use screenphoto::limits::MAX_INPUT_BYTES;

use super::plan::PlanError;

/// A command failure including the exit code and input label it reports.
pub(crate) struct Failure {
    message: String,
    input: String,
    code: i32,
}

impl Failure {
    /// Invalid input or arguments: exit 2.
    pub(crate) fn input(message: String, input: &str) -> Self {
        Self {
            message,
            input: input.to_owned(),
            code: 2,
        }
    }

    /// Processing, output, or environment failure: exit 1.
    pub(crate) fn processing(message: String, input: &str) -> Self {
        Self {
            message,
            input: input.to_owned(),
            code: 1,
        }
    }

    /// Map a renderer error: bad input exits 2, processing failures exit 1.
    pub(crate) fn render(error: &screenphoto::Error, input: &str) -> Self {
        let code = if error.is_input() { 2 } else { 1 };
        Self {
            message: error.to_string(),
            input: input.to_owned(),
            code,
        }
    }

    /// The rendered JPEG had no readable dimensions.
    pub(crate) fn missing_dimensions(input: &str) -> Self {
        Self::processing("输出 JPEG 缺少尺寸".to_owned(), input)
    }

    /// The output file could not be written.
    pub(crate) fn write_failed(output: &Path, error: &std::io::Error, input: &str) -> Self {
        Self::processing(format!("写不了 {}：{error}", output.display()), input)
    }

    /// Map a plan refusal: bad input exits 2, capture failures exit 1.
    pub(crate) fn from_plan(error: PlanError, input: &str, output: &Path) -> Self {
        match error {
            PlanError::NotFound => Self::input(format!("找不到文件：{input}"), input),
            PlanError::Alias => Self::input(
                format!("输出路径不能是源文件本身：{}", output.display()),
                input,
            ),
            PlanError::TooLarge => Self::input(
                format!(
                    "输入超过 {} MiB 上限：{input}",
                    MAX_INPUT_BYTES / (1024 * 1024)
                ),
                input,
            ),
            PlanError::CaptureFailed(error) => {
                Self::processing(format!("抓屏失败：{error}"), input)
            }
            PlanError::ReadFailed(error) => Self::input(format!("读不了 {input}：{error}"), input),
        }
    }
}

/// Writes command output to the given streams, so tests can capture it instead
/// of spawning a process.
pub(crate) struct Reporter<'a> {
    json: bool,
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
}

impl<'a> Reporter<'a> {
    pub(crate) fn new(json: bool, out: &'a mut dyn Write, err: &'a mut dyn Write) -> Self {
        Self { json, out, err }
    }

    /// Report a written JPEG: one JSON line, or two human lines, on stdout.
    ///
    /// A failed write is returned rather than swallowed; the caller maps it to
    /// a processing failure. The JSON contract and exit code are unchanged
    /// when the write succeeds.
    pub(crate) fn success(
        &mut self,
        input: &str,
        output: &Path,
        width: u32,
        height: u32,
        bytes: usize,
        seed: u64,
    ) -> std::io::Result<()> {
        if self.json {
            let payload = serde_json::json!({
                "ok": true,
                "input": input,
                "output": output.to_string_lossy(),
                "width": width,
                "height": height,
                "bytes": bytes,
                "seed": seed,
            });
            writeln!(self.out, "{payload}")?;
        } else {
            writeln!(
                self.out,
                "  输出 {} ({width}x{height}, {} KB)",
                output.display(),
                bytes / 1024
            )?;
            writeln!(self.out, "  seed {seed}")?;
        }
        Ok(())
    }

    /// Report a failure: JSON on stdout, human errors on stderr. Returns the
    /// exit code carried by the failure.
    ///
    /// If the report itself cannot be written, fall back to stderr and report
    /// a processing failure (exit 1) instead of hiding the outcome.
    pub(crate) fn failure(&mut self, failure: Failure) -> i32 {
        let written = if self.json {
            let payload = serde_json::json!({
                "ok": false,
                "error": failure.message,
                "input": failure.input,
            });
            writeln!(self.out, "{payload}")
        } else {
            writeln!(self.err, "{}", failure.message)
        };
        match written {
            Ok(()) => failure.code,
            Err(error) => {
                let _ = writeln!(self.err, "{}", failure.message);
                self.report_failed(&error)
            }
        }
    }

    /// Best-effort stderr fallback when a success report could not be written.
    /// Returns the processing-failure exit code.
    pub(crate) fn report_failed(&mut self, error: &std::io::Error) -> i32 {
        let _ = writeln!(self.err, "报告输出失败：{error}");
        1
    }
}

#[cfg(test)]
mod tests {
    use super::{Failure, Reporter};
    use std::path::Path;

    /// Every write fails with `BrokenPipe`, like a closed stdout pipe.
    struct BrokenPipe;

    impl std::io::Write for BrokenPipe {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
    }

    #[test]
    fn report_write_failures_exit_one_and_fall_back_to_stderr() {
        let mut out = BrokenPipe;
        let mut err = Vec::new();
        let mut reporter = Reporter::new(true, &mut out, &mut err);
        let error = reporter
            .success("in.png", Path::new("out.jpg"), 10, 20, 4096, 7)
            .expect_err("a broken stdout must surface");
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
        assert_eq!(reporter.report_failed(&error), 1);

        let mut out = BrokenPipe;
        let mut err = Vec::new();
        let mut reporter = Reporter::new(true, &mut out, &mut err);
        let code = reporter.failure(Failure::input("找不到文件：in.png".to_owned(), "in.png"));
        assert_eq!(code, 1);
    }
}
