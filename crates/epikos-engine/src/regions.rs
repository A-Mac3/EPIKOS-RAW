//! Step 3 masks for rendering and export: every [`MaskTarget`] as a 0–1 plane of the
//! upright scene (Steps 1–2 develop) at up to [`MASK_INPUT_SIDE`], cached per image
//! and lens geometry.
//!
//! | Target      | Source                                                   |
//! |-------------|----------------------------------------------------------|
//! | Subject     | IS-Net                                                   |
//! | Background  | 1 − subject (formed in the pipeline)                     |
//! | Sky         | U²-Net skyseg                                            |
//! | Skin        | colour model; wider (all Monk tones) inside the subject; |
//! |             | plus face-parsed skin                                    |
//! | Eyes, Hair  | BiSeNet face parsing on face crops found from skin blobs |
//! | Foreground  | depth, split at its Otsu threshold                       |

use std::sync::{Arc, PoisonError};

use epikos_core::{resize_plane, Result};
use epikos_masks::{MaskKind, RgbImage};
use epikos_pipeline::{skin_likelihood, skin_likelihood_relaxed};
use epikos_sidecar::{Adjustments, MaskTarget};
use serde::Serialize;

use crate::{render, rgba_to_rgb, scene_only, Engine, Loaded, MASK_INPUT_SIDE};

/// One mask, row-major, 0–1.
#[derive(Debug, Clone)]
pub struct MaskData {
    pub target: MaskTarget,
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
    /// Model time for this mask (0 when it came from a cache or a colour model).
    pub infer_ms: u64,
}

impl MaskData {
    pub fn coverage(&self) -> f32 {
        self.data.iter().sum::<f32>() / self.data.len().max(1) as f32
    }

    /// 8-bit alpha for the UI overlay.
    pub fn alpha(&self) -> Vec<u8> {
        self.data.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
    }
}

/// Which targets can be produced with the installed models.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetStatus {
    pub target: MaskTarget,
    pub available: bool,
    /// What provides it, for the UI.
    pub source: &'static str,
}

impl Engine {
    pub fn mask_targets(&self) -> Vec<TargetStatus> {
        let model = |k: MaskKind| self.masker.status().iter().any(|s| s.kind == k && s.available);
        MaskTarget::ALL
            .iter()
            .map(|&target| {
                let (available, source) = match target {
                    MaskTarget::Subject => (model(MaskKind::Subject), "IS-Net"),
                    MaskTarget::Background => (model(MaskKind::Subject), "inverse of the subject"),
                    MaskTarget::Sky => (model(MaskKind::Sky), "U²-Net skyseg"),
                    MaskTarget::Skin => (true, "colour model, subject and face parsing"),
                    MaskTarget::Eyes | MaskTarget::Hair => (self.masker.face_available(), "BiSeNet face parsing"),
                    MaskTarget::Foreground => (self.masker.depth_available(), "Depth Anything V2"),
                };
                TargetStatus { target, available, source }
            })
            .collect()
    }

    fn target_available(&self, target: MaskTarget) -> bool {
        self.mask_targets().iter().any(|t| t.target == target && t.available)
    }

    /// The mask for `target`, or `None` when its model isn't installed. The model runs
    /// once on the unstraightened frame; straighten and perspective only warp its output
    /// (a few milliseconds), so dragging those sliders never re-runs a model.
    pub(crate) fn mask_plane(
        &self,
        loaded: &Loaded,
        adjustments: &Adjustments,
        target: MaskTarget,
    ) -> Result<Option<Arc<MaskData>>> {
        let lens = &adjustments.lens;
        if !lens.has_transform() {
            return self.mask_plane_flat(loaded, adjustments, target);
        }
        let flat_adj = crate::unstraightened(adjustments);
        let target = if target == MaskTarget::Background { MaskTarget::Subject } else { target };
        let key = format!("warp|{}|{}|{}", lens.rotation, lens.vertical, cache_key(&flat_adj, target));
        if let Some(m) = cached(loaded, &key) {
            return Ok(Some(m));
        }
        let Some(flat) = self.mask_plane_flat(loaded, &flat_adj, target)? else { return Ok(None) };
        let warped = Arc::new(MaskData {
            data: crate::warp_plane(&flat.data, flat.width, flat.height, lens.rotation, lens.vertical),
            ..(*flat).clone()
        });
        let mut cache = loaded.planes.lock().unwrap_or_else(PoisonError::into_inner);
        // Keep only the latest two warps per target: a slider drag makes many.
        let prefix = format!("|{}", cache_key(&flat_adj, target));
        let mut seen = 0;
        for i in (0..cache.len()).rev() {
            if cache[i].0.starts_with("warp|") && cache[i].0.ends_with(&prefix) {
                seen += 1;
                if seen >= 2 {
                    cache.remove(i);
                }
            }
        }
        cache.push((key, warped.clone()));
        Ok(Some(warped))
    }

