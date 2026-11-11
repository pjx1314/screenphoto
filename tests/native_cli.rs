//! Real-process regressions for `screenphoto shot`: aliasing, damaged input,
//! usage errors, and the dimensions/byte count reported on success.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

const EXE: &str = env!("CARGO_BIN_EXE_screenphoto");

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "屏 幕 照 片 {tag} {}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = RgbImage::from_fn(width, height, |x, y| {
        Rgb([(x % 256) as u8, (y % 256) as u8, 90])
    });
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .expect("encode PNG");
    bytes.into_inner()
}

fn shot(source: &Path, out: Option<&Path>, extra: &[&str]) -> Output {
    let mut command = Command::new(EXE);
    command.arg("shot").arg(source);
    if let Some(out) = out {
        command.arg("-o").arg(out);
    }
    command.args(extra).output().expect("spawn screenphoto")
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not one JSON line: {error}: {:?}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn assert_refused(output: &Output, label: &str) {
    assert_eq!(
        output.status.code(),
        Some(2),
        "{label}: stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(output);
    assert_eq!(payload["ok"].as_bool(), Some(false), "{label}: {payload}");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()),
        "{label}: {payload}"
    );
}

fn assert_usage_error(output: &Output, label: &str) {
    assert_eq!(
        output.status.code(),
        Some(2),
        "{label}: stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "{label}: usage errors must not print JSON: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !output.stderr.is_empty(),
        "{label}: usage errors belong on stderr"
    );
}

#[test]
fn output_aliases_are_refused_and_leave_source_bytes_intact() {
    let dir = TempDir::new("aliases");
    let source = dir.path().join("源 图.png");
    let original = png(48, 32);
    std::fs::write(&source, &original).expect("source");

    let normalized = dir.path().join(".").join("源 图.png");
    let hard = dir.path().join("硬 链.png");
    std::fs::hard_link(&source, &hard).expect("hard link");
    let aliases = [
        ("source", source.clone()),
        ("normalized", normalized),
        ("hardlink", hard),
        #[cfg(unix)]
        ("symlink", {
            let soft = dir.path().join("符 号 链.png");
            std::os::unix::fs::symlink(&source, &soft).expect("symlink");
            soft
        }),
    ];

    for (label, alias) in aliases {
        let output = shot(&source, Some(&alias), &["--json"]);
        assert_refused(&output, label);
        assert_eq!(
            std::fs::read(&source).expect("source still readable"),
            original,
            "{label}: source bytes changed"
        );
        assert_eq!(
            std::fs::read(&alias).expect("alias still readable"),
            original,
            "{label}: alias bytes changed"
        );
    }
}

#[test]
fn damaged_input_does_not_replace_an_existing_output() {
    let dir = TempDir::new("corrupt");
    let source = dir.path().join("坏 图.png");
    std::fs::write(&source, b"\x89PNG\r\n\x1a\nnot actually a PNG").expect("corrupt source");
    let out = dir.path().join("已有 输出.jpg");
    let sentinel = b"existing output must survive";
    std::fs::write(&out, sentinel).expect("existing output");

    let output = shot(&source, Some(&out), &["--json"]);
    assert_refused(&output, "corrupt input");
    assert_eq!(
        std::fs::read(&out).expect("output still readable"),
        sentinel,
        "damaged input overwrote the output"
    );
}

#[test]
fn typed_and_unknown_arguments_exit_2_on_stderr_without_stdout_json() {
    let dir = TempDir::new("usage");
    let source = dir.path().join("源 图.png");
    std::fs::write(&source, png(16, 16)).expect("source");
    let out = dir.path().join("输出.jpg");

    assert_usage_error(
        &shot(&source, Some(&out), &["--json", "--seed", "not-a-number"]),
        "typed value",
    );
    assert_usage_error(
        &shot(&source, Some(&out), &["--json", "--no-such-flag"]),
        "unknown flag",
    );
    assert!(!out.exists(), "usage errors must not write output");
}

#[test]
fn success_json_dimensions_and_bytes_match_the_written_jpeg() {
    let dir = TempDir::new("success");
    let source = dir.path().join("中文 截图.png");
    std::fs::write(&source, png(160, 120)).expect("source");
    let out = dir.path().join("照片 输出.jpg");

    let output = shot(&source, Some(&out), &["--json", "--seed", "7"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = json(&output);
    assert_eq!(payload["ok"].as_bool(), Some(true), "{payload}");

    let written = std::fs::read(&out).expect("written JPEG");
    assert_eq!(
        payload["bytes"].as_u64(),
        Some(written.len() as u64),
        "reported byte count differs from the file"
    );
    let decoded = image::load_from_memory(&written).expect("output decodes as an image");
    assert_eq!(
        payload["width"].as_u64(),
        Some(u64::from(decoded.width())),
        "reported width differs from the JPEG"
    );
    assert_eq!(
        payload["height"].as_u64(),
        Some(u64::from(decoded.height())),
        "reported height differs from the JPEG"
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_default_outputs_do_not_collide() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = TempDir::new("non-utf8");
    let render = |source: &[u8], output: &[u8], width| {
        let source = OsStr::from_bytes(source);
        std::fs::write(dir.path().join(source), png(width, 32)).expect("source");
        let result = Command::new(EXE)
            .current_dir(dir.path())
            .arg("shot")
            .arg(source)
            .args(["--seed", "7", "--json"])
            .output()
            .expect("spawn screenphoto");
        assert!(result.status.success(), "{result:?}");
        let image = image::open(dir.path().join(OsStr::from_bytes(output))).expect("distinct JPEG");
        assert_eq!((image.width(), image.height()), (width, 32));
    };

    render(b"foo\xff.png", b"foo\xff-photo.jpg", 32);
    let first_output = dir.path().join(OsStr::from_bytes(b"foo\xff-photo.jpg"));
    let first_jpeg = std::fs::read(&first_output).expect("first JPEG");
    render(b"foo\xfe.png", b"foo\xfe-photo.jpg", 48);
    assert_eq!(
        std::fs::read(first_output).expect("first JPEG survives second render"),
        first_jpeg
    );
}
