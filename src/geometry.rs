//! Pinhole framing, 3x3 projective transforms and the shared inverse sampler.
//!
//! The frame is the camera photo; the screen (screenshot) is the source. Camera
//! fitting keeps screenshot samples inside the source by bounded cropping and
//! reduced pose/lens strength. Reflection samples may extend edge pixels.
//! Source and output use pixel-boundary coordinates: an output sample centre
//! is `x + 0.5`, mapped and shifted by `-0.5` into source pixel-index space.

use image::RgbImage;

use crate::color::{DISPLAY_EMISSION, srgb_lut};
use crate::frame::Frame;

pub(crate) type Mat3 = [[f64; 3]; 3];

#[derive(Clone, Copy, Debug)]
pub(crate) struct Lens {
    pub distortion: f32,
    pub center: (f32, f32),
}

#[cfg(test)]
const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

#[inline]
pub(crate) fn apply_h(m: &Mat3, x: f64, y: f64) -> (f64, f64) {
    let w = m[2][0] * x + m[2][1] * y + m[2][2];
    (
        (m[0][0] * x + m[0][1] * y + m[0][2]) / w,
        (m[1][0] * x + m[1][1] * y + m[1][2]) / w,
    )
}

/// Solve an 8x8 dense system with partial pivoting.
fn solve8(mut a: [[f64; 8]; 8], mut b: [f64; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let mut pivot = col;
        for row in col + 1..8 {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if !a[pivot][col].is_finite() || a[pivot][col].abs() < 1e-12 {
            return None;
        }
        if pivot != col {
            a.swap(pivot, col);
            b.swap(pivot, col);
        }
        let lead = a[col][col];
        let (upper, lower) = a.split_at_mut(col + 1);
        for (row, values) in lower.iter_mut().enumerate() {
            let factor = values[col] / lead;
            if factor != 0.0 {
                for (value, pivot_value) in values[col..].iter_mut().zip(&upper[col][col..]) {
                    *value -= factor * pivot_value;
                }
                b[col + 1 + row] -= factor * b[col];
            }
        }
    }
    let mut x = [0.0; 8];
    for row in (0..8).rev() {
        let mut acc = b[row];
        for k in row + 1..8 {
            acc -= a[row][k] * x[k];
        }
        x[row] = acc / a[row][row];
    }
    Some(x)
}

/// Hartley normalisation: translate to the centroid and scale the mean radius
/// to `sqrt(2)`, so the DLT is solved in well-conditioned coordinates even for
/// multi-thousand-pixel screenshots.
fn normalize(points: &[[f64; 2]; 4]) -> (Mat3, Mat3) {
    let cx = points.iter().map(|p| p[0]).sum::<f64>() / 4.0;
    let cy = points.iter().map(|p| p[1]).sum::<f64>() / 4.0;
    let mean = points
        .iter()
        .map(|p| ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt())
        .sum::<f64>()
        / 4.0;
    assert!(
        mean.is_finite() && mean > 0.0,
        "nondegenerate internal screen corners"
    );
    let scale = std::f64::consts::SQRT_2 / mean;
    (
        [
            [scale, 0.0, -scale * cx],
            [0.0, scale, -scale * cy],
            [0.0, 0.0, 1.0],
        ],
        [
            [1.0 / scale, 0.0, cx],
            [0.0, 1.0 / scale, cy],
            [0.0, 0.0, 1.0],
        ],
    )
}

/// DLT homography mapping four source corners onto four destination corners.
/// The result is normalised so its bottom-right entry is 1, matching the
/// screen-to-frame convention used by the earlier implementation.
fn homography(src: &[[f64; 2]; 4], dst: &[[f64; 2]; 4]) -> Mat3 {
    let (src_norm, _) = normalize(src);
    let (dst_norm, dst_inv) = normalize(dst);
    let mut normalized_src = [[0.0; 2]; 4];
    let mut normalized_dst = [[0.0; 2]; 4];
    for i in 0..4 {
        let (x, y) = apply_h(&src_norm, src[i][0], src[i][1]);
        normalized_src[i] = [x, y];
        let (u, v) = apply_h(&dst_norm, dst[i][0], dst[i][1]);
        normalized_dst[i] = [u, v];
    }

    let mut a = [[0.0; 8]; 8];
    let mut b = [0.0; 8];
    for i in 0..4 {
        let (x, y) = (normalized_src[i][0], normalized_src[i][1]);
        let (u, v) = (normalized_dst[i][0], normalized_dst[i][1]);
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y];
        b[2 * i] = u;
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y];
        b[2 * i + 1] = v;
    }
    let h = solve8(a, b).expect("nondegenerate fitted screen homography");
    let normalized_h = [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]];
    let h = matmul(&matmul(&dst_inv, &normalized_h), &src_norm);
    let w = h[2][2];
    assert!(
        w.is_finite() && w != 0.0,
        "finite internal screen homography"
    );
    h.map(|row| row.map(|value| value / w))
}