    fn mask_plane_flat(
        &self,
        loaded: &Loaded,
        adjustments: &Adjustments,
        target: MaskTarget,
    ) -> Result<Option<Arc<MaskData>>> {
        let target = if target == MaskTarget::Background { MaskTarget::Subject } else { target };
        if !self.target_available(target) {
            return Ok(None);
        }
        let key = cache_key(adjustments, target);
        if let Some(m) = cached(loaded, &key) {
            return Ok(Some(m));
        }
        let planes: Vec<MaskData> = match target {
            MaskTarget::Subject | MaskTarget::Sky => {
                let kind = if target == MaskTarget::Subject { MaskKind::Subject } else { MaskKind::Sky };
                let m = self.segment(loaded, adjustments, kind)?;
                vec![MaskData {
                    target,
                    width: m.width,
                    height: m.height,
                    data: m.alpha.iter().map(|&a| a as f32 / 255.0).collect(),
                    infer_ms: m.infer_ms,
                }]
            }
            MaskTarget::Skin => {
                let (width, height, data) = self.inclusive_skin(loaded, adjustments)?;
                vec![MaskData { target, width, height, data, infer_ms: 0 }]
            }
            MaskTarget::Eyes | MaskTarget::Hair => self.face_planes(loaded, adjustments)?,
            MaskTarget::Foreground => {
                let d = self.depth_for(loaded, adjustments)?;
                vec![MaskData {
                    target,
                    width: d.width,
                    height: d.height,
                    data: foreground(&d.depth),
                    infer_ms: d.infer_ms,
                }]
            }
            MaskTarget::Background => unreachable!("mapped to the subject above"),
        };
        let mut out = None;
        let mut cache = loaded.planes.lock().unwrap_or_else(PoisonError::into_inner);
        for plane in planes {
            let plane = Arc::new(plane);
            let k = cache_key(adjustments, plane.target);
            cache.retain(|(key, _)| *key != k);
            if plane.target == target {
                out = Some(plane.clone());
            }
            cache.push((k, plane));
        }
        Ok(out)
    }

    /// A loose skin map for finding faces: the strict colour model everywhere and the
    /// relaxed one (every Monk tone, but also beige and sand) inside the subject.
    fn person_skin(&self, loaded: &Loaded, adjustments: &Adjustments) -> Result<(u32, u32, Vec<f32>)> {
        let rgb = loaded.develop_scene(MASK_INPUT_SIDE, adjustments)?;
        let (w, h) = (rgb.width, rgb.height);
        let mut skin = skin_likelihood(&rgb);
        if let Some(subject) = self.mask_plane(loaded, adjustments, MaskTarget::Subject)? {
            let subject = crate::analysis::fit(&subject.data, subject.width, subject.height, w, h);
            let relaxed = skin_likelihood_relaxed(&rgb);
            for ((s, r), m) in skin.iter_mut().zip(relaxed).zip(subject) {
                *s = s.max(r * m);
            }
        }
        Ok((w, h, skin))
    }

