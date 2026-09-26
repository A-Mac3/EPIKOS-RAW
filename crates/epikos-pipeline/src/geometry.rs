//! Step 1 auto-geometry on the upright frame: straighten (rotation) and vertical
//! perspective (keystone), with the frame enlarged just enough to stay filled.
//!
//! [`estimate_upright`] measures both from the photo's own straight edges: near-vertical
//! edges tell the tilt (their mean lean) and the keystone (how their lean changes
//! across the frame); near-horizontal edges add to the tilt when there are few
//! verticals.

use epikos_core::ImageRgbF32;
use rayon::prelude::*;

/// Keystone strength at `vertical = ±100`: the top edge narrows or widens by this
/// fraction relative to the centre row.
const KEYSTONE_MAX: f64 = 0.3;

/// Apply rotation (degrees, positive = counter-clockwise) and vertical perspective
/// (−100…100) to an upright image.
pub fn apply_geometry(image: &ImageRgbF32, rotation: f32, vertical: f32) -> ImageRgbF32 {
    if (rotation == 0.0 && vertical == 0.0) || image.width < 4 || image.height < 4 {
        return image.clone();
    }
    let t = Transform::new(image.width as f64, image.height as f64, rotation as f64, vertical as f64);
    let mut out = ImageRgbF32::new(image.width, image.height, image.space);
    let width = image.width as usize;
    out.r
        .par_chunks_mut(width)
        .zip(out.g.par_chunks_mut(width))
        .zip(out.b.par_chunks_mut(width))
        .enumerate()
        .for_each(|(y, ((r, g), b))| {
            for x in 0..width {
                let (sx, sy) = t.source(x as f64, y as f64);
                let p = image.sample_bilinear(sx as f32, sy as f32);
                (r[x], g[x], b[x]) = (p.r, p.g, p.b);
            }
        });
    out
}

/// Output → source mapping for one frame size.
struct Transform {
    cx: f64,
    cy: f64,
    half_h: f64,
    cos: f64,
    sin: f64,
    keystone: f64,
    zoom: f64,
}

