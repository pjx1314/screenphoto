use std::path::Path;

use crate::devtools::SAMPLE_SEED;
use crate::devtools::util::{self, Result};
use crate::render_bytes;
use image::imageops::{FilterType, crop_imm, overlay, resize};
use image::{Rgb, RgbImage};

const BACKGROUND: [u8; 3] = [65, 65, 65];
const LABEL: [u8; 3] = [255, 255, 255];

pub fn run(root: &Path, out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    let samples = root.join("samples");

    let (photo, _) = render_bytes(&std::fs::read(samples.join("test.jpg"))?, Some(SAMPLE_SEED))?;
    std::fs::write(out.join("photo.jpg"), &photo)?;

    let photo_image = util::to_rgb(&util::decode(&photo)?);
    let thumbnail = util::fit(&photo_image, 360, 360);
    std::fs::write(
        out.join("thumbnail.jpg"),
        util::encode_jpeg(&thumbnail, 88)?,
    )?;

    let social = std::fs::read(samples.join("social.jpg"))?;
    let (social_photo, _) = render_bytes(&social, Some(SAMPLE_SEED))?;
    std::fs::write(out.join("social-photo.jpg"), &social_photo)?;

    let poster = std::fs::read(samples.join("poster.jpg"))?;
    let (poster_photo, _) = render_bytes(&poster, Some(SAMPLE_SEED))?;
    std::fs::write(out.join("poster-photo.jpg"), &poster_photo)?;

    reference_sheet(&samples, out, &social)?;
    seeds_sheet(out, &social, &social_photo)?;
    contact_sheet(&root.join("tests/fixtures/bot_realism"), out)?;

    println!(
        "gallery: {} photo, social-photo, poster-photo, thumbnail, reference, seeds, corpus contact sheet",
        out.display()
    );
    println!(
        "reference sheet is a morphology comparison; source content differs from the photographic references"
    );
    Ok(())
}

fn reference_sheet(samples: &Path, out: &Path, social: &[u8]) -> Result<()> {
    let mut sheet = RgbImage::from_pixel(1230, 1160, Rgb(BACKGROUND));
    let reference_two = samples.join("_ref/ref2_macbook.jpg");
    let reference_one = samples.join("_ref/ref1_monitor.jpg");
    let rendered_page = out.join("social-photo.jpg");
    let panels: [(&Path, i32, i32, u32, u32, &str); 3] = [
        (
            reference_two.as_path(),
            0,
            24,
            602,
            803,
            "Reference 2 (photo)",
        ),
        (
            reference_one.as_path(),
            0,
            864,
            602,
            275,
            "Reference 1 (photo)",
        ),
        (
            rendered_page.as_path(),
            620,
            24,
            602,
            843,
            "Rendered social page",
        ),
    ];
    for (path, x, y, max_width, max_height, label) in panels {
        let image = util::to_rgb(&util::decode(&std::fs::read(path)?)?);
        let panel = util::fit(&image, max_width, max_height);
        overlay(&mut sheet, &panel, x as i64, y as i64);
        util::draw_text(&mut sheet, x + 5, y - 18, label, LABEL);
    }

    let source = util::to_rgb(&util::decode(social)?);
    let crop_width = source.width();
    let crop_height = (crop_width as f32 * 271.0 / 602.0).round() as u32;
    let crop = crop_imm(&source, 0, 0, crop_width, crop_height.min(source.height())).to_image();
    let crop = resize(&crop, 602, 271, FilterType::Lanczos3);
    overlay(&mut sheet, &crop, 620, 886);
    util::draw_text(
        &mut sheet,
        625,
        868,
        "Clean source (crop, different content from references)",
        LABEL,
    );

    std::fs::write(out.join("reference.jpg"), util::encode_jpeg(&sheet, 92)?)?;
    Ok(())
}

fn seeds_sheet(out: &Path, social: &[u8], social_photo: &[u8]) -> Result<()> {
    let clean = util::to_rgb(&util::decode(social)?);
    let rendered_social = util::to_rgb(&util::decode(social_photo)?);
    let mut sheet = RgbImage::from_pixel(1230, 475, Rgb(BACKGROUND));

    let seeds: [Option<u64>; 4] = [None, Some(7), Some(42), Some(SAMPLE_SEED)];
    for (index, seed) in seeds.into_iter().enumerate() {
        let image = match seed {
            None => clean.clone(),
            Some(SAMPLE_SEED) => rendered_social.clone(),
            Some(seed) => {
                let (jpeg, _) = render_bytes(social, Some(seed))?;
                util::to_rgb(&util::decode(&jpeg)?)
            }
        };
        let panel = util::fit(&image, 300, 420);
        let x = index as i32 * 310;
        overlay(&mut sheet, &panel, x as i64, 28);
        let label = match seed {
            None => "Clean source".to_string(),
            Some(seed) => format!("Capture - seed {seed}"),
        };
        util::draw_text(&mut sheet, x + 5, 6, &label, LABEL);
    }

    std::fs::write(out.join("seeds.jpg"), util::encode_jpeg(&sheet, 92)?)?;
    Ok(())
}

fn contact_sheet(fixtures: &Path, out: &Path) -> Result<()> {
    const COLUMNS: u32 = 4;
    const CELL_WIDTH: u32 = 288;
    const CELL_HEIGHT: u32 = 280;
    const IMAGE_HEIGHT: u32 = 260;
    const GAP: u32 = 12;
    const MARGIN: u32 = 12;
    const LABEL_HEIGHT: u32 = 16;

    let manifest = util::read_manifest(fixtures)?;
    let rows = (manifest.images.len() as u32).div_ceil(COLUMNS);
    let width = MARGIN * 2 + COLUMNS * CELL_WIDTH + (COLUMNS - 1) * GAP;
    let height = MARGIN * 2 + rows * CELL_HEIGHT + rows.saturating_sub(1) * GAP;
    let mut sheet = RgbImage::from_pixel(width, height, Rgb(BACKGROUND));

    for (index, entry) in manifest.images.iter().enumerate() {
        let index = index as u32;
        let column = index % COLUMNS;
        let row = index / COLUMNS;
        let cell_x = MARGIN + column * (CELL_WIDTH + GAP);
        let cell_y = MARGIN + row * (CELL_HEIGHT + GAP);

        let bytes = std::fs::read(fixtures.join(&entry.file))?;
        let (jpeg, _) = render_bytes(&bytes, Some(SAMPLE_SEED))?;
        let image = util::to_rgb(&util::decode(&jpeg)?);
        let panel = util::fit(&image, CELL_WIDTH, IMAGE_HEIGHT);
        let panel_x = cell_x + (CELL_WIDTH - panel.width()) / 2;
        overlay(
            &mut sheet,
            &panel,
            panel_x as i64,
            (cell_y + LABEL_HEIGHT) as i64,
        );

        let label = format!("{} {}x{}", entry.file, entry.size[0], entry.size[1]);
        util::draw_text(&mut sheet, cell_x as i32, cell_y as i32, &label, LABEL);
    }

    std::fs::write(out.join("corpus.jpg"), util::encode_jpeg(&sheet, 92)?)?;
    Ok(())
}
