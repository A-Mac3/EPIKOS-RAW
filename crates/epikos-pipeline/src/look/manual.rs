//! PRD Step 3 hand-drawn masks: brush strokes, linear and radial gradients, each with
//! its own local edit (exposure, contrast, saturation, warmth, tint, clarity and
//! dehaze), alongside the AI masks.
//!
//! Masks are soft by nature, so they're drawn at up to [`RASTER_SIDE`] px and scaled
//! to the image: the preview and the full-size export see the same shape.

use epikos_core::{resize_plane, ImageRgbF32};
use epikos_sidecar::{BrushStroke, LocalAdjustment, ManualAdjustment, ManualShape};
use rayon::prelude::*;

use super::local::apply_local;
use crate::dehaze::apply_dehaze;

const RASTER_SIDE: usize = 1024;

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The mask (0–1) of `shape` at `w × h`.
pub(crate) fn rasterize(shape: &ManualShape, w: usize, h: usize) -> Vec<f32> {
    let (wf, hf) = (w as f32, h as f32);
    match shape {
        ManualShape::Linear { x0, y0, x1, y1 } => {
            let (ax, ay) = (x0 * wf, y0 * hf);
            let (dx, dy) = (x1 * wf - ax, y1 * hf - ay);
            let len2 = dx * dx + dy * dy;
            if len2 < 1.0 {
                return vec![0.0; w * h];
            }
            (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let (px, py) = ((i % w) as f32 + 0.5 - ax, (i / w) as f32 + 0.5 - ay);
                    1.0 - smoothstep(0.0, 1.0, (px * dx + py * dy) / len2)
                })
                .collect()
        }
        ManualShape::Radial { cx, cy, rx, ry, angle, feather, invert } => {
            // Into the ellipse's own axes: rotate by −angle.
            let (sin, cos) = (-angle.to_radians()).sin_cos();
            let (rxp, ryp) = ((rx * wf).max(1.0), (ry * hf).max(1.0));
            let inner = (1.0 - feather / 100.0).clamp(0.0, 0.99);
            (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let (dx, dy) = ((i % w) as f32 + 0.5 - cx * wf, (i / w) as f32 + 0.5 - cy * hf);
                    let (u, v) = ((dx * cos - dy * sin) / rxp, (dx * sin + dy * cos) / ryp);
                    let m = 1.0 - smoothstep(inner, 1.0, u.hypot(v));
                    if *invert { 1.0 - m } else { m }
                })
                .collect()
        }
        ManualShape::Brush { strokes } => {
            let mut mask = vec![0.0f32; w * h];
            for stroke in strokes {
                let s = stamp_stroke(stroke, w, h);
                for (m, v) in mask.iter_mut().zip(s) {
                    *m = if stroke.erase { *m * (1.0 - v) } else { *m + v * (1.0 - *m) };
                }
            }
            mask
        }
    }
}

/// One stroke's coverage: round dabs along its path, soft by `feather`, at `flow`.
fn stamp_stroke(stroke: &BrushStroke, w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; w * h];
    let r = (stroke.size * w as f32 / 2.0).max(0.5);
    let hard = (1.0 - stroke.feather / 100.0).clamp(0.0, 0.99);
    let flow = (stroke.flow / 100.0).clamp(0.0, 1.0);
    let pts: Vec<(f32, f32)> = stroke.points.iter().map(|[x, y]| (x * w as f32, y * h as f32)).collect();
    let mut dab = |cx: f32, cy: f32| {
        let (x0, x1) = (((cx - r).floor().max(0.0)) as usize, ((cx + r).ceil() as usize).min(w));
        let (y0, y1) = (((cy - r).floor().max(0.0)) as usize, ((cy + r).ceil() as usize).min(h));
        for y in y0..y1 {
            for x in x0..x1 {
                let q = ((x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy)) / r;
                if q < 1.0 {
                    let v = flow * (1.0 - smoothstep(hard, 1.0, q));
                    let o = &mut out[y * w + x];
                    *o = o.max(v);
                }
            }
        }
    };
    let step = (r * 0.25).max(0.5);
    match pts.as_slice() {
        [] => {}
        [p] => dab(p.0, p.1),
        _ => {
            for pair in pts.windows(2) {
                let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
                let n = ((bx - ax).hypot(by - ay) / step).ceil().max(1.0) as usize;
                for k in 0..=n {
                    let t = k as f32 / n as f32;
                    dab(ax + (bx - ax) * t, ay + (by - ay) * t);
                }
            }
        }
    }
    out
}