impl Transform {
    fn new(w: f64, h: f64, rotation: f64, vertical: f64) -> Self {
        let a = rotation.to_radians();
        let mut t = Transform {
            cx: (w - 1.0) / 2.0,
            cy: (h - 1.0) / 2.0,
            half_h: h / 2.0,
            cos: a.cos(),
            sin: a.sin(),
            keystone: (vertical / 100.0).clamp(-1.0, 1.0) * KEYSTONE_MAX,
            zoom: 1.0,
        };
        // Smallest enlargement that keeps every border pixel inside the source.
        let inside = |t: &Transform| {
            let steps = 24;
            (0..=steps).all(|i| {
                let f = i as f64 / steps as f64;
                [(f * (w - 1.0), 0.0), (f * (w - 1.0), h - 1.0), (0.0, f * (h - 1.0)), (w - 1.0, f * (h - 1.0))]
                    .into_iter()
                    .all(|(x, y)| {
                        let (sx, sy) = t.source(x, y);
                        (-0.5..=w - 0.5).contains(&sx) && (-0.5..=h - 0.5).contains(&sy)
                    })
            })
        };
        let (mut lo, mut hi) = (1.0, 4.0);
        t.zoom = hi;
        if inside(&t) {
            for _ in 0..30 {
                let mid = 0.5 * (lo + hi);
                t.zoom = mid;
                if inside(&t) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            t.zoom = hi;
        }
        t
    }

    /// Output pixel → source pixel.
    fn source(&self, x: f64, y: f64) -> (f64, f64) {
        let (u, v) = ((x - self.cx) / self.zoom, (y - self.cy) / self.zoom);
        // Undo a counter-clockwise rotation (y points down).
        let (px, py) = (u * self.cos - v * self.sin, u * self.sin + v * self.cos);
        // Keystone: rows above the centre sample closer to the middle when positive.
        let k = 1.0 + self.keystone * py / self.half_h;
        (self.cx + px * k, self.cy + py)
    }
}

/// Suggested `(rotation, vertical)` to level the horizon and straighten verticals.
/// `luma` is any lightness plane of the upright frame (a preview-sized one is plenty).
pub fn estimate_upright(luma: &[f32], w: usize, h: usize) -> (f32, f32) {
    if w < 16 || h < 16 || luma.len() != w * h {
        return (0.0, 0.0);
    }
    // Structure tensor: gradient products averaged over a window, so the orientation is
    // the edge's overall direction, not a pixel staircase's local 0°/45° steps.
    let mut gxx = vec![0.0f32; w * h];
    let mut gyy = vec![0.0f32; w * h];
    let mut gxy = vec![0.0f32; w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: isize, dy: isize| luma[(y as isize + dy) as usize * w + (x as isize + dx) as usize];
            let gx = (p(1, -1) + 2.0 * p(1, 0) + p(1, 1)) - (p(-1, -1) + 2.0 * p(-1, 0) + p(-1, 1));
            let gy = (p(-1, 1) + 2.0 * p(0, 1) + p(1, 1)) - (p(-1, -1) + 2.0 * p(0, -1) + p(1, -1));
            let i = y * w + x;
            (gxx[i], gyy[i], gxy[i]) = (gx * gx, gy * gy, gx * gy);
        }
    }
    let r = (w.max(h) / 40).max(6);
    let (jxx, jyy, jxy) = (
        crate::look::box_plane(&gxx, w, h, r),
        crate::look::box_plane(&gyy, w, h, r),
        crate::look::box_plane(&gxy, w, h, r),
    );
    let mut mags = Vec::new();
    let mut edges = Vec::new(); // (x, y, angle of the edge line, strength)
    for y in r..h.saturating_sub(r) {
        for x in r..w.saturating_sub(r) {
            let i = y * w + x;
            let (a, b, c) = (jxx[i], jyy[i], jxy[i]);
            let energy = a + b;
            // Coherence: 1 for a single clear orientation, 0 for texture or corners.
            let coherence = if energy > 0.0 { ((a - b).powi(2) + 4.0 * c * c).sqrt() / energy } else { 0.0 };
            if coherence < 0.7 {
                continue;
            }
            let gradient = 0.5 * (2.0 * c as f64).atan2((a - b) as f64);
            mags.push(energy);
            edges.push((x, y, gradient + std::f64::consts::FRAC_PI_2, energy));
        }
    }
    if mags.is_empty() {
        return (0.0, 0.0);
    }
    let mut sorted = mags.clone();
    let k = ((sorted.len() as f64 * 0.5) as usize).min(sorted.len() - 1);
    let threshold = *sorted.select_nth_unstable_by(k, f32::total_cmp).1;
    let limit = 10f64.to_radians();
    // Deviation of an edge from vertical / horizontal, folded into (−90°, 90°].
    let fold = |a: f64| {
        let mut a = a % std::f64::consts::PI;
        if a > std::f64::consts::FRAC_PI_2 {
            a -= std::f64::consts::PI;
        } else if a <= -std::f64::consts::FRAC_PI_2 {
            a += std::f64::consts::PI;
        }
        a
    };
    // Verticals: lean (radians, positive = top leans right) against x (−1…1).
    let (mut vs, mut hs) = (Vec::new(), Vec::new());
    for &(x, y, a, m) in &edges {
        if m < threshold.max(1e-4) {
            continue;
        }
        let from_vertical = fold(a - std::f64::consts::FRAC_PI_2);
        let from_horizontal = fold(a);
        let xn = (x as f64 - w as f64 / 2.0) / (w as f64 / 2.0);
        let yn = (y as f64 - h as f64 / 2.0) / (h as f64 / 2.0);
        if from_vertical.abs() < limit {
            vs.push((xn, from_vertical, m as f64));
        } else if from_horizontal.abs() < limit {
            hs.push((yn, from_horizontal, m as f64));
        }
    }
    let enough = (w * h / 400).max(40);
    // Weighted least squares: lean = tilt + slope · x.
    let fit = |pts: &[(f64, f64, f64)]| {
        let sw: f64 = pts.iter().map(|p| p.2).sum();
        let mx = pts.iter().map(|p| p.2 * p.0).sum::<f64>() / sw;
        let my = pts.iter().map(|p| p.2 * p.1).sum::<f64>() / sw;
        let sxx: f64 = pts.iter().map(|p| p.2 * (p.0 - mx).powi(2)).sum();
        let sxy: f64 = pts.iter().map(|p| p.2 * (p.0 - mx) * (p.1 - my)).sum();
        let slope = if sxx > 1e-9 { sxy / sxx } else { 0.0 };
        (my - slope * mx, slope)
    };
    let (mut tilt, mut vertical) = (None, 0.0);
    if vs.len() >= enough {
        let (t, slope) = fit(&vs);
        tilt = Some(t);
        // Converging verticals (camera tilted up): a line at x0 runs along
        // x = cx + (x0 − cx)(1 + k·y/half-height), so its lean is −xn·k·aspect.
        let aspect = w as f64 / h as f64;
        let k = (-slope / aspect).clamp(-KEYSTONE_MAX, KEYSTONE_MAX);
        // Small keystones are usually intentional or noise: leave them.
        if k.abs() > 0.01 {
            vertical = k / KEYSTONE_MAX * 100.0;
        }
    }
    if tilt.is_none() && hs.len() >= enough {
        tilt = Some(fit(&hs).0);
    }
    // A lean with the top to the right needs a counter-clockwise turn.
    let rotation = tilt.map_or(0.0, |t| t.to_degrees());
    ((rotation as f32 * 100.0).round() / 100.0, (vertical as f32).round())
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::{ColorSpace, Pixel};