    /// The Step 3 Skin mask, for every Monk tone: the strict colour model, face-parsed
    /// skin, and — inside the subject — pixels close in colour to that person's own skin
    /// (learnt from face-parsed skin, else from confident colour skin). A fair or a very
    /// deep face is covered; a sand-coloured strap or a beige coat is not, unless it
    /// really matches the skin.
    fn inclusive_skin(&self, loaded: &Loaded, adjustments: &Adjustments) -> Result<(u32, u32, Vec<f32>)> {
        let rgb = loaded.develop_scene(MASK_INPUT_SIDE, adjustments)?;
        let (w, h) = (rgb.width, rgb.height);
        let n = (w * h) as usize;
        let mut skin = skin_likelihood(&rgb);
        let face = if self.masker.face_available() {
            let f = match cached(loaded, &face_skin_key(adjustments)) {
                Some(f) => f,
                None => {
                    self.face_planes(loaded, adjustments)?;
                    cached(loaded, &face_skin_key(adjustments)).expect("face_planes caches face skin")
                }
            };
            Some(crate::analysis::fit(&f.data, f.width, f.height, w, h))
        } else {
            None
        };
        let Some(subject) = self.mask_plane(loaded, adjustments, MaskTarget::Subject)? else {
            if let Some(f) = face {
                skin.iter_mut().zip(f).for_each(|(s, f)| *s = s.max(f));
            }
            return Ok((w, h, skin));
        };
        let subject = crate::analysis::fit(&subject.data, subject.width, subject.height, w, h);
        let lab = epikos_pipeline::oklab_planes(&rgb);
        let relaxed = skin_likelihood_relaxed(&rgb);
        // The person's skin colour: face-parsed skin first, else confident colour skin
        // on the subject.
        let seeds: Vec<usize> = match &face {
            Some(f) if f.iter().filter(|&&v| v > 0.5).count() > n / 2000 => (0..n).filter(|&i| f[i] > 0.5).collect(),
            _ => (0..n).filter(|&i| skin[i] > 0.5 && subject[i] > 0.5).collect(),
        };
        let model = (seeds.len() > n / 4000).then(|| SkinColour::learn(&lab, &seeds));
        for i in 0..n {
            let inside = subject[i];
            let colour = skin[i].max(relaxed[i]);
            let on_person = match &model {
                // On the subject, colour skin counts only where it matches this person's
                // own skin (a khaki strap passes the colour model, not the match).
                Some(m) => colour * m.likelihood(lab.r[i], lab.g[i], lab.b[i]),
                // No skin to learn from (e.g. a very fair face without the face model):
                // the relaxed model on the subject, as the only evidence there is.
                None => colour,
            };
            let blended = skin[i] * (1.0 - inside) + on_person * inside;
            skin[i] = blended.max(face.as_ref().map_or(0.0, |f| f[i]));
        }
        Ok((w, h, skin))
    }

    /// Eyes and hair (and face skin, cached for the Skin mask) from one face-parsing
    /// pass over every face found in the frame.
    fn face_planes(&self, loaded: &Loaded, adjustments: &Adjustments) -> Result<Vec<MaskData>> {
        let display = render(loaded, &scene_only(adjustments), MASK_INPUT_SIDE, MASK_INPUT_SIDE)?;
        let rgb = rgba_to_rgb(&display);
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);
        // Faces are found from skin of any tone, so fair and very deep faces are parsed too.
        let (_, _, skin) = self.person_skin(loaded, adjustments)?;
        let skin = if skin.len() == w * h { skin } else { vec![0.0; w * h] };
        let mut eyes = vec![0.0f32; w * h];
        let mut hair = vec![0.0f32; w * h];
        let mut face_skin = vec![0.0f32; w * h];
        let mut infer_ms = 0;
        for [x0, y0, x1, y1] in face_boxes(&skin, w, h) {
            let (cw, ch) = (x1 - x0, y1 - y0);
            let mut data = Vec::with_capacity(cw * ch * 3);
            for y in y0..y1 {
                let row = &rgb.as_raw()[(y * w + x0) * 3..(y * w + x1) * 3];
                data.extend_from_slice(row);
            }
            let crop = RgbImage { width: cw as u32, height: ch as u32, data };
            let parts = self.masker.parse_face(&crop)?;
            infer_ms += parts.infer_ms;
            // A crop that holds no face (a hand, a wooden table) is dropped.
            if parts.face_share() < 0.12 {
                continue;
            }
            // Fade out towards the crop's edges, where the parser sees a cut-off head.
            let feather = |v: usize, n: usize| {
                let e = (n as f32 * 0.08).max(1.0);
                let d = v.min(n - 1 - v) as f32;
                (d / e).clamp(0.0, 1.0)
            };
            for y in 0..ch {
                for x in 0..cw {
                    let (i, j) = ((y0 + y) * w + x0 + x, y * cw + x);
                    let f = feather(x, cw).min(feather(y, ch));
                    // Low probabilities are the model's doubt, not a thin mask.
                    eyes[i] = eyes[i].max(f * confident(parts.eyes[j]));
                    hair[i] = hair[i].max(f * confident(parts.hair[j]));
                    face_skin[i] = face_skin[i].max(f * confident(parts.skin[j]));
                }
            }
        }
        let (width, height) = (w as u32, h as u32);
        let face = Arc::new(MaskData { target: MaskTarget::Skin, width, height, data: face_skin, infer_ms: 0 });
        {
            let key = face_skin_key(adjustments);
            let mut cache = loaded.planes.lock().unwrap_or_else(PoisonError::into_inner);
            cache.retain(|(k, _)| *k != key);
            cache.push((key, face));
        }
        Ok(vec![
            MaskData { target: MaskTarget::Eyes, width, height, data: eyes, infer_ms },
            MaskData { target: MaskTarget::Hair, width, height, data: hair, infer_ms: 0 },
        ])
    }
}

