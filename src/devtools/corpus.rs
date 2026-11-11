use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use image::{ColorType, GenericImageView, RgbImage};

use crate::devtools::util::{self, Result};

const REQUIRED_KEY_REGIONS: usize = 69;
const MIN_GRAY_RANGE: i32 = 80;

pub fn run(fixtures: &Path) -> Result<()> {
    let manifest = util::read_manifest(fixtures)?;
    let mut failures = Vec::new();

    if manifest.schema != 1 {
        failures.push(format!(
            "manifest schema is {}, expected 1",
            manifest.schema
        ));
    }
    if manifest.image_count != manifest.images.len() {
        failures.push(format!(
            "image_count is {}, manifest lists {}",
            manifest.image_count,
            manifest.images.len()
        ));
    }
    if manifest.images.len() != 12 {
        failures.push(format!(
            "expected 12 corpus images, found {}",
            manifest.images.len()
        ));
    }

    let mut names = HashSet::new();
    for entry in &manifest.images {
        if !names.insert(entry.file.as_str()) {
            failures.push(format!("duplicate manifest file: {}", entry.file));
        }
    }

    let mut on_disk = BTreeSet::new();
    for entry in std::fs::read_dir(fixtures)? {
        let entry = entry?;
        let path = entry.path();
        if is_image(&path) {
            on_disk.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    let declared = manifest
        .images
        .iter()
        .map(|entry| entry.file.clone())
        .collect::<BTreeSet<_>>();
    for missing in declared.difference(&on_disk) {
        failures.push(format!("manifest entry missing on disk: {missing}"));
    }
    for extra in on_disk.difference(&declared) {
        failures.push(format!("image not listed in manifest: {extra}"));
    }

    let mut region_count = 0usize;
    for entry in &manifest.images {
        let path = fixtures.join(&entry.file);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{}: {error}", entry.file));
                continue;
            }
        };
        let digest = util::sha256_hex(&bytes);
        if digest != entry.sha256 {
            failures.push(format!(
                "{}: sha256 {digest}, manifest {}",
                entry.file, entry.sha256
            ));
        }

        let image = match util::decode(&bytes) {
            Ok(image) => image,
            Err(error) => {
                failures.push(format!("{}: cannot decode: {error}", entry.file));
                continue;
            }
        };
        let (width, height) = image.dimensions();
        if [width, height] != entry.size {
            failures.push(format!(
                "{}: size {width}x{height}, manifest {}x{}",
                entry.file, entry.size[0], entry.size[1]
            ));
        }
        if let Some(mode) = color_mode(image.color()) {
            if mode != entry.mode.as_str() {
                failures.push(format!(
                    "{}: mode {mode}, manifest {}",
                    entry.file, entry.mode
                ));
            }
        } else {
            failures.push(format!(
                "{}: unsupported color type {:?}",
                entry.file,
                image.color()
            ));
        }
        match format_of(&path) {
            Some(format) if format == entry.format.as_str() => {}
            Some(format) => failures.push(format!(
                "{}: format {format}, manifest {}",
                entry.file, entry.format
            )),
            None => failures.push(format!("{}: unsupported extension", entry.file)),
        }

        let mut region_names = HashSet::new();
        let rgb = util::to_rgb(&image);
        for region in &entry.key_regions {
            region_count += 1;
            if !region_names.insert(region.name.as_str()) {
                failures.push(format!(
                    "{}: duplicate key region {}",
                    entry.file, region.name
                ));
            }
            let [x0, y0, x1, y1] = region.bounds;
            if x0 >= x1 || y0 >= y1 || x1 > width || y1 > height {
                failures.push(format!(
                    "{}:{} outside image {}x{}",
                    entry.file, region.name, width, height
                ));
                continue;
            }
            if (x1 - x0) * (y1 - y0) < 100 {
                failures.push(format!("{}:{} is degenerate", entry.file, region.name));
            }
            let span = gray_span(&rgb, x0, y0, x1, y1);
            if span < MIN_GRAY_RANGE {
                failures.push(format!(
                    "{}:{} has grayscale range {span}, expected at least {MIN_GRAY_RANGE}",
                    entry.file, region.name
                ));
            }
        }
    }

    if region_count != REQUIRED_KEY_REGIONS {
        failures.push(format!(
            "expected {REQUIRED_KEY_REGIONS} key regions, checked {region_count}"
        ));
    }

    if failures.is_empty() {
        println!(
            "verify-corpus: {} images, {} key regions OK",
            manifest.images.len(),
            region_count
        );
        Ok(())
    } else {
        for failure in &failures {
            println!("  FAIL {failure}");
        }
        Err(util::Error::internal(format!(
            "{} corpus checks failed",
            failures.len()
        )))
    }
}

// Fixture contrast is measured in encoded BT.601 grayscale, not the
// linear-light Rec.709 luminance used by the renderer's exposure meter.
fn gray_span(image: &RgbImage, x0: u32, y0: u32, x1: u32, y1: u32) -> i32 {
    let (mut low, mut high) = (255, 0);
    for y in y0..y1 {
        for x in x0..x1 {
            let [r, g, b] = image.get_pixel(x, y).0.map(i32::from);
            let gray = (299 * r + 587 * g + 114 * b) / 1000;
            low = low.min(gray);
            high = high.max(gray);
        }
    }
    high - low
}

fn color_mode(color: ColorType) -> Option<&'static str> {
    match color {
        ColorType::Rgb8 => Some("RGB"),
        ColorType::Rgba8 => Some("RGBA"),
        ColorType::L8 => Some("L"),
        ColorType::La8 => Some("LA"),
        _ => None,
    }
}

fn format_of(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("PNG"),
        "jpg" | "jpeg" => Some("JPEG"),
        _ => None,
    }
}

fn is_image(path: &Path) -> bool {
    format_of(path).is_some()
}