fn inverse(m: &Mat3) -> Mat3 {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    assert!(
        det.is_finite() && det != 0.0,
        "invertible internal screen homography"
    );
    let inv_det = 1.0 / det;
    [
        [
            (e * i - f * h) * inv_det,
            (c * h - b * i) * inv_det,
            (b * f - c * e) * inv_det,
        ],
        [
            (f * g - d * i) * inv_det,
            (a * i - c * g) * inv_det,
            (c * d - a * f) * inv_det,
        ],
        [
            (d * h - e * g) * inv_det,
            (b * g - a * h) * inv_det,
            (a * e - b * d) * inv_det,
        ],
    ]
}

fn matmul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut out = [[0.0; 3]; 3];
    for (row, out_row) in out.iter_mut().enumerate() {
        for (col, value) in out_row.iter_mut().enumerate() {
            *value = (0..3).map(|k| a[row][k] * b[k][col]).sum();
        }
    }
    out
}

fn scale_matrix(sx: f64, sy: f64) -> Mat3 {
    [[sx, 0.0, 0.0], [0.0, sy, 0.0], [0.0, 0.0, 1.0]]
}

/// Radial lens mapping from frame coordinates to sampling coordinates.
///
/// Positive distortion pushes sample positions outward, so the content seen in
/// the frame shrinks. Phase and content warp share this exact map.
#[inline]
pub(crate) fn lens_coords(x: f64, y: f64, w: usize, h: usize, lens: Lens) -> (f64, f64) {
    let half = w.max(h) as f64 / 2.0;
    let cx = lens.center.0 as f64 * w as f64;
    let cy = lens.center.1 as f64 * h as f64;
    let dx = x - cx;
    let dy = y - cy;
    let k = 1.0 + lens.distortion as f64 * (dx * dx + dy * dy) / (half * half);
    (cx + dx * k, cy + dy * k)
}

/// Jacobian of the radial lens map at frame coordinate `(x, y)`.
///
/// Display carrier frequencies are differentiated through this matrix; the
/// entries are `d(sample) / d(frame)` under the same normalisation as
/// [`lens_coords`]. Rows are `(dx, dy)`, columns the sampled axes.
#[inline]
pub(crate) fn lens_jacobian(x: f64, y: f64, w: usize, h: usize, lens: Lens) -> [[f64; 2]; 2] {
    let half = w.max(h) as f64 / 2.0;
    let cx = lens.center.0 as f64 * w as f64;
    let cy = lens.center.1 as f64 * h as f64;
    let dx = (x - cx) / half;
    let dy = (y - cy) / half;
    let distortion = lens.distortion as f64;
    let r2 = dx * dx + dy * dy;
    let jxx = 1.0 + distortion * (r2 + 2.0 * dx * dx);
    let jxy = 2.0 * distortion * dx * dy;
    let jyy = 1.0 + distortion * (r2 + 2.0 * dy * dy);
    [[jxx, jxy], [jxy, jyy]]
}

