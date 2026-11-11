//! Lanczos3 resampling for low-resolution auxiliary grids.

use crate::frame::Frame;

fn lanczos3(x: f32) -> f32 {
    let x = x.abs();
    if x < 1e-7 {
        1.0
    } else if x >= 3.0 {
        0.0
    } else {
        let pix = std::f32::consts::PI * x;
        (pix.sin() / pix) * (pix / 3.0).sin() / (pix / 3.0)
    }
}

fn axis_weights(input: usize, output: usize) -> Vec<Vec<(usize, f32)>> {
    if input == 0 || output == 0 {
        return Vec::new();
    }
    let scale = input as f32 / output as f32;
    let filter_scale = scale.max(1.0);
    let support = 3.0 * filter_scale;
    let last = input - 1;
    let mut table = Vec::with_capacity(output);
    for out in 0..output {
        let center = (out as f32 + 0.5) * scale - 0.5;
        let from = (center - support).ceil() as i64;
        let to = (center + support).floor() as i64;
        let mut taps: Vec<(usize, f32)> = Vec::new();
        let mut total = 0.0_f32;
        for tap in from..=to {
            let weight = lanczos3((tap as f32 - center) / filter_scale);
            if weight == 0.0 {
                continue;
            }
            let clamp = tap.clamp(0, last as i64) as usize;
            taps.push((clamp, weight));
            total += weight;
        }
        if total.abs() < 1e-8 {
            taps.clear();
            total = 1.0;
            taps.push((center.round().clamp(0.0, last as f32) as usize, 1.0));
        }
        for (_, weight) in &mut taps {
            *weight /= total;
        }
        table.push(taps);
    }
    table
}

/// Float HDR-safe separable Lanczos3 resampler (no 8-bit quantisation).
pub(crate) fn resize(src: &Frame, width: usize, height: usize) -> Frame {
    if width == 0 || height == 0 {
        return Frame::zeros(width, height);
    }
    if src.width == 0 || src.height == 0 {
        return Frame::zeros(width, height);
    }
    if (src.width, src.height) == (width, height) {
        return src.clone();
    }
    let src_pixels = src.pixels();
    if src.width == width {
        let weights = axis_weights(src.height, height);
        let mut out = Frame::zeros(width, height);
        for (y, taps) in weights.iter().enumerate() {
            for x in 0..width {
                let mut sum = [0.0_f32; 3];
                for &(source_y, weight) in taps {
                    let pixel = src_pixels[source_y * src.width + x];
                    for c in 0..3 {
                        sum[c] += pixel[c] * weight;
                    }
                }
                out.pixels_mut()[y * width + x] = sum;
            }
        }
        return out;
    }
    let weights_x = axis_weights(src.width, width);
    let mut intermediate = Frame::zeros(width, src.height);
    for y in 0..src.height {
        let row = y * src.width;
        for (x, taps) in weights_x.iter().enumerate() {
            let mut sum = [0.0_f32; 3];
            for &(source_x, weight) in taps {
                let pixel = src_pixels[row + source_x];
                for c in 0..3 {
                    sum[c] += pixel[c] * weight;
                }
            }
            intermediate.pixels_mut()[y * width + x] = sum;
        }
    }
    if src.height == height {
        return intermediate;
    }
    let weights_y = axis_weights(src.height, height);
    let mut out = Frame::zeros(width, height);
    for (y, taps) in weights_y.iter().enumerate() {
        for x in 0..width {
            let mut sum = [0.0_f32; 3];
            for &(source_y, weight) in taps {
                let pixel = intermediate.pixels()[source_y * width + x];
                for c in 0..3 {
                    sum[c] += pixel[c] * weight;
                }
            }
            out.pixels_mut()[y * width + x] = sum;
        }
    }
    out
}
