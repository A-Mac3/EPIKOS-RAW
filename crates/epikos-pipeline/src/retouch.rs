//! Retouching on the upright frame, before exposure, tone and the look (so a fill stays
//! right whatever is done to the photo afterwards):
//!
//! - **Generative erase**: fills made by the engine with an inpainting model (LaMa), on
//!   a crop around each erased object, blended in through a feathered mask.
//! - **Dust spots / healing**: each spot is replaced by the best-matching nearby patch
//!   (searched in eight directions around it), colour-matched to its surroundings and
//!   feathered, like a healing brush.
//! - **Red eye**: inside the detected eyes, strongly red pupils are neutralised and
//!   darkened.

use epikos_core::{resize_plane, ImageRgbF32};
use epikos_sidecar::{MaskTarget, Retouch, Spot};

use crate::look::{LookInputs, MaskPlane};

/// A fill for an erased area: a box of the upright frame (fractions), its pixels
/// (planar linear RGB) and a soft 0–1 mask of where to use them.
#[derive(Debug, Clone, Copy)]
pub struct FillPlane<'a> {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub width: u32,
    pub height: u32,
    pub rgb: [&'a [f32]; 3],
    pub mask: &'a [f32],
}

/// The soft mask of brush `strokes` (on the whole upright frame) at `w × h`.
pub fn stroke_mask(strokes: &[epikos_sidecar::BrushStroke], w: usize, h: usize) -> Vec<f32> {
    let shape = epikos_sidecar::ManualShape::Brush { strokes: strokes.to_vec() };
    crate::look::manual::mask_for(&shape, w, h)
}