/// Where the four screen corners land in the photo, TL/TR/BR/BL.
///
/// This is the real pinhole projection (with its pre-roll fit into the frame)
/// followed by the final whole-quad fit, so the caller never applies a second
/// fit and every corner stays inside the output bounds.
fn camera_quad(
    sw: usize,
    sh: usize,
    w: usize,
    h: usize,
    pose: (f64, f64, f64),
    distance_w: f64,
) -> [[f64; 2]; 4] {
    let (yaw, pitch, roll) = pose;
    let wm = sw as f64;
    let hm = sh as f64;
    let d = distance_w * wm;

    // R = rot_y(yaw) @ rot_x(pitch); world = R * screen points + [0, 0, d].
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sx, cx) = pitch.to_radians().sin_cos();
    let r = [
        [cy, sy * sx, sy * cx],
        [0.0, cx, -sx],
        [-sy, cy * sx, cy * cx],
    ];

    let corners = [
        [-wm / 2.0, -hm / 2.0],
        [wm / 2.0, -hm / 2.0],
        [wm / 2.0, hm / 2.0],
        [-wm / 2.0, hm / 2.0],
    ];

    let mut u = [0.0; 4];
    let mut v = [0.0; 4];
    for (i, [x, y]) in corners.into_iter().enumerate() {
        let wx = r[0][0] * x + r[0][1] * y;
        let wy = r[1][0] * x + r[1][1] * y;
        let wz = r[2][0] * x + r[2][1] * y + d;
        u[i] = wx / wz;
        v[i] = wy / wz;
    }

    // Pre-roll fit: centre, then scale by whichever axis binds first (fill=1).
    let cu = u.iter().sum::<f64>() / 4.0;
    let cv = v.iter().sum::<f64>() / 4.0;
    let span_u = (u.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - u.iter().cloned().fold(f64::INFINITY, f64::min))
    .max(1e-9);
    let span_v = (v.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - v.iter().cloned().fold(f64::INFINITY, f64::min))
    .max(1e-9);
    let k = (w as f64 / span_u).min(h as f64 / span_v);
    let (sa, ca) = roll.to_radians().sin_cos();
    for i in 0..4 {
        let du = (u[i] - cu) * k;
        let dv = (v[i] - cv) * k;
        u[i] = du * ca - dv * sa + w as f64 * 0.5;
        v[i] = du * sa + dv * ca + h as f64 * 0.5;
    }

    // Final whole-quad fit: the roll can push corners past the pre-roll bounds.
    let low_u = u.iter().cloned().fold(f64::INFINITY, f64::min);
    let high_u = u.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let low_v = v.iter().cloned().fold(f64::INFINITY, f64::min);
    let high_v = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let factor = (w as f64 / (high_u - low_u)).min(h as f64 / (high_v - low_v));
    let mid_u = (low_u + high_u) * 0.5;
    let mid_v = (low_v + high_v) * 0.5;
    let mut quad = [[0.0; 2]; 4];
    for i in 0..4 {
        quad[i] = [
            ((u[i] - mid_u) * factor + w as f64 * 0.5).clamp(0.0, w as f64),
            ((v[i] - mid_v) * factor + h as f64 * 0.5).clamp(0.0, h as f64),
        ];
    }
    quad
}

fn screen_to_frame_matrix(sw: usize, sh: usize, quad: &[[f64; 2]; 4]) -> Mat3 {
    let src = [
        [0.0, 0.0],
        [sw as f64, 0.0],
        [sw as f64, sh as f64],
        [0.0, sh as f64],
    ];
    homography(&src, quad)
}

/// Screen-to-frame projection with its inverse cached.
///
/// Fits the camera pose, lens and bounded crop together. Content, display
/// carriers and reflection use the same fitted geometry. The auxiliary matrix
/// composes the two low-resolution rescales around the same homography.
#[derive(Clone, Copy)]
pub(crate) struct Projection {
    screen: (usize, usize),
    frame: (usize, usize),
    screen_to_frame: Mat3,
    frame_to_screen: Mat3,
    lens: Lens,
}

