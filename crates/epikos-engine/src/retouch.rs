//! Retouching the engine prepares for the pipeline: generative-erase fills (LaMa
//! inpainting on a crop around each erased object) and dust-spot detection.

use std::sync::{Arc, PoisonError};

use epikos_core::{resize_plane, ImageRgbF32, Result};
use epikos_masks::RgbImage;
use epikos_pipeline::{develop_upright, stroke_mask, LookInputs};
use epikos_sidecar::{Adjustments, EraseArea, Spot};

use crate::analysis::box_mean;
use crate::{Engine, Loaded};

/// Long side of the upright frame the fills and the dust search are made on.
const FILL_SIDE: u32 = 2048;
const DUST_SIDE: u32 = 1600;
const LAMA: usize = 512;
/// Fills kept per photo: the current erase areas and a few undone ones.
const KEEP: usize = 12;

/// A fill for one erased area: a box of the upright frame (fractions) and its pixels.
pub(crate) struct FillData {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub side: u32,
    pub rgb: [Vec<f32>; 3],
    pub mask: Vec<f32>,
}

fn fill_key(area: &EraseArea, a: &Adjustments) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}",
        area.strokes, a.white_balance, a.lens, a.noise_reduction, a.highlight_recovery
    )
}

fn encode(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

fn decode(v: u8) -> f32 {
    let s = v as f32 / 255.0;
    if s <= 0.040_45 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

/// The upright develop (before exposure and tone) of the photo at up to `side` px.
fn upright(loaded: &Loaded, adjustments: &Adjustments, side: u32) -> Result<ImageRgbF32> {
    let base = loaded.base(side, side);
    let lens = loaded.lens_profile();
    let inputs = LookInputs { lens: lens.as_deref(), ..Default::default() };
    develop_upright((*base).clone(), &loaded.raw.profile, adjustments, &inputs)
}

/// Pixel box (x0, y0, side) of the square crop around an erased area, with enough of
/// its surroundings for the model to continue.
fn crop_box(area: &EraseArea, w: usize, h: usize) -> Option<(usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for s in &area.strokes {
        if s.erase {
            continue;
        }
        let r = s.size * w as f32 / 2.0;
        for p in &s.points {
            let (x, y) = (p[0] * w as f32, p[1] * h as f32);
            (x0, y0, x1, y1) = (x0.min(x - r), y0.min(y - r), x1.max(x + r), y1.max(y + r));
        }
    }
    if x0 > x1 {
        return None;
    }
    let side = ((x1 - x0).max(y1 - y0) * 1.8 + 48.0).min(w.min(h) as f32) as usize;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let bx = (cx - side as f32 / 2.0).clamp(0.0, (w - side) as f32) as usize;
    let by = (cy - side as f32 / 2.0).clamp(0.0, (h - side) as f32) as usize;
    Some((bx, by, side))
}

fn crop(plane: &[f32], w: usize, (x0, y0, side): (usize, usize, usize)) -> Vec<f32> {
    (y0..y0 + side).flat_map(|y| plane[y * w + x0..y * w + x0 + side].iter().copied()).collect()
}

impl Engine {
    /// Generative-erase fills for every erased area, made once and cached.
    pub(crate) fn erase_fills(&self, loaded: &Loaded, adjustments: &Adjustments) -> Result<Vec<Arc<FillData>>> {
        let areas = &adjustments.retouch.erase;
        if areas.is_empty() || !self.masker.inpaint_available() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = areas.iter().map(|a| fill_key(a, adjustments)).collect();
        let mut image: Option<ImageRgbF32> = None;
        let mut out = Vec::new();
        for (area, key) in areas.iter().zip(&keys) {
            let cached = {
                let fills = loaded.fills.lock().unwrap_or_else(PoisonError::into_inner);
                fills.iter().find(|(k, _)| k == key).map(|(_, f)| f.clone())
            };
            let fill = match cached {
                Some(f) => f,
                None => {
                    if image.is_none() {
                        image = Some(upright(loaded, adjustments, FILL_SIDE)?);
                    }
                    let Some(fill) = self.make_fill(image.as_ref().unwrap(), area)? else { continue };
                    let fill = Arc::new(fill);
                    let mut fills = loaded.fills.lock().unwrap_or_else(PoisonError::into_inner);
                    fills.insert(0, (key.clone(), fill.clone()));
                    fills.truncate(KEEP.max(areas.len()));
                    fill
                }
            };
            out.push(fill);
        }
        Ok(out)
    }

    fn make_fill(&self, img: &ImageRgbF32, area: &EraseArea) -> Result<Option<FillData>> {
        let (w, h) = (img.width as usize, img.height as usize);
        let Some(bx) = crop_box(area, w, h) else { return Ok(None) };
        let side = bx.2;
        if side < 8 {
            return Ok(None);
        }
        let hole_full = stroke_mask(&area.strokes, w, h);
        let to_model = |p: &[f32]| resize_plane(&crop(p, w, bx), side as u32, side as u32, LAMA as u32, LAMA as u32);
        let planes = [to_model(&img.r), to_model(&img.g), to_model(&img.b)];
        // The hole, a little larger than painted: the model must not see the object's rim.
        let hole = to_model(&hole_full);
        let grown = box_mean(&hole.iter().map(|&v| if v > 0.05 { 1.0 } else { 0.0 }).collect::<Vec<_>>(), LAMA, LAMA, 5);
        let hole: Vec<f32> = grown.iter().map(|&v| if v > 0.0 { 1.0 } else { 0.0 }).collect();

        // Scale the crop so its bright end sits near 1 (the image is before exposure).
        let mut peaks: Vec<f32> = (0..LAMA * LAMA)
            .filter(|&i| hole[i] == 0.0)
            .map(|i| planes[0][i].max(planes[1][i]).max(planes[2][i]))
            .collect();
        let scale = if peaks.is_empty() {
            1.0
        } else {
            let k = (peaks.len() - 1) * 995 / 1000;
            let (_, p, _) = peaks.select_nth_unstable_by(k, f32::total_cmp);
            (1.0 / p.max(1e-4)).clamp(0.25, 16.0)
        };
        let mut data = Vec::with_capacity(LAMA * LAMA * 3);
        for i in 0..LAMA * LAMA {
            data.extend(planes.iter().map(|p| encode(p[i] * scale)));
        }
        let filled = self.masker.inpaint(&RgbImage { width: LAMA as u32, height: LAMA as u32, data }, &hole)?;
        let rgb: [Vec<f32>; 3] =
            std::array::from_fn(|c| (0..LAMA * LAMA).map(|i| decode(filled.data[i * 3 + c]) / scale).collect());
        // Blend softly just past the hole's edge.
        let mask = box_mean(&box_mean(&grown.iter().map(|&v| if v > 0.0 { 1.0 } else { 0.0 }).collect::<Vec<_>>(), LAMA, LAMA, 3), LAMA, LAMA, 3);
        let (x0, y0, s) = (bx.0 as f32, bx.1 as f32, side as f32);
        Ok(Some(FillData {
            x0: x0 / w as f32,
            y0: y0 / h as f32,
            x1: (x0 + s) / w as f32,
            y1: (y0 + s) / h as f32,
            side: LAMA as u32,
            rgb,
            mask,
        }))
    }

    /// Dust spots on the photo: small, round, darker specks on smooth areas (skies,
    /// walls, studio backdrops), strongest first.
    pub fn find_dust_spots(&self, path: &std::path::Path, adjustments: &Adjustments) -> Result<Vec<Spot>> {
        let loaded = self.load(path)?;
        let img = upright(&loaded, adjustments, DUST_SIDE)?;
        Ok(dust_spots(&img))
    }
}

pub(crate) fn dust_spots(img: &ImageRgbF32) -> Vec<Spot> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w < 64 || h < 64 {
        return Vec::new();
    }
    let lum: Vec<f32> = (0..w * h).map(|i| 0.2627 * img.r[i] + 0.678 * img.g[i] + 0.0593 * img.b[i]).collect();
    let near = box_mean(&lum, w, h, 1);
    let around = box_mean(&lum, w, h, 14);
    let sq: Vec<f32> = lum.iter().map(|v| v * v).collect();
    let around_sq = box_mean(&sq, w, h, 14);
    // Per channel, for the colour check: dust dims every channel alike.
    let chans = [&img.r, &img.g, &img.b];
    let near_c = chans.map(|p| box_mean(p, w, h, 1));
    let around_c = chans.map(|p| box_mean(p, w, h, 14));
    let hue_shift = |i: usize| -> f32 {
        let (sn, sa) = (near_c.iter().map(|p| p[i]).sum::<f32>().max(1e-5), around_c.iter().map(|p| p[i]).sum::<f32>().max(1e-5));
        (0..3).map(|c| (near_c[c][i] / sn - around_c[c][i] / sa).abs()).sum()
    };
    // A candidate pixel: noticeably darker than a smooth neighbourhood.
    let cand: Vec<bool> = (0..w * h)
        .map(|i| {
            let m = around[i].max(1e-4);
            let std = (around_sq[i] - m * m).max(0.0).sqrt();
            let dip = (m - near[i]) / m;
            m > 0.02 && dip > 0.04 && std / m < 0.06 && dip * m > 1.5 * std
        })
        .collect();
    let mut seen = vec![false; w * h];
    let mut spots: Vec<(f32, Spot)> = Vec::new();
    let mut blobs: Vec<(f32, f32, f32, f32, bool)> = Vec::new();
    let border = 8;
    for start in 0..w * h {
        if !cand[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let (mut n, mut sx, mut sy, mut x0, mut y0, mut x1, mut y1, mut depth) =
            (0usize, 0.0f32, 0.0f32, usize::MAX, usize::MAX, 0usize, 0usize, 0.0f32);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            n += 1;
            (sx, sy) = (sx + x as f32, sy + y as f32);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            depth = depth.max((around[i] - near[i]) / around[i].max(1e-4));
            if n > 600 {
                continue;
            }
            for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if cand[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        let (cx, cy) = (sx / n as f32, sy / n as f32);
        let r = bw.max(bh) / 2.0;
        let centre = (cy as usize).min(h - 1) * w + (cx as usize).min(w - 1);
        // Sensor dust is faint and soft; dark, crisp marks are detail (text, rivets).
        let dusty = (3..=450).contains(&n)
            && bw.min(bh) / bw.max(bh) >= 0.5
            && n as f32 / (bw * bh) >= 0.45
            && depth < 0.4
            && hue_shift(centre) < 0.04
            && cx >= border as f32
            && cy >= border as f32
            && cx <= (w - border) as f32
            && cy <= (h - border) as f32;
        blobs.push((cx, cy, r, depth, dusty));
    }
    // And alone: a mark with others close by is a pattern, not dust.
    for (k, &(cx, cy, r, depth, dusty)) in blobs.iter().enumerate() {
        let reach = r * 6.0 + 10.0;
        let alone = blobs
            .iter()
            .enumerate()
            .all(|(j, b)| j == k || (b.0 - cx).powi(2) + (b.1 - cy).powi(2) > reach * reach);
        if dusty && alone {
            spots.push((depth, Spot { x: cx / w as f32, y: cy / h as f32, radius: (r * 1.8 + 1.5) / w as f32 }));
        }
    }
    spots.sort_by(|a, b| b.0.total_cmp(&a.0));
    spots.truncate(40);
    spots.into_iter().map(|(_, s)| s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    #[test]
    fn dust_is_found_on_a_smooth_sky_and_texture_is_left() {
        let (w, h) = (300u32, 200u32);
        let mut img = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
        for i in 0..img.len() {
            let (x, y) = (i % w as usize, i / w as usize);
            let sky = 0.3 + 0.1 * y as f32 / h as f32;
            // Right third: a busy texture (foliage), where specks are part of the scene.
            let v = if x > 200 { sky * (0.5 + 0.5 * ((x * 7 + y * 13) % 5) as f32 / 4.0) } else { sky };
            (img.r[i], img.g[i], img.b[i]) = (v, v, v);
        }
        for (cx, cy) in [(60usize, 50usize), (150, 140), (250, 100)] {
            for dy in 0..5 {
                for dx in 0..5 {
                    let i = (cy + dy - 2) * w as usize + cx + dx - 2;
                    img.r[i] *= 0.7;
                    img.g[i] *= 0.7;
                    img.b[i] *= 0.7;
                }
            }
        }
        let spots = dust_spots(&img);
        let found = |x: f32, y: f32| spots.iter().any(|s| (s.x * w as f32 - x).abs() < 3.0 && (s.y * h as f32 - y).abs() < 3.0);
        assert!(found(60.0, 50.0) && found(150.0, 140.0), "{spots:?}");
        assert!(!spots.iter().any(|s| s.x * w as f32 > 200.0), "texture left alone: {spots:?}");
    }
}
