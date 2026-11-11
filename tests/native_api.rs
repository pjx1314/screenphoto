use std::io::Cursor;

use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use screenphoto::{render_bytes, render_rgb};

fn png(image: RgbImage) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn markers(width: u32, height: u32) -> Vec<u8> {
    let marker = (width.min(height) / 4).max(4);
    png(RgbImage::from_fn(width, height, |x, y| {
        let rgb = match (
            x < marker,
            x >= width - marker,
            y < marker,
            y >= height - marker,
        ) {
            (true, _, true, _) => [255, 0, 0],
            (_, true, true, _) => [0, 255, 0],
            (_, true, _, true) => [0, 0, 255],
            (true, _, _, true) => [255, 0, 255],
            _ => [255; 3],
        };
        Rgb(rgb)
    }))
}

fn photo(data: &[u8], seed: u64) -> RgbImage {
    let (jpeg, resolved) = render_bytes(data, Some(seed)).unwrap();
    assert_eq!(resolved, seed);
    image::load_from_memory(&jpeg).unwrap().into_rgb8()
}

#[test]
fn resolved_seed_reproduces_the_capture_and_explicit_seeds_change_it() {
    let source = png(RgbImage::from_pixel(63, 41, Rgb([40, 90, 150])));
    let (first, seed) = render_bytes(&source, None).unwrap();
    assert!(seed <= 0x7fff_ffff);
    assert_eq!(first, render_bytes(&source, Some(seed)).unwrap().0);
    assert_ne!(
        render_bytes(&source, Some(7)).unwrap().0,
        render_bytes(&source, Some(42)).unwrap().0,
    );
    let (max_first, max_seed) = render_bytes(&source, Some(u64::MAX)).unwrap();
    assert_eq!(max_seed, u64::MAX);
    assert_eq!(max_first, render_bytes(&source, Some(u64::MAX)).unwrap().0);
}

#[test]
fn raw_rgb_render_matches_the_png_path_and_rejects_bad_buffers() {
    let image = RgbImage::from_fn(37, 23, |x, y| Rgb([(x * 7) as u8, (y * 11) as u8, 90]));
    let (from_rgb, seed_rgb) = render_rgb(37, 23, image.clone().into_raw(), Some(7)).unwrap();
    let (from_png, seed_png) = render_bytes(&png(image), Some(7)).unwrap();
    assert_eq!(seed_rgb, seed_png);
    assert_eq!(from_rgb, from_png);

    assert!(
        render_rgb(0, 1, Vec::new(), Some(7))
            .unwrap_err()
            .is_input()
    );
    assert!(
        render_rgb(2, 2, vec![0; 3], Some(7))
            .unwrap_err()
            .is_input()
    );
}

#[test]
fn output_is_capped_at_1920_and_only_shrinks() {
    let large = photo(&markers(2000, 1000), 7);
    assert_eq!(large.dimensions(), (1920, 960));
    let small = photo(&markers(320, 181), 7);
    assert_eq!(small.dimensions(), (320, 181));
}

#[test]
fn bounded_crop_preserves_inset_subjects_for_landscape_portrait_and_long_pages() {
    for (width, height) in [(240, 160), (160, 240), (60, 360)] {
        let radius = width.min(height) / 8;
        let centres = [
            (width / 4, height / 4, [255, 0, 0]),
            (width * 3 / 4, height / 4, [0, 255, 0]),
            (width * 3 / 4, height * 3 / 4, [0, 0, 255]),
            (width / 4, height * 3 / 4, [255, 0, 255]),
        ];
        let source = png(RgbImage::from_fn(width, height, |x, y| {
            let colour = centres
                .iter()
                .find(|&&(cx, cy, _)| x.abs_diff(cx) <= radius && y.abs_diff(cy) <= radius)
                .map_or([255; 3], |&(_, _, colour)| colour);
            Rgb(colour)
        }));
        let output = photo(&source, 7);
        assert_eq!(output.dimensions(), (width, height));
        let mut positions = [[0_u64; 3]; 4];
        for (x, y, pixel) in output.enumerate_pixels() {
            let [r, g, b] = pixel.0.map(i16::from);
            let classes = [
                r > g + 40 && r > b + 40,
                g > r + 40 && g > b + 40,
                b > r + 40 && b > g + 40,
                r > g + 40 && b > g + 40,
            ];
            for (position, found) in positions.iter_mut().zip(classes) {
                if found {
                    position[0] += u64::from(x);
                    position[1] += u64::from(y);
                    position[2] += 1;
                }
            }
        }
        for (position, (right, bottom)) in
            positions
                .into_iter()
                .zip([(false, false), (true, false), (true, true), (false, true)])
        {
            let [sum_x, sum_y, count] = position;
            assert!(count > 20, "inset subject lost in {width}x{height}");
            let x = sum_x as f64 / count as f64 / f64::from(width);
            let y = sum_y as f64 / count as f64 / f64::from(height);
            assert!(if right { x > 0.6 } else { x < 0.4 });
            assert!(if bottom { y > 0.6 } else { y < 0.4 });
        }
    }
}

#[test]
fn tiny_single_axis_and_odd_sources_keep_their_bounds() {
    for (width, height) in [(1, 1), (1, 7), (7, 1), (37, 23)] {
        let source = png(RgbImage::from_pixel(width, height, Rgb([230, 180, 90])));
        let output = photo(&source, 7);
        assert_eq!(output.dimensions(), (width, height));
        let mean = output.pixels().map(|p| u64::from(p.0[0])).sum::<u64>() as f64
            / f64::from(width * height);
        assert!(mean > 30.0, "tiny source lost its emitted content");
    }
}

#[test]
fn corrupt_sources_fail() {
    assert!(render_bytes(b"corrupt", None).unwrap_err().is_input());
}