impl Projection {
    pub(crate) fn camera(
        screen: (usize, usize),
        frame: (usize, usize),
        pose: (f64, f64, f64),
        distance_w: f64,
        lens: Lens,
        crop_limit: f64,
    ) -> Self {
        let (sw, sh) = screen;
        let (w, h) = frame;
        let mut projection = Self {
            screen,
            frame,
            screen_to_frame: scale_matrix(w as f64 / sw as f64, h as f64 / sh as f64),
            frame_to_screen: scale_matrix(sw as f64 / w as f64, sh as f64 / h as f64),
            lens: Lens {
                distortion: 0.0,
                ..lens
            },
        };
        // A single-pixel axis has no room for a two-dimensional crop.
        if sw.min(sh).min(w).min(h) == 1 {
            return projection;
        }
        let candidate = |strength: f64| {
            let angles = (pose.0 * strength, pose.1 * strength, pose.2 * strength);
            let quad = camera_quad(sw, sh, w, h, angles, distance_w);
            let screen_to_frame = screen_to_frame_matrix(sw, sh, &quad);
            let fitted = Self {
                screen,
                frame,
                screen_to_frame,
                frame_to_screen: inverse(&screen_to_frame),
                lens: Lens {
                    distortion: lens.distortion * strength as f32,
                    ..lens
                },
            };
            let retained = fitted.safe_scale();
            (fitted, retained)
        };
        let minimum = 1.0 - crop_limit;
        let (full, retained) = candidate(1.0);
        let mut crop_scale = 1.0;
        if retained >= minimum {
            projection = full;
            crop_scale = retained;
        } else {
            let (mut low, mut high) = (0.0, 1.0);
            for _ in 0..12 {
                let strength = (low + high) * 0.5;
                let (fitted, retained) = candidate(strength);
                if retained >= minimum {
                    low = strength;
                    projection = fitted;
                    crop_scale = retained;
                } else {
                    high = strength;
                }
            }
        }
        let zoom = 1.0 / crop_scale;
        let source_zoom = [
            [zoom, 0.0, (1.0 - zoom) * sw as f64 * 0.5],
            [0.0, zoom, (1.0 - zoom) * sh as f64 * 0.5],
            [0.0, 0.0, 1.0],
        ];
        projection.screen_to_frame = matmul(&projection.screen_to_frame, &source_zoom);
        projection.frame_to_screen = inverse(&projection.screen_to_frame);
        projection
    }

    /// Bound the radial image of the output pixel-centre rectangle, then its
    /// inverse projection. With a positive lens and no projective pole, this
    /// enclosure contains every interior sample, not just the frame corners.
    fn safe_scale(&self) -> f64 {
        let (sw, sh) = self.screen;
        let (w, h) = self.frame;
        let (right, bottom) = (w as f64 - 0.5, h as f64 - 0.5);
        let cx = (self.lens.center.0 as f64 * w as f64).clamp(0.5, right);
        let cy = (self.lens.center.1 as f64 * h as f64).clamp(0.5, bottom);
        let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
        let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for x in [0.5, cx, right] {
            for y in [0.5, cy, bottom] {
                let (lx, ly) = lens_coords(x, y, w, h, self.lens);
                min_x = min_x.min(lx);
                max_x = max_x.max(lx);
                min_y = min_y.min(ly);
                max_y = max_y.max(ly);
            }
        }
        let mut retained = 1.0_f64;
        for x in [min_x, max_x] {
            for y in [min_y, max_y] {
                let m = &self.frame_to_screen;
                let denominator = m[2][0] * x + m[2][1] * y + m[2][2];
                if denominator <= 0.0 || !denominator.is_finite() {
                    return 0.0;
                }
                let (u, v) = apply_h(m, x, y);
                retained = retained
                    .min((sw as f64 - 1.0) * 0.5 / (u - sw as f64 * 0.5).abs())
                    .min((sh as f64 - 1.0) * 0.5 / (v - sh as f64 * 0.5).abs());
            }
        }
        if retained < 1.0 {
            retained * (1.0 - 1e-9)
        } else {
            1.0
        }
    }

    pub(crate) fn lens(&self) -> Lens {
        self.lens
    }

    pub(crate) fn screen_to_frame(&self) -> &Mat3 {
        &self.screen_to_frame
    }

    pub(crate) fn frame_to_screen(&self) -> &Mat3 {
        &self.frame_to_screen
    }

    pub(crate) fn auxiliary_matrix(&self, auxiliary: (usize, usize)) -> Mat3 {
        let (sw, sh) = self.screen;
        let (w, h) = self.frame;
        let (aw, ah) = auxiliary;
        matmul(
            &matmul(
                &scale_matrix(aw as f64 / w as f64, ah as f64 / h as f64),
                &self.screen_to_frame,
            ),
            &scale_matrix(sw as f64 / aw as f64, sh as f64 / ah as f64),
        )
    }
}