/// A local adjustment's mask as its refinement leaves it (grown or shrunk, brushed,
/// feathered): what the edit actually covers.
pub fn refine_local_mask(mask: &mut Vec<f32>, adj: &epikos_sidecar::LocalAdjustment, w: usize, h: usize) {
    crate::look::manual::refine_mask(mask, adj, w, h);
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn apply_retouch(rgb: &mut ImageRgbF32, retouch: &Retouch, inputs: &LookInputs) {
    if retouch.is_empty() || rgb.width < 4 || rgb.height < 4 {
        return;
    }
    for fill in inputs.fills {
        apply_fill(rgb, fill);
    }
    for spot in &retouch.spots {
        heal(rgb, spot);
    }
    if retouch.red_eye {
        if let Some(eyes) = inputs.masks.iter().find(|m| m.target == MaskTarget::Eyes) {
            remove_red_eye(rgb, eyes);
        }
    }
}

fn apply_fill(rgb: &mut ImageRgbF32, f: &FillPlane) {
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    let px0 = ((f.x0 * w as f32).round().max(0.0) as usize).min(w - 1);
    let py0 = ((f.y0 * h as f32).round().max(0.0) as usize).min(h - 1);
    let px1 = ((f.x1 * w as f32).round() as usize).clamp(px0 + 1, w);
    let py1 = ((f.y1 * h as f32).round() as usize).clamp(py0 + 1, h);
    let (bw, bh) = ((px1 - px0) as u32, (py1 - py0) as u32);
    if f.mask.len() != (f.width * f.height) as usize {
        return;
    }
    let scale = |p: &[f32]| resize_plane(p, f.width, f.height, bw, bh);
    let mask = scale(f.mask);
    let planes = f.rgb.map(scale);
    for y in 0..bh as usize {
        for x in 0..bw as usize {
            let (i, j) = ((py0 + y) * w + px0 + x, y * bw as usize + x);
            let m = mask[j];
            if m <= 0.0 {
                continue;
            }
            rgb.r[i] += (planes[0][j] - rgb.r[i]) * m;
            rgb.g[i] += (planes[1][j] - rgb.g[i]) * m;
            rgb.b[i] += (planes[2][j] - rgb.b[i]) * m;
        }
    }
}

/// Healing: copy the best-matching patch from around the spot, matched in colour to
/// the spot's own surroundings, into a feathered circle.
fn heal(rgb: &mut ImageRgbF32, spot: &Spot) {
    let (w, h) = (rgb.width as i64, rgb.height as i64);
    let r = (spot.radius * w as f32).max(1.5);
    let (cx, cy) = (spot.x * w as f32, spot.y * h as f32);
    let ri = r.ceil() as i64 + 1;
    let ring = (r * 1.5).ceil() as i64;
    let lum = |i: usize| 0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i];
    let idx = |x: i64, y: i64| (y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize;
    // The spot's surroundings (an annulus) describe what the patch should look like.
    let ring_px: Vec<(i64, i64)> = (-ring..=ring)
        .flat_map(|dy| (-ring..=ring).map(move |dx| (dx, dy)))
        .filter(|&(dx, dy)| {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            d > r && d <= r * 1.5
        })
        .collect();
    if ring_px.is_empty() {
        return;
    }
    let (tx, ty) = (cx.round() as i64, cy.round() as i64);
    let mut best: Option<(f32, i64, i64)> = None;
    for k in 0..16 {
        let a = k as f32 * std::f32::consts::TAU / 16.0;
        for dist in [2.3f32, 3.2] {
            let (sx, sy) = (tx + (a.cos() * r * dist).round() as i64, ty + (a.sin() * r * dist).round() as i64);
            if sx - ring < 0 || sy - ring < 0 || sx + ring >= w || sy + ring >= h {
                continue;
            }
            let cost: f32 = ring_px
                .iter()
                .map(|&(dx, dy)| (lum(idx(tx + dx, ty + dy)) - lum(idx(sx + dx, sy + dy))).powi(2))
                .sum();
            if best.is_none_or(|b| cost < b.0) {
                best = Some((cost, sx, sy));
            }
        }
    }
    let Some((_, sx, sy)) = best else { return };
    // Colour match: shift the source by the difference between the two surroundings.
    let mean = |ox: i64, oy: i64, p: &[f32]| ring_px.iter().map(|&(dx, dy)| p[idx(ox + dx, oy + dy)]).sum::<f32>() / ring_px.len() as f32;
    let shift = [
        mean(tx, ty, &rgb.r) - mean(sx, sy, &rgb.r),
        mean(tx, ty, &rgb.g) - mean(sx, sy, &rgb.g),
        mean(tx, ty, &rgb.b) - mean(sx, sy, &rgb.b),
    ];
    let src: Vec<(usize, usize, f32)> = (-ri..=ri)
        .flat_map(|dy| (-ri..=ri).map(move |dx| (dx, dy)))
        .filter_map(|(dx, dy)| {
            let d = ((dx * dx + dy * dy) as f32).sqrt() / r;
            let m = 1.0 - smoothstep(0.65, 1.0, d);
            let (x, y) = (tx + dx, ty + dy);
            (m > 0.0 && x >= 0 && y >= 0 && x < w && y < h).then(|| ((y * w + x) as usize, idx(sx + dx, sy + dy), m))
        })
        .collect();
    let patch: Vec<[f32; 3]> = src.iter().map(|&(_, s, _)| [rgb.r[s], rgb.g[s], rgb.b[s]]).collect();
    for ((t, _, m), p) in src.into_iter().zip(patch) {
        rgb.r[t] += (p[0] + shift[0] - rgb.r[t]) * m;
        rgb.g[t] += (p[1] + shift[1] - rgb.g[t]) * m;
        rgb.b[t] += (p[2] + shift[2] - rgb.b[t]) * m;
    }
}

/// Red pupils inside the eye mask: red brought down to the green–blue level and a
/// little darker, so the pupil reads as dark and natural.
fn remove_red_eye(rgb: &mut ImageRgbF32, eyes: &MaskPlane) {
    let (w, h) = (rgb.width, rgb.height);
    if eyes.data.len() != (eyes.width * eyes.height) as usize {
        return;
    }
    let mask = resize_plane(eyes.data, eyes.width, eyes.height, w, h);
    for (i, m) in mask.into_iter().enumerate() {
        if m <= 0.05 {
            continue;
        }
        let (r, g, b) = (rgb.r[i], rgb.g[i], rgb.b[i]);
        let other = g.max(b);
        // How red: red well above both other channels.
        let redness = smoothstep(1.8, 2.6, r / other.max(1e-4)) * smoothstep(0.01, 0.04, r);
        let k = (m * redness).clamp(0.0, 1.0);
        if k > 0.0 {
            let target = 0.8 * (g + b) * 0.5;
            rgb.r[i] = r + (target - r) * k;
            rgb.g[i] = g * (1.0 - 0.2 * k);
            rgb.b[i] = b * (1.0 - 0.2 * k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    #[test]
    fn a_dust_spot_is_healed_from_its_surroundings() {
        let (w, h) = (80u32, 60u32);
        let mut img = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
        // A smooth sky gradient with a dark dust speck.
        for i in 0..img.len() {
            let y = (i as u32 / w) as f32 / h as f32;
            let v = 0.4 + 0.2 * y;
            (img.r[i], img.g[i], img.b[i]) = (v * 0.7, v * 0.85, v);
        }
        let speck = img.index(40, 30);
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                let i = img.index((40 + dx) as u32, (30 + dy) as u32);
                (img.r[i], img.g[i], img.b[i]) = (0.05, 0.05, 0.05);
            }
        }
        let expected = 0.4 + 0.2 * 0.5;
        heal(&mut img, &Spot { x: 0.5, y: 0.5, radius: 4.0 / w as f32 });
        assert!((img.b[speck] - expected).abs() < 0.03, "healed to {}", img.b[speck]);
    }

    #[test]
    fn red_pupils_turn_dark_and_other_colours_stay() {
        let mut img = ImageRgbF32::new(4, 1, ColorSpace::LinearRec2020);
        (img.r[0], img.g[0], img.b[0]) = (0.5, 0.05, 0.05); // red eye
        (img.r[1], img.g[1], img.b[1]) = (0.3, 0.2, 0.15); // iris, brown
        let data = [1.0f32; 4];
        let eyes = MaskPlane { target: MaskTarget::Eyes, width: 4, height: 1, data: &data };
        remove_red_eye(&mut img, &eyes);
        assert!(img.r[0] < 0.1, "red pupil {}", img.r[0]);
        assert!((img.r[1] - 0.3).abs() < 1e-3, "brown iris kept");
    }
}
