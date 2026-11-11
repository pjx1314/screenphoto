//! File and IO policy: capped reads, JPEG probing, atomic replacement, aliasing.

use std::fs::{File, OpenOptions};
use std::io::{self, Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Read at most `cap` bytes plus one, so callers can detect an oversized file
/// without loading it whole.
pub(super) fn read_capped(path: &Path, cap: usize) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    File::open(path)?
        .take(cap as u64 + 1)
        .read_to_end(&mut data)?;
    Ok(data)
}

/// Width and height from a JPEG header, without decoding the pixels.
pub(super) fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg)
        .into_dimensions()
        .ok()
}

/// Write `bytes` to `path` through a synced temp file in the same directory, so
/// a failed or interrupted write never leaves a partial or stale output.
pub(super) fn write_output(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    write_output_with(path, bytes, TEMP_COUNTER.fetch_add(1, Ordering::Relaxed))
}

/// [`write_output`] with the temp-name counter injected, so tests can pin a
/// collision without touching the shared counter.
fn write_output_with(path: &Path, bytes: &[u8], counter: u64) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(temp_name(path, counter));
    // Only a temp file this call created may be cleaned up: a failed
    // `create_new` means the path was not ours.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// A short temp basename that never equals the destination: dot-prefixed
/// targets get a plain name and everything else gets a dot-prefixed one, so a
/// destination that already looks like a temp file still cannot collide.
fn temp_name(destination: &Path, counter: u64) -> String {
    let dot_target = destination
        .file_name()
        .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."));
    let prefix = if dot_target { "" } else { "." };
    format!("{prefix}screenphoto-{}-{counter}.tmp", std::process::id())
}

#[cfg(windows)]
fn replace_file(temp: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let from = wide(temp);
    let to = wide(destination);
    let result = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(temp, destination)
}

/// Whether writing `output` would replace `source`: same file identity, or the
/// same normalized absolute path (including paths that do not exist yet).
pub(super) fn output_aliases_source(source: &Path, output: &Path) -> bool {
    if same_file::is_same_file(source, output).unwrap_or(false) {
        return true;
    }
    match (normalized_absolute(source), normalized_absolute(output)) {
        (Some(source), Some(output)) => source == output,
        _ => false,
    }
}

fn normalized_absolute(path: &Path) -> Option<PathBuf> {
    let absolute = std::path::absolute(path).ok()?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Some(normalized)
}

#[cfg(test)]
mod tests {
    use super::{
        jpeg_dimensions, output_aliases_source, read_capped, temp_name, write_output,
        write_output_with,
    };
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(tag: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "screenphoto-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn temp_file(name: &str, data: &[u8]) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "screenphoto-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, data).expect("write temp file");
        path
    }

    #[test]
    fn capped_read_stops_at_limit_plus_one() {
        let path = temp_file("capped.bin", b"abcdef");
        assert_eq!(read_capped(&path, 3).expect("read"), b"abcd".to_vec());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn jpeg_dimensions_come_from_the_frame_header() {
        let image = image::RgbImage::from_pixel(3, 2, image::Rgb([10, 20, 30]));
        let mut jpeg = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .expect("encode JPEG");
        let jpeg = jpeg.into_inner();
        assert_eq!(jpeg_dimensions(&jpeg), Some((3, 2)));
        assert_eq!(jpeg_dimensions(&jpeg[..2]), None);
        assert_eq!(jpeg_dimensions(b"not a jpeg"), None);
    }

    #[test]
    fn output_write_replaces_existing_file() {
        let dir = temp_dir("replace");
        let output = dir.join("output.jpg");
        std::fs::write(&output, b"old").expect("old output");
        write_output(&output, b"new").expect("replace output");
        assert_eq!(std::fs::read(&output).expect("output bytes"), b"new");
    }

    #[test]
    fn failed_output_write_keeps_target_and_cleans_temp_file() {
        let dir = temp_dir("failed-write");
        let output = dir.join("output.jpg");
        std::fs::create_dir(&output).expect("blocking directory");
        assert!(write_output(&output, b"jpeg").is_err());
        assert!(output.is_dir());
        let leftovers = std::fs::read_dir(&dir)
            .expect("list directory")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn existing_temp_collision_preserves_both_files() {
        let dir = temp_dir("collision");
        let output = dir.join("output.jpg");
        std::fs::write(&output, b"old").expect("old output");
        let temp = dir.join(temp_name(&output, 42));
        std::fs::write(&temp, b"pre-existing").expect("pre-existing temp");

        let error = write_output_with(&output, b"new", 42).expect_err("collision must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&temp).expect("temp intact"), b"pre-existing");
        assert_eq!(std::fs::read(&output).expect("target intact"), b"old");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn long_destination_name_is_written_without_leftovers() {
        let dir = temp_dir("long-name");
        let output = dir.join("a".repeat(255));
        std::fs::write(&output, b"old").expect("long destination");
        write_output(&output, b"jpeg").expect("replace long destination");
        assert_eq!(std::fs::read(&output).expect("output bytes"), b"jpeg");
        assert_eq!(
            std::fs::read_dir(&dir).expect("list directory").count(),
            1,
            "temp file left behind"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn temp_looking_destination_does_not_collide_with_the_temp_name() {
        let pid = std::process::id();
        for (tag, name, counter) in [
            ("plain", format!("screenphoto-{pid}-7.tmp"), 7),
            ("dot", format!(".screenphoto-{pid}-8.tmp"), 8),
            ("plain-upper", format!("SCREENPHOTO-{pid}-9.TMP"), 9),
            ("dot-upper", format!(".SCREENPHOTO-{pid}-10.TMP"), 10),
        ] {
            let dir = temp_dir(tag);
            let output = dir.join(name);
            std::fs::write(&output, b"old").expect("existing destination");
            write_output_with(&output, tag.as_bytes(), counter).expect(tag);
            assert_eq!(std::fs::read(&output).expect(tag), tag.as_bytes());
            assert_eq!(
                std::fs::read_dir(&dir).expect("list directory").count(),
                1,
                "{tag}: unexpected extra files"
            );
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn output_aliases_of_the_source_are_detected() {
        let dir = temp_dir("aliases");
        let source = dir.join("source.png");
        std::fs::write(&source, b"original").expect("source");

        assert!(output_aliases_source(&source, &source));
        assert!(output_aliases_source(
            &source,
            &dir.join(".").join("source.png")
        ));
        assert!(!output_aliases_source(&source, &dir.join("out.jpg")));

        let hard = dir.join("hard.png");
        std::fs::hard_link(&source, &hard).expect("hard link");
        assert!(output_aliases_source(&source, &hard));

        #[cfg(unix)]
        {
            let soft = dir.join("soft.png");
            std::os::unix::fs::symlink(&source, &soft).expect("symlink");
            assert!(output_aliases_source(&source, &soft));
        }

        assert_eq!(
            std::fs::read(&source).expect("source intact").as_slice(),
            b"original"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