/// The mask of `shape` at the image's size.
pub(crate) fn mask_for(shape: &ManualShape, w: usize, h: usize) -> Vec<f32> {
    let scale = (RASTER_SIDE as f32 / w.max(h) as f32).min(1.0);
    let (sw, sh) = (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1));
    let small = rasterize(shape, sw, sh);
    if (sw, sh) == (w, h) {
        small
    } else {
        resize_plane(&small, sw as u32, sh as u32, w as u32, h as u32)
    }
}

pub(crate) fn apply_manual(rgb: &mut ImageRgbF32, m: &ManualAdjustment) {
    if m.is_neutral() || rgb.width < 2 || rgb.height < 2 {
        return;
    }
    let mask = mask_for(&m.shape, rgb.width as usize, rgb.height as usize);
    if m.dehaze != 0.0 {
        let mut clear = rgb.clone();
        apply_dehaze(&mut clear, m.dehaze);
        for (dst, src) in [(&mut rgb.r, &clear.r), (&mut rgb.g, &clear.g), (&mut rgb.b, &clear.b)] {
            dst.par_iter_mut().zip(src.par_iter()).zip(mask.par_iter()).for_each(|((d, s), k)| *d += (s - *d) * k);
        }
    }
    let local = LocalAdjustment {
        exposure: m.exposure,
        contrast: m.contrast,
        saturation: m.saturation,
        warmth: m.warmth,
        tint: m.tint,
        clarity: m.clarity,
        highlights: m.highlights,
        shadows: m.shadows,
        whites: m.whites,
        blacks: m.blacks,
        ..Default::default()
    };
    apply_local(rgb, &local, &mask);
}