fn confident(p: f32) -> f32 {
    let t = ((p - 0.35) / 0.4).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One person's skin colour in Oklab: mean and spread, with floors so a small or very
/// even patch still accepts the same skin in other light (shadow side, highlights).
struct SkinColour {
    mean: [f32; 3],
    spread: [f32; 3],
}

impl SkinColour {
    fn learn(lab: &epikos_core::ImageRgbF32, idx: &[usize]) -> Self {
        let n = idx.len().max(1) as f32;
        let planes = [&lab.r, &lab.g, &lab.b];
        let mean: [f32; 3] = std::array::from_fn(|c| idx.iter().map(|&i| planes[c][i]).sum::<f32>() / n);
        let floors = [0.1, 0.015, 0.015];
        let spread: [f32; 3] = std::array::from_fn(|c| {
            let var = idx.iter().map(|&i| (planes[c][i] - mean[c]).powi(2)).sum::<f32>() / n;
            var.sqrt().max(floors[c])
        });
        Self { mean, spread }
    }

    /// 1 within ~1.2 spreads of the person's typical skin colour, 0 beyond 2.2; lightness
    /// counts less than colour (skin is lit and shaded across a face).
    fn likelihood(&self, l: f32, a: f32, b: f32) -> f32 {
        let d = |v: f32, c: usize, k: f32| (v - self.mean[c]) / (k * self.spread[c]);
        let d2 = d(l, 0, 1.6).powi(2) + d(a, 1, 1.0).powi(2) + d(b, 2, 1.0).powi(2);
        (1.0 - (d2.sqrt() - 1.2) / 1.0).clamp(0.0, 1.0)
    }
}

/// Face-parsed skin, kept apart from the combined Skin mask.
fn face_skin_key(a: &Adjustments) -> String {
    format!("face-skin|{:?}", a.lens)
}

fn cached(loaded: &Loaded, key: &str) -> Option<Arc<MaskData>> {
    let cache = loaded.planes.lock().unwrap_or_else(PoisonError::into_inner);
    cache.iter().find(|(k, _)| k == key).map(|(_, m)| m.clone())
}

/// Masks follow the geometry (lens, straighten); the colour skin model also follows
/// white balance and exposure.
fn cache_key(a: &Adjustments, target: MaskTarget) -> String {
    match target {
        MaskTarget::Skin => format!("{}|{:?}|{:?}|{}|{:?}", target.id(), a.lens, a.white_balance, a.exposure, a.tone),
        _ => format!("{}|{:?}", target.id(), a.lens),
    }
}

/// Near part of the scene: depth split at its Otsu threshold, with a soft edge.
pub(crate) fn foreground(depth: &[f32]) -> Vec<f32> {
    const BINS: usize = 64;
    let mut hist = [0usize; BINS];
    for &d in depth {
        hist[((d.clamp(0.0, 1.0) * (BINS - 1) as f32).round()) as usize] += 1;
    }
    let total = depth.len().max(1) as f64;
    let sum_all: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
    let mut between = [0.0f64; BINS];
    let (mut w0, mut sum0) = (0.0, 0.0);
    for (t, &c) in hist.iter().enumerate() {
        w0 += c as f64;
        sum0 += t as f64 * c as f64;
        let w1 = total - w0;
        if w0 > 0.0 && w1 > 0.0 {
            let (m0, m1) = (sum0 / w0, (sum_all - sum0) / w1);
            between[t] = w0 * w1 * (m0 - m1).powi(2);
        }
    }
    // Every split across an empty gap scores the same: cut in the middle of the gap.
    let best = between.iter().copied().fold(0.0, f64::max);
    let tied: Vec<usize> = (0..BINS).filter(|&t| best > 0.0 && between[t] >= best * (1.0 - 1e-9)).collect();
    let split = match (tied.first(), tied.last()) {
        (Some(&a), Some(&b)) => (a + b) as f32 / 2.0,
        _ => (BINS / 2) as f32,
    };
    let t = (split + 0.5) / (BINS - 1) as f32;
    depth
        .iter()
        .map(|&d| {
            let x = ((d - (t - 0.05)) / 0.1).clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        })
        .collect()
}

/// Square crops around face-like skin blobs (largest first, up to four), in pixels of
/// the `w × h` frame: `[x0, y0, x1, y1]`.
pub(crate) fn face_boxes(skin: &[f32], w: usize, h: usize) -> Vec<[usize; 4]> {
    if w < 8 || h < 8 || skin.len() != w * h {
        return Vec::new();
    }
    // Work at ≤ 256 px: faces are blobs, not details.
    let scale = (256.0 / w.max(h) as f32).min(1.0);
    let (sw, sh) = (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1));
    let small = resize_plane(skin, w as u32, h as u32, sw as u32, sh as u32);
    let on: Vec<bool> = small.iter().map(|&v| v > 0.5).collect();
    let mut label = vec![0u32; sw * sh];
    let mut blobs = Vec::new(); // (area, x0, y0, x1, y1)
    let mut next = 0;
    for start in 0..sw * sh {
        if !on[start] || label[start] != 0 {
            continue;
        }
        next += 1;
        let mut stack = vec![start];
        label[start] = next;
        let (mut area, mut x0, mut y0, mut x1, mut y1) = (0usize, sw, sh, 0usize, 0usize);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % sw, i / sw);
            area += 1;
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            let mut visit = |j: usize| {
                if on[j] && label[j] == 0 {
                    label[j] = next;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < sw {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - sw);
            }
            if y + 1 < sh {
                visit(i + sw);
            }
        }
        blobs.push((area, x0, y0, x1 + 1, y1 + 1));
    }
    let min_area = (sw * sh / 1500).max(12);
    blobs.retain(|&(area, x0, y0, x1, y1)| {
        let (bw, bh) = ((x1 - x0) as f32, (y1 - y0) as f32);
        // Faces (with neck) are roughly as tall as wide to three times as tall, and
        // fill a good part of their box.
        area >= min_area && (0.6..=3.0).contains(&(bh / bw)) && area as f32 >= 0.35 * bw * bh
    });
    blobs.sort_by_key(|b| std::cmp::Reverse(b.0));
    let boxes: Vec<[usize; 4]> = blobs
        .into_iter()
        .map(|(_, x0, y0, x1, y1)| {
            let inv = 1.0 / scale;
            let (bw, bh) = ((x1 - x0) as f32 * inv, (y1 - y0) as f32 * inv);
            // The skin blob is the face (and neck); hair and forehead sit above it.
            let face = bw.max(bh.min(1.3 * bw));
            let side = (2.0 * face).min(w.min(h) as f32);
            let cx = (x0 + x1) as f32 * 0.5 * inv;
            let cy = y0 as f32 * inv + 0.5 * face - 0.1 * side;
            let x0 = (cx - side / 2.0).clamp(0.0, w as f32 - side) as usize;
            let y0 = (cy - side / 2.0).clamp(0.0, h as f32 - side) as usize;
            let s = side as usize;
            [x0, y0, (x0 + s).min(w), (y0 + s).min(h)]
        })
        .filter(|[x0, y0, x1, y1]| x1 - x0 >= 16 && y1 - y0 >= 16)
        .collect();
    // One crop per head: a beard or shadow can split a face into two skin blobs, and
    // the smaller one's crop would parse a half face.
    let mut out: Vec<[usize; 4]> = Vec::new();
    for b in boxes {
        let overlaps = out.iter().any(|a| {
            let ix = a[2].min(b[2]).saturating_sub(a[0].max(b[0]));
            let iy = a[3].min(b[3]).saturating_sub(a[1].max(b[1]));
            let area = |r: &[usize; 4]| (r[2] - r[0]) * (r[3] - r[1]);
            ix * iy * 4 > area(&b).min(area(a))
        });
        if !overlaps && out.len() < 4 {
            out.push(b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_splits_near_from_far() {
        let depth: Vec<f32> = (0..100).map(|i| if i < 30 { 0.9 } else { 0.2 }).collect();
        let f = foreground(&depth);
        assert!(f[0] > 0.99 && f[99] < 0.01);
    }

    #[test]
    fn face_boxes_find_a_face_blob_and_skip_thin_strips() {
        let (w, h) = (400, 300);
        let mut skin = vec![0.0f32; w * h];
        // A face: 60 × 80 ellipse centred at (200, 120).
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = ((x as f32 - 200.0) / 30.0, (y as f32 - 120.0) / 40.0);
                if dx * dx + dy * dy <= 1.0 {
                    skin[y * w + x] = 1.0;
                }
            }
        }
        // A thin arm-like strip.
        for y in 250..258 {
            for x in 20..220 {
                skin[y * w + x] = 1.0;
            }
        }
        // A second blob inside the same head (a beard splits the skin) adds no crop.
        for y in 150..160 {
            for x in 190..210 {
                skin[y * w + x] = 0.0;
            }
        }
        let boxes = face_boxes(&skin, w, h);
        assert_eq!(boxes.len(), 1, "{boxes:?}");
        let [x0, y0, x1, y1] = boxes[0];
        assert!(x0 < 170 && x1 > 230 && y0 < 80 && y1 > 160, "{:?}", boxes[0]);
        assert_eq!(x1 - x0, y1 - y0);
    }
}