    /// Stripes that are vertical after the image is rotated by `deg` (counter-clockwise),
    /// anti-aliased (4×4 supersampling) as a lens would render them.
    fn tilted_stripes(w: usize, h: usize, deg: f64) -> Vec<f32> {
        let a = deg.to_radians();
        (0..w * h)
            .map(|i| {
                let mut sum = 0.0;
                for s in 0..16 {
                    let x = (i % w) as f64 + (s % 4) as f64 / 4.0 - w as f64 / 2.0;
                    let y = (i / w) as f64 + (s / 4) as f64 / 4.0 - h as f64 / 2.0;
                    // Coordinate across the stripes in the tilted frame.
                    let u = x * a.cos() - y * a.sin();
                    sum += if (u / 12.0).floor().rem_euclid(2.0) == 0.0 { 0.8 } else { 0.2 };
                }
                sum / 16.0
            })
            .collect()
    }

    #[test]
    fn zero_geometry_is_identity() {
        let mut img = ImageRgbF32::new(8, 6, ColorSpace::LinearRec2020);
        img.set(3, 2, Pixel::new(1.0, 0.5, 0.2));
        let out = apply_geometry(&img, 0.0, 0.0);
        assert_eq!(out.r, img.r);
    }

    #[test]
    fn rotation_keeps_the_frame_filled() {
        let mut img = ImageRgbF32::new(60, 40, ColorSpace::LinearRec2020);
        img.r.fill(1.0);
        let out = apply_geometry(&img, 7.0, 25.0);
        // Sampling stayed inside the source, so no clamped-edge smear is needed to fill.
        let t = Transform::new(60.0, 40.0, 7.0, 25.0);
        assert!(t.zoom > 1.0 && t.zoom < 1.6, "{}", t.zoom);
        assert!(out.r.iter().all(|&v| (v - 1.0).abs() < 1e-6));
    }