/// Refine a local adjustment's AI mask in place: extend or shrink it (`grow`,
/// −100…100, up to 6 % of the frame), add or remove hand-brushed strokes, then soften
/// its edge (`feather`, 0…100, up to 3 % of the frame) so the edit fades in instead of
/// ending on a visible line.
pub(crate) fn refine_mask(mask: &mut Vec<f32>, adj: &LocalAdjustment, w: usize, h: usize) {
    if !adj.is_refined() || mask.len() != w * h {
        return;
    }
    let long = w.max(h) as f32;
    let g = (adj.grow / 100.0).clamp(-1.0, 1.0);
    if g != 0.0 {
        let r = ((g.abs() * 0.06 * long).round() as usize).max(1);
        let blurred = super::blur::smooth(mask, w, h, r, 2);
        // A lower threshold on the blurred mask reaches further out; a higher one pulls
        // the edge in. The ramp keeps the edge soft.
        let t = 0.5 - 0.42 * g;
        *mask = blurred.into_iter().map(|v| smoothstep(t - 0.12, t + 0.12, v)).collect();
    }
    let (add, erase): (Vec<_>, Vec<_>) = adj.refine.iter().cloned().partition(|s| !s.erase);
    if !add.is_empty() {
        let a = mask_for(&ManualShape::Brush { strokes: add }, w, h);
        mask.iter_mut().zip(a).for_each(|(m, a)| *m = m.max(a));
    }
    if !erase.is_empty() {
        let strokes = erase.into_iter().map(|s| BrushStroke { erase: false, ..s }).collect();
        let e = mask_for(&ManualShape::Brush { strokes }, w, h);
        mask.iter_mut().zip(e).for_each(|(m, e)| *m *= 1.0 - e);
    }
    let f = (adj.feather / 100.0).clamp(0.0, 1.0);
    if f > 0.0 {
        let r = ((f * 0.03 * long).round() as usize).max(1);
        *mask = super::blur::smooth(mask, w, h, r, 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    #[test]
    fn gradients_and_brush_cover_what_they_should() {
        let (w, h) = (100, 80);
        // Sky gradient: full at the top, gone by 40% down.
        let lin = rasterize(&ManualShape::Linear { x0: 0.5, y0: 0.0, x1: 0.5, y1: 0.4 }, w, h);
        assert!(lin[w * 2 + 50] > 0.95 && lin[w * 60 + 50] < 0.01);
        // Radial: inside the ellipse, not outside; inverted the other way round.
        let shape = |invert| ManualShape::Radial { cx: 0.5, cy: 0.5, rx: 0.2, ry: 0.1, angle: 0.0, feather: 30.0, invert };
        let rad = rasterize(&shape(false), w, h);
        assert!(rad[40 * w + 50] > 0.99 && rad[40 * w + 90] < 0.01 && rad[20 * w + 50] < 0.01);
        let inv = rasterize(&shape(true), w, h);
        assert!(inv[40 * w + 50] < 0.01 && inv[40 * w + 90] > 0.99);
        // Brush: a stroke paints its path; an erase stroke over half of it removes it.
        let stroke = BrushStroke { points: vec![[0.1, 0.5], [0.9, 0.5]], size: 0.1, feather: 20.0, flow: 100.0, erase: false };
        let erase = BrushStroke { points: vec![[0.5, 0.5], [0.95, 0.5]], erase: true, ..stroke.clone() };
        let b = rasterize(&ManualShape::Brush { strokes: vec![stroke, erase] }, w, h);
        assert!(b[40 * w + 20] > 0.99, "painted {}", b[40 * w + 20]);
        assert!(b[40 * w + 80] < 0.01, "erased {}", b[40 * w + 80]);
        assert!(b[10 * w + 20] < 0.01, "off the path");
    }

    #[test]
    fn refinement_grows_shrinks_brushes_and_feathers() {
        let (w, h) = (100, 100);
        // A hard square in the middle.
        let square: Vec<f32> = (0..w * h).map(|i| ((30..70).contains(&(i % w)) && (30..70).contains(&(i / w))) as u8 as f32).collect();
        let area = |m: &[f32]| m.iter().sum::<f32>();
        let refined = |adj: LocalAdjustment| {
            let mut m = square.clone();
            refine_mask(&mut m, &adj, w, h);
            m
        };
        let grown = refined(LocalAdjustment { grow: 80.0, ..Default::default() });
        let shrunk = refined(LocalAdjustment { grow: -80.0, ..Default::default() });
        assert!(area(&grown) > area(&square) * 1.05, "grown {} vs {}", area(&grown), area(&square));
        assert!(area(&shrunk) < area(&square) * 0.95, "shrunk {}", area(&shrunk));
        // Brushing: add a patch outside, erase one inside.
        let brushed = refined(LocalAdjustment {
            refine: vec![
                BrushStroke { points: vec![[0.1, 0.1]], size: 0.08, feather: 0.0, flow: 100.0, erase: false },
                BrushStroke { points: vec![[0.5, 0.5]], size: 0.08, feather: 0.0, flow: 100.0, erase: true },
            ],
            ..Default::default()
        });
        assert!(brushed[10 * w + 10] > 0.9 && brushed[50 * w + 50] < 0.1);
        // Feathering: the edge becomes a ramp, not a step.
        let soft = refined(LocalAdjustment { feather: 60.0, ..Default::default() });
        let edge = soft[50 * w + 30];
        assert!(edge > 0.2 && edge < 0.8, "edge {edge}");
    }

    #[test]
    fn a_graduated_filter_darkens_only_its_side() {
        let mut img = ImageRgbF32::new(64, 64, ColorSpace::LinearRec2020);
        img.r.fill(0.4);
        img.g.fill(0.4);
        img.b.fill(0.4);
        let m = ManualAdjustment {
            shape: ManualShape::Linear { x0: 0.5, y0: 0.0, x1: 0.5, y1: 0.5 },
            exposure: -1.0,
            ..Default::default()
        };
        apply_manual(&mut img, &m);
        assert!((img.g[img.index(32, 1)] - 0.2).abs() < 0.02, "top {}", img.g[img.index(32, 1)]);
        assert!((img.g[img.index(32, 60)] - 0.4).abs() < 1e-4, "bottom untouched");
    }
}