/// Shared inverse-warp sampler.
///
/// `sample` returns one source pixel in the caller's working colour space;
/// both RGB8 emission (LUT-decoded) and HDR float input use this same path.
fn warp<S>(sw: usize, sh: usize, m: &Mat3, w: usize, h: usize, lens: Lens, sample: S) -> Frame
where
    S: Fn(usize, usize) -> [f32; 3],
{
    let mut out = Frame::zeros(w, h);
    assert!(
        sw > 0 && sh > 0 && w > 0 && h > 0,
        "positive validated image dimensions"
    );
    let inv = inverse(m);
    let max_x = (sw - 1) as f64;
    let max_y = (sh - 1) as f64;

    for row in 0..h {
        for col in 0..w {
            let (lx, ly) = lens_coords(col as f64 + 0.5, row as f64 + 0.5, w, h, lens);
            let (mx, my) = apply_h(&inv, lx, ly);

            // Pixel-boundary mapping -> source pixel-index space.
            let u = (mx - 0.5).clamp(0.0, max_x);
            let v = (my - 0.5).clamp(0.0, max_y);
            let x0 = u.floor() as usize;
            let y0 = v.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let y1 = (y0 + 1).min(sh - 1);
            let fx = u - x0 as f64;
            let fy = v - y0 as f64;

            // Camera fitting keeps screen samples valid; clamping also serves
            // subpixel roundoff and the low-resolution reflection buffer.
            let c00 = sample(x0, y0);
            let c10 = sample(x1, y0);
            let c01 = sample(x0, y1);
            let c11 = sample(x1, y1);
            let w00 = (1.0 - fx) * (1.0 - fy);
            let w10 = fx * (1.0 - fy);
            let w01 = (1.0 - fx) * fy;
            let w11 = fx * fy;

            let mut pixel = [0.0f32; 3];
            for (k, value) in pixel.iter_mut().enumerate() {
                *value = (c00[k] as f64 * w00
                    + c10[k] as f64 * w10
                    + c01[k] as f64 * w01
                    + c11[k] as f64 * w11) as f32;
            }
            out.pixels_mut()[row * w + col] = pixel;
        }
    }
    out
}

/// Average source pixel areas in linear light without a full-size float copy.
fn prefilter_screen(src: &RgbImage, width: usize, height: usize) -> Frame {
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let lut = srgb_lut();
    let raw = src.as_raw();
    let mut out = Frame::zeros(width, height);
    let scale = DISPLAY_EMISSION as f64 / (sw * sh) as f64;
    for y in 0..height {
        let (top, bottom) = (y * sh, (y + 1) * sh);
        for x in 0..width {
            let (left, right) = (x * sw, (x + 1) * sw);
            let mut sum = [0.0_f64; 3];
            for sy in top / height..=(bottom - 1) / height {
                let wy = bottom.min((sy + 1) * height) - top.max(sy * height);
                for sx in left / width..=(right - 1) / width {
                    let wx = right.min((sx + 1) * width) - left.max(sx * width);
                    let weight = (wx * wy) as f64;
                    let offset = (sy * sw + sx) * 3;
                    for c in 0..3 {
                        sum[c] += lut[raw[offset + c] as usize] as f64 * weight;
                    }
                }
            }
            out.pixels_mut()[y * width + x] = sum.map(|value| (value * scale) as f32);
        }
    }
    out
}

/// Warp an RGB8 screenshot, prefiltering large reductions before perspective.
/// The bounded prefilter is linear-light; 1:1 sampling decodes neighbours lazily.
pub(crate) fn warp_screen(src: &RgbImage, m: &Mat3, w: usize, h: usize, lens: Lens) -> Frame {
    let sw = src.width() as usize;
    let sh = src.height() as usize;
    if sw > w || sh > h {
        // ponytail: axis-aligned prefilter; use footprint filtering if steeper poses need anisotropy.
        let filtered = prefilter_screen(src, sw.min(w), sh.min(h));
        let source_scale = scale_matrix(
            sw as f64 / filtered.width as f64,
            sh as f64 / filtered.height as f64,
        );
        return warp_edges(&filtered, &matmul(m, &source_scale), w, h, lens);
    }
    let lut = srgb_lut();
    let raw = src.as_raw();
    let stride = sw * 3;
    let sample = |x: usize, y: usize| -> [f32; 3] {
        let offset = y * stride + x * 3;
        [
            lut[raw[offset] as usize] * DISPLAY_EMISSION,
            lut[raw[offset + 1] as usize] * DISPLAY_EMISSION,
            lut[raw[offset + 2] as usize] * DISPLAY_EMISSION,
        ]
    };
    warp(sw, sh, m, w, h, lens, sample)
}