    #[test]
    fn estimates_the_tilt_of_leaning_verticals() {
        let (w, h) = (240, 180);
        // Stripes whose edges lean by 3°.
        let luma = tilted_stripes(w, h, 3.0);
        let (rotation, vertical) = estimate_upright(&luma, w, h);
        assert!((rotation.abs() - 3.0).abs() < 0.6, "rotation {rotation}");
        assert!(vertical.abs() < 10.0, "vertical {vertical}");
        // Applying the suggestion straightens the stripes: re-estimating finds ~0.
        let mut img = ImageRgbF32::new(w as u32, h as u32, ColorSpace::LinearRec2020);
        img.r.copy_from_slice(&luma);
        img.g.copy_from_slice(&luma);
        img.b.copy_from_slice(&luma);
        let fixed = apply_geometry(&img, rotation, 0.0);
        let (again, _) = estimate_upright(&fixed.g, w, h);
        assert!(again.abs() < 0.6, "after correction {again}");
    }

    #[test]
    fn straightens_verticals_that_converge_towards_the_top() {
        let (w, h) = (240, 240);
        let k = 0.12;
        // A camera tilted up: vertical lines at x0 appear at cx + (x0 − cx)(1 + k·y/half).
        let luma: Vec<f32> = (0..w * h)
            .map(|i| {
                let mut sum = 0.0;
                for s in 0..16 {
                    let x = (i % w) as f64 + (s % 4) as f64 / 4.0 - w as f64 / 2.0;
                    let y = (i / w) as f64 + (s / 4) as f64 / 4.0 - h as f64 / 2.0;
                    let x0 = x / (1.0 + k * y / (h as f64 / 2.0));
                    sum += if (x0 / 12.0).floor().rem_euclid(2.0) == 0.0 { 0.8 } else { 0.2 };
                }
                sum / 16.0
            })
            .collect();
        let (rotation, vertical) = estimate_upright(&luma, w, h);
        assert!(vertical > 15.0, "vertical {vertical}");
        assert!(rotation.abs() < 0.5, "rotation {rotation}");
        let mut img = ImageRgbF32::new(w as u32, h as u32, ColorSpace::LinearRec2020);
        img.g.copy_from_slice(&luma);
        let fixed = apply_geometry(&img, 0.0, vertical);
        let (_, again) = estimate_upright(&fixed.g, w, h);
        assert!(again.abs() < vertical.abs() * 0.4, "{vertical} → {again}");
    }

    #[test]
    fn flat_images_need_nothing() {
        assert_eq!(estimate_upright(&vec![0.5; 100 * 80], 100, 80), (0.0, 0.0));
    }

    #[test]
    fn the_warp_is_the_same_at_any_resolution() {
        // A mask made small and warped must line up with the full-size image warped the
        // same way: compare a 400 px and a 100 px warp of the same pattern.
        let pattern = |w: u32, h: u32| {
            let mut img = ImageRgbF32::new(w, h, epikos_core::ColorSpace::LinearRec2020);
            for i in 0..img.len() {
                let (x, y) = ((i as u32 % w) as f32 / w as f32, (i as u32 / w) as f32 / h as f32);
                let v = if ((x - 0.35).hypot(y - 0.55)) < 0.2 { 1.0 } else { 0.0 };
                (img.r[i], img.g[i], img.b[i]) = (v, v, v);
            }
            img
        };
        let (big, small) = (apply_geometry(&pattern(400, 300), 4.0, 25.0), apply_geometry(&pattern(100, 75), 4.0, 25.0));
        let (mut diff, mut n) = (0.0f32, 0);
        for y in 0..75 {
            for x in 0..100 {
                // The big warp sampled at the small one's pixel centres.
                let b = big.r[big.index(x * 4 + 2, y * 4 + 2)];
                diff += (b - small.r[small.index(x, y)]).abs();
                n += 1;
            }
        }
        assert!(diff / (n as f32) < 0.03, "mean difference {}", diff / n as f32);
    }
}