/// Warp an existing linear HDR frame through the same lens/homography sampler.
pub(crate) fn warp_edges(src: &Frame, m: &Mat3, w: usize, h: usize, lens: Lens) -> Frame {
    let sw = src.width;
    let sh = src.height;
    let pixels = src.pixels();
    let sample = |x: usize, y: usize| -> [f32; 3] { pixels[y * sw + x] };
    warp(sw, sh, m, w, h, lens, sample)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn approx(actual: f64, expected: f64, tolerance: f64) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} != {expected} (tolerance {tolerance})"
        );
    }

    fn assert_rgb(actual: [f32; 3], expected: [f32; 3], tolerance: f32) {
        for k in 0..3 {
            assert!(
                (actual[k] - expected[k]).abs() <= tolerance,
                "channel {k}: {actual:?} != {expected:?}"
            );
        }
    }

    #[test]
    fn full_frame_corners_keep_their_source_pixels() {
        let (sw, sh, w, h) = (4, 3, 8, 6);
        let quad = camera_quad(sw, sh, w, h, (0.0, 0.0, 0.0), 2.4);
        // Zero pose degenerates to the exact frame bounds.
        approx(quad[0][0], 0.0, 1e-9);
        approx(quad[0][1], 0.0, 1e-9);
        approx(quad[2][0], w as f64, 1e-9);
        approx(quad[2][1], h as f64, 1e-9);

        let matrix = screen_to_frame_matrix(sw, sh, &quad);
        let src_corners = [
            [0.0, 0.0],
            [sw as f64, 0.0],
            [sw as f64, sh as f64],
            [0.0, sh as f64],
        ];
        for (i, corner) in src_corners.iter().enumerate() {
            let (u, v) = apply_h(&matrix, corner[0], corner[1]);
            approx(u, quad[i][0], 1e-9);
            approx(v, quad[i][1], 1e-9);
        }

        let mut src = image::RgbImage::new(sw as u32, sh as u32);
        src.put_pixel(0, 0, Rgb([128, 64, 32]));
        src.put_pixel(3, 0, Rgb([0, 255, 0]));
        src.put_pixel(3, 2, Rgb([0, 0, 255]));
        src.put_pixel(0, 2, Rgb([255, 255, 0]));
        let lens = Lens {
            distortion: 0.0,
            center: (0.5, 0.5),
        };
        let frame = warp_screen(&src, &matrix, w, h, lens);
        let emission = crate::color::DISPLAY_EMISSION;
        let lut = crate::color::srgb_lut();
        assert_rgb(
            frame.pixels()[0],
            [lut[128] * emission, lut[64] * emission, lut[32] * emission],
            1e-6,
        );
        assert_rgb(frame.pixels()[w - 1], [0.0, emission, 0.0], 1e-6);
        assert_rgb(
            frame.pixels()[(h - 1) * w + w - 1],
            [0.0, 0.0, emission],
            1e-6,
        );
        assert_rgb(frame.pixels()[(h - 1) * w], [emission, emission, 0.0], 1e-6);
    }

    #[test]
    fn shrinking_screen_averages_periodic_detail_in_linear_light() {
        let lens = Lens {
            distortion: 0.0,
            center: (0.5, 0.5),
        };
        let lut = srgb_lut();
        let expected = [
            DISPLAY_EMISSION / 3.0,
            lut[128] * DISPLAY_EMISSION / 3.0,
            0.0,
        ];
        for (sw, sh, w, h) in [(99, 75, 33, 25), (99, 1, 33, 1), (1, 75, 1, 25)] {
            for vertical in [false, true] {
                if (vertical && sw == 1) || (!vertical && sh == 1) {
                    continue;
                }
                for phase in 0..3 {
                    let src = RgbImage::from_fn(sw, sh, |x, y| {
                        let position = if vertical { x } else { y };
                        Rgb(if position % 3 == phase {
                            [255, 128, 0]
                        } else {
                            [0; 3]
                        })
                    });
                    let m = scale_matrix(w as f64 / sw as f64, h as f64 / sh as f64);
                    let out = warp_screen(&src, &m, w as usize, h as usize, lens);
                    for &pixel in out.pixels() {
                        assert_rgb(pixel, expected, 1e-6);
                    }
                }
            }
        }
    }

    #[test]
    fn odd_screen_reduction_preserves_edge_pixel_energy() {
        let src = RgbImage::from_fn(17, 11, |x, y| {
            Rgb(if x == 16 || y == 10 { [255; 3] } else { [0; 3] })
        });
        let m = scale_matrix(7.0 / 17.0, 5.0 / 11.0);
        let out = warp_screen(
            &src,
            &m,
            7,
            5,
            Lens {
                distortion: 0.0,
                center: (0.5, 0.5),
            },
        );
        let mean = out
            .pixels()
            .iter()
            .map(|pixel| pixel[0] as f64)
            .sum::<f64>()
            / 35.0;
        let expected = DISPLAY_EMISSION as f64 * 27.0 / 187.0;
        approx(mean, expected, 1e-6);
    }

    #[test]
    fn fitted_camera_keeps_samples_valid_and_preserves_the_crop_budget() {
        for (sw, sh, w, h) in [
            (240, 160, 240, 160),
            (160, 240, 160, 240),
            (33, 241, 33, 241),
            (1000, 333, 320, 107),
            (3, 2, 3, 2),
            (1, 1, 1, 1),
            (1, 7, 1, 7),
            (7, 1, 7, 1),
            (2, 12_000_000, 1, 1920),
            (12_000_000, 2, 1920, 1),
        ] {
            for crop_limit in [0.0, 0.02, 0.10] {
                for (pose, distortion) in [
                    ((4.0, -3.0, 0.6), 0.0),
                    ((4.0, -3.0, 0.6), 0.035),
                    ((18.0, -12.0, 8.0), 0.0),
                    ((18.0, -12.0, 8.0), 0.3),
                ] {
                    let projection = Projection::camera(
                        (sw, sh),
                        (w, h),
                        pose,
                        2.4,
                        Lens {
                            distortion,
                            center: (0.4, 0.3),
                        },
                        crop_limit,
                    );
                    let lens = projection.lens();
                    if sw.min(sh).min(w).min(h) == 1 {
                        assert_eq!(lens.distortion, 0.0);
                    } else if crop_limit > 0.0 && distortion > 0.0 {
                        assert!(lens.distortion > 0.0, "nonzero lens effect was discarded");
                    }
                    for row in 0..h {
                        for col in 0..w {
                            let (lx, ly) =
                                lens_coords(col as f64 + 0.5, row as f64 + 0.5, w, h, lens);
                            let (u, v) = apply_h(projection.frame_to_screen(), lx, ly);
                            assert!(
                                u >= 0.5 - 1e-6
                                    && u <= sw as f64 - 0.5 + 1e-6
                                    && v >= 0.5 - 1e-6
                                    && v <= sh as f64 - 0.5 + 1e-6,
                                "edge fill at {col},{row} in {sw}x{sh} -> {w}x{h}: {u},{v}"
                            );
                        }
                    }
                    // The central (1 - crop_limit) rectangle must remain visible,
                    // even after inverting the radial lens on its four corners.
                    let margin = crop_limit * 0.5;
                    for x in [sw as f64 * margin, sw as f64 * (1.0 - margin)] {
                        for y in [sh as f64 * margin, sh as f64 * (1.0 - margin)] {
                            let (lx, ly) = apply_h(projection.screen_to_frame(), x, y);
                            let (cx, cy) = (
                                lens.center.0 as f64 * w as f64,
                                lens.center.1 as f64 * h as f64,
                            );
                            let (dx, dy) = (lx - cx, ly - cy);
                            let radial = f64::from(lens.distortion) * (dx * dx + dy * dy)
                                / (w.max(h) as f64 * 0.5).powi(2);
                            let (mut low, mut high) = (0.0, 1.0);
                            for _ in 0..48 {
                                let scale = (low + high) * 0.5;
                                if scale * (1.0 + radial * scale * scale) > 1.0 {
                                    high = scale;
                                } else {
                                    low = scale;
                                }
                            }
                            let scale = (low + high) * 0.5;
                            let (x, y) = (cx + dx * scale, cy + dy * scale);
                            assert!(
                                x >= -1e-6
                                    && x <= w as f64 + 1e-6
                                    && y >= -1e-6
                                    && y <= h as f64 + 1e-6,
                                "crop exceeds {crop_limit} in {sw}x{sh}: {x},{y}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn uniform_white_never_gains_black_edges() {
        let (sw, sh, w, h) = (5, 3, 32, 20);
        let quad = camera_quad(sw, sh, w, h, (7.0, -5.0, 2.0), 1.9);
        for [x, y] in quad {
            assert!((0.0..=w as f64).contains(&x));
            assert!((0.0..=h as f64).contains(&y));
        }
        let matrix = screen_to_frame_matrix(sw, sh, &quad);
        let src = Frame::new(sw, sh, vec![[1.0; 3]; sw * sh]);
        let lens = Lens {
            distortion: 0.3,
            center: (0.4, 0.6),
        };
        let frame = warp_edges(&src, &matrix, w, h, lens);
        for pixel in frame.pixels() {
            assert_rgb(*pixel, [1.0; 3], 1e-6);
        }
    }

    #[test]
    fn lens_coords_matches_the_analytic_radial_field() {
        let lens = Lens {
            distortion: 0.2,
            center: (0.5, 0.5),
        };
        let (cx, cy) = lens_coords(50.0, 25.0, 100, 50, lens);
        approx(cx, 50.0, 1e-6);
        approx(cy, 25.0, 1e-6);

        // half = 50, r^2 = 625, k = 1 + 0.2 * 625 / 2500 = 1.05.
        let (x, y) = lens_coords(30.0, 10.0, 100, 50, lens);
        approx(x, 29.0, 1e-5);
        approx(y, 9.25, 1e-5);
    }

    #[test]
    fn warp_edges_uses_the_same_radial_lens_map() {
        let (sw, sh, w, h) = (40, 20, 40, 20);
        let pixels = (0..sw * sh)
            .map(|i| {
                let value = (i % sw) as f32 / (sw - 1) as f32;
                [value; 3]
            })
            .collect();
        let src = Frame::new(sw, sh, pixels);
        let lens = Lens {
            distortion: 0.3,
            center: (0.5, 0.5),
        };
        let out = warp_edges(&src, &IDENTITY, w, h, lens);

        let (col, row) = (10, 5);
        let (lx, _) = lens_coords(col as f64 + 0.5, row as f64 + 0.5, w, h, lens);
        let expected = ((lx - 0.5) / (sw as f64 - 1.0)) as f32;
        let pixel = out.pixels()[row * w + col];
        assert!((pixel[0] - expected).abs() < 1e-5);
        assert!((pixel[1] - expected).abs() < 1e-5);
        assert!((pixel[2] - expected).abs() < 1e-5);
    }

    #[test]
    fn single_axis_and_odd_frames_stay_finite_and_uniform() {
        let lens = Lens {
            distortion: 0.3,
            center: (0.4, 0.6),
        };
        for (sw, sh, w, h) in [(1usize, 5usize, 1usize, 5usize), (7, 5, 7, 5), (1, 1, 3, 3)] {
            let src = Frame::new(sw, sh, vec![[1.0; 3]; sw * sh]);
            let out = warp_edges(&src, &IDENTITY, w, h, lens);
            assert_eq!((out.width, out.height), (w, h));
            for pixel in out.pixels() {
                assert!(pixel.iter().all(|c| c.is_finite()));
                assert_rgb(*pixel, [1.0; 3], 1e-6);
            }
        }
    }

    #[test]
    fn matrix_inverse_round_trips() {
        let m: Mat3 = [
            [1.03, 0.04, -0.6],
            [0.02, 0.97, 0.4],
            [1.1e-4, -0.8e-4, 1.0],
        ];
        let product = matmul(&m, &inverse(&m));
        for (row, values) in product.iter().enumerate() {
            for (col, value) in values.iter().enumerate() {
                let expected = if row == col { 1.0 } else { 0.0 };
                approx(*value, expected, 1e-12);
            }
        }
    }
}
