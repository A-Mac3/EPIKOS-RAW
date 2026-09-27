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
                    MaskTarget::Eyes | MaskTarget::Hair | MaskTarget::Eyebrows | MaskTarget::Teeth | MaskTarget::FaceSkin => {
                        (self.masker.face_available(), "BiSeNet face parsing")
                    }
                    MaskTarget::Eyelashes => (self.masker.face_available(), "face parsing: dark detail at the eyes' edge"),
                    MaskTarget::BodySkin => (true, "Skin minus facial skin"),
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
            MaskTarget::Eyes
            | MaskTarget::Hair
            | MaskTarget::Eyebrows
            | MaskTarget::Eyelashes
            | MaskTarget::Teeth
            | MaskTarget::FaceSkin => self.face_planes(loaded, adjustments)?,
            MaskTarget::BodySkin => {
                let Some(skin) = self.mask_plane_flat(loaded, adjustments, MaskTarget::Skin)? else { return Ok(None) };
                let face = if self.masker.face_available() {
                    self.mask_plane_flat(loaded, adjustments, MaskTarget::FaceSkin)?
                        .map(|f| crate::analysis::fit(&f.data, f.width, f.height, skin.width, skin.height))
                } else {
                    None
                };
                let data = match face {
                    Some(f) => skin.data.iter().zip(f).map(|(s, f)| (s - f).max(0.0)).collect(),
                    None => skin.data.clone(),
                };
                vec![MaskData { target, width: skin.width, height: skin.height, data, infer_ms: 0 }]
            }
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

    /// The Step 3 Skin mask, for every Monk tone: skin colour (strict or relaxed model)
    /// that is on the subject *and* close to that person's own skin colour (learnt from
    /// face-parsed skin, else from confident colour skin), plus face-parsed skin. A fair
    /// or a very deep face is covered; a wall, a sand-coloured strap or a beige coat is
    /// not. Without the subject model, the strict colour model alone.
    fn inclusive_skin(&self, loaded: &Loaded, adjustments: &Adjustments) -> Result<(u32, u32, Vec<f32>)> {
        let rgb = loaded.develop_scene(MASK_INPUT_SIDE, adjustments)?;
        let (w, h) = (rgb.width, rgb.height);
        let n = (w * h) as usize;
        let mut skin = skin_likelihood(&rgb);
        let face = if self.masker.face_available() {
            let f = match cached(loaded, &face_skin_key(adjustments)) {
                Some(f) => f,
                None => {
                    // Through the cache, so hair and the other features are kept too.
                    self.mask_plane_flat(loaded, adjustments, MaskTarget::FaceSkin)?;
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
            // No face parsed: learn only from the heads (top of the silhouette), never
            // from the whole body, where a warm jumper would outvote the skin.
            _ => {
                let heads = head_boxes(&subject, w as usize, h as usize);
                (0..n)
                    .filter(|&i| {
                        let (x, y) = (i % w as usize, i / w as usize);
                        skin[i] > 0.5 && subject[i] > 0.5 && heads.iter().any(|b| x >= b[0] && x < b[2] && y >= b[1] && y < b[3])
                    })
                    .collect()
            }
        };
        let model = (seeds.len() > n / 4000).then(|| SkinColour::learn(&lab, &seeds));
        // The skin's median lightness (for telling a beard from shaded skin).
        let skin_l = {
            let mut l: Vec<f32> = seeds.iter().map(|&i| lab.r[i]).collect();
            if l.is_empty() {
                0.5
            } else {
                let mid = l.len() / 2;
                *l.select_nth_unstable_by(mid, f32::total_cmp).1
            }
        };
        let cloth = cached(loaded, &face_cloth_key(adjustments)).map(|c| crate::analysis::fit(&c.data, c.width, c.height, w, h));
        let hair = cached(loaded, &cache_key(adjustments, MaskTarget::Hair)).map(|c| crate::analysis::fit(&c.data, c.width, c.height, w, h));
        // Hair is textured, strongly lit skin smooth: the face model sometimes calls a
        // bright cheek by the hairline "hair", so hair counts only where it's textured.
        let texture: Vec<f32> = {
            let (wu, hu) = (w as usize, h as usize);
            let mean = crate::analysis::box_mean(&lab.r, wu, hu, 2);
            let sq: Vec<f32> = lab.r.iter().map(|v| v * v).collect();
            let mean_sq = crate::analysis::box_mean(&sq, wu, hu, 2);
            (0..n).map(|i| smoothstep(0.012, 0.03, (mean_sq[i] - mean[i] * mean[i]).max(0.0).sqrt())).collect()
        };
        // Around the parsed face: a reach of about a quarter of the face's size.
        let face_near = face.as_ref().and_then(|f| {
            let area = f.iter().filter(|&&v| v > 0.5).count();
            (area > n / 2000).then(|| {
                let r = ((area as f32).sqrt() * 0.25).round().max(2.0) as usize;
                crate::analysis::box_mean(f, w as usize, h as usize, r).into_iter().map(|v| smoothstep(0.01, 0.08, v)).collect::<Vec<f32>>()
            })
        });
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
            // Skin only on the person: a warm wall, wood or clothing outside the subject
            // never counts, whatever its colour. Right next to the parsed face (a
            // strongly lit or shaded cheek, the ears, the neck), skin colour on the
            // subject counts even where the light makes it unlike the rest.
            // Clothing and hair the face model saw keep collars and braids out.
            let not_cloth = 1.0 - cloth.as_ref().map_or(0.0, |c| c[i]);
            let not_hair = 1.0 - hair.as_ref().map_or(0.0, |c| c[i]) * texture[i];
            // A beard: textured and clearly darker than this person's skin.
            let not_beard = 1.0 - texture[i] * smoothstep(0.04, 0.1, skin_l - lab.r[i]);
            let by_face = face_near.as_ref().map_or(0.0, |f| f[i]) * colour * not_cloth * not_hair * not_beard * inside;
            skin[i] = (on_person * inside).max(by_face).max(face.as_ref().map_or(0.0, |f| f[i]));
        }
        Ok((w, h, skin))
    }

    /// Face features from one face-parsing pass over every face in the frame: eyes,
    /// eyebrows, the lash line, teeth, facial skin (also cached for the Skin mask) and
    /// hair. Faces are found at mask resolution and parsed from a render at twice that,
    /// so small features (eyes, lashes, teeth) keep their shape.
    fn face_planes(&self, loaded: &Loaded, adjustments: &Adjustments) -> Result<Vec<MaskData>> {
        // Faces are found from skin of any tone, so fair and very deep faces are parsed too.
        let (sw, sh, skin) = self.person_skin(loaded, adjustments)?;
        // Heads from the subject's silhouette first, then faces found from skin blobs
        // that aren't one of those heads.
        let subject_small = self
            .mask_plane(loaded, adjustments, MaskTarget::Subject)?
            .map(|sub| crate::analysis::fit(&sub.data, sub.width, sub.height, sw, sh));
        let mut boxes: Vec<[usize; 4]> = match &subject_small {
            Some(sub) => head_boxes(sub, sw as usize, sh as usize)
                .into_iter()
                // A head shows some skin; the top of a lamp or a bag doesn't.
                .filter(|&[x0, y0, x1, y1]| {
                    let area = ((x1 - x0) * (y1 - y0)).max(1);
                    let on = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| skin[y * sw as usize + x] > 0.5).count();
                    on * 25 >= area
                })
                .collect(),
            None => Vec::new(),
        };
        for b in face_boxes(&skin, sw as usize, sh as usize) {
            // A face is on the subject: a skin-coloured wall or wood beside it isn't.
            let on_subject = subject_small.as_ref().is_none_or(|sub| {
                let [x0, y0, x1, y1] = b;
                let area = ((x1 - x0) * (y1 - y0)).max(1);
                let on = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| sub[y * sw as usize + x] > 0.5).count();
                on * 10 >= area * 3
            });
            if on_subject && !boxes.iter().any(|a| overlap(a, &b) > 0.3) {
                boxes.push(b);
            }
        }
        let display = render(loaded, &scene_only(adjustments), MASK_INPUT_SIDE * FACE_DETAIL, MASK_INPUT_SIDE * FACE_DETAIL)?;
        let rgb = rgba_to_rgb(&display);
        let lab = crate::learn::display_lab(&display);
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);
        let (fx, fy) = (w as f32 / sw as f32, h as f32 / sh as f32);
        let n = w * h;
        let plane = || vec![0.0f32; n];
        let (mut eyes, mut brows, mut lips, mut mouth, mut hair) = (plane(), plane(), plane(), plane(), plane());
        let (mut neck, mut cloth, mut face_skin) = (plane(), plane(), plane());
        let mut heads: Vec<[usize; 4]> = Vec::new();
        let mut infer_ms = 0;
        for [bx0, by0, bx1, by1] in boxes {
            let x0 = ((bx0 as f32 * fx) as usize).min(w - 1);
            let y0 = ((by0 as f32 * fy) as usize).min(h - 1);
            let x1 = ((bx1 as f32 * fx).ceil() as usize).clamp(x0 + 1, w);
            let y1 = ((by1 as f32 * fy).ceil() as usize).clamp(y0 + 1, h);
            let (cw, ch) = (x1 - x0, y1 - y0);
            let mut data = Vec::with_capacity(cw * ch * 3);
            for y in y0..y1 {
                data.extend_from_slice(&rgb.as_raw()[(y * w + x0) * 3..(y * w + x1) * 3]);
            }
            let crop = RgbImage { width: cw as u32, height: ch as u32, data };
            let parts = self.masker.parse_face(&crop)?;
            infer_ms += parts.infer_ms;
            // A crop that holds no face (a hand, a wooden table) is dropped.
            if parts.face_share() < 0.12 {
                continue;
            }
            heads.push([x0, y0, x1, y1]);
            // Fade out towards the crop's edges, where the parser sees a cut-off head.
            let feather = |v: usize, n: usize| {
                let e = (n as f32 * 0.08).max(1.0);
                (v.min(n - 1 - v) as f32 / e).clamp(0.0, 1.0)
            };
            for y in 0..ch {
                for x in 0..cw {
                    let (i, j) = ((y0 + y) * w + x0 + x, y * cw + x);
                    let f = feather(x, cw).min(feather(y, ch));
                    // Low probabilities are the model's doubt, not a thin mask.
                    for (dst, src) in [
                        (&mut eyes, &parts.eyes),
                        (&mut brows, &parts.brows),
                        (&mut lips, &parts.lips),
                        (&mut mouth, &parts.mouth),
                        (&mut hair, &parts.hair),
                        (&mut neck, &parts.neck),
                        (&mut cloth, &parts.cloth),
                        (&mut face_skin, &parts.skin),
                    ] {
                        dst[i] = dst[i].max(f * confident(src[j]));
                    }
                }
            }
        }

        // Teeth: the light, low-colour part of the inside of the mouth; lips never.
        let teeth: Vec<f32> = (0..n)
            .map(|i| {
                let c = lab.g[i].hypot(lab.b[i]);
                mouth[i] * smoothstep(0.45, 0.62, lab.r[i]) * (1.0 - smoothstep(0.07, 0.13, c)) * (1.0 - lips[i])
            })
            .collect();

        // The lash line: detail darker than its surroundings in a thin band around each
        // eye (no lash class). Darker *relative* to the skin around it, so it works on
        // every skin tone.
        let band = heads.iter().map(|b| ((b[3] - b[1]) as f32 * 0.03).round() as usize).max().unwrap_or(2).max(2);
        let near_eye = crate::analysis::box_mean(&eyes, w, h, (band / 2).max(1));
        let around = crate::analysis::box_mean(&lab.r, w, h, band * 2);
        let lashes: Vec<f32> = (0..n)
            .map(|i| {
                let ring = smoothstep(0.08, 0.3, near_eye[i]) * (1.0 - eyes[i]);
                ring * smoothstep(0.04, 0.1, around[i] - lab.r[i]) * (1.0 - brows[i])
            })
            .collect();

        // Hair: the model's, or where it finds almost none (very dark or close-cropped
        // hair, a beard, which the face model reads as skin), the parts of the head
        // clearly darker than this person's own skin, clothing and features excluded,
        // with thin rims (feature outlines) opened away.
        let hair_area = hair.iter().filter(|&&v| v > 0.5).count() as f32 / n as f32;
        if !heads.is_empty() && hair_area < 0.01 {
            let mut skin_l: Vec<f32> = (0..n).filter(|&i| face_skin[i] > 0.6).map(|i| lab.r[i]).collect();
            if let (Some(subject), true) = (self.mask_plane(loaded, adjustments, MaskTarget::Subject)?, skin_l.len() > 50) {
                let subject = crate::analysis::fit(&subject.data, subject.width, subject.height, w as u32, h as u32);
                let mid = skin_l.len() / 2;
                let skin_mid = *skin_l.select_nth_unstable_by(mid, f32::total_cmp).1;
                let features: Vec<f32> = (0..n).map(|i| eyes[i].max(brows[i]).max(lips[i]).max(mouth[i])).collect();
                let features = crate::analysis::box_mean(&features, w, h, band);
                // Hair is textured; skin in shadow is smooth: local variation of lightness.
                let r_tex = (band / 2).max(1);
                let mean_l = crate::analysis::box_mean(&lab.r, w, h, r_tex);
                let sq: Vec<f32> = lab.r.iter().map(|v| v * v).collect();
                let mean_sq = crate::analysis::box_mean(&sq, w, h, r_tex);
                let texture: Vec<f32> =
                    (0..n).map(|i| smoothstep(0.012, 0.03, (mean_sq[i] - mean_l[i] * mean_l[i]).max(0.0).sqrt())).collect();
                // The skin's lightness nearby (a face lit from one side is darker on the
                // other): hair is darker than the skin *next to it*.
                let r_skin = heads.iter().map(|b| (b[2] - b[0]) / 5).max().unwrap_or(8).max(4);
                let weighted: Vec<f32> = (0..n).map(|i| lab.r[i] * face_skin[i]).collect();
                let (sum_l, sum_w) = (crate::analysis::box_mean(&weighted, w, h, r_skin), crate::analysis::box_mean(&face_skin, w, h, r_skin));
                let mut candidate = vec![0.0f32; n];
                for &[x0, y0, x1, y1] in &heads {
                    // The head: the face box, higher (hair, crown) and lower (beard).
                    let (bw, bh) = ((x1 - x0) as f32, (y1 - y0) as f32);
                    let hy0 = (y0 as f32 - 0.45 * bh).max(0.0) as usize;
                    let hy1 = ((y1 as f32 + 0.25 * bh) as usize).min(h);
                    // Wide: the face box is found from skin, which a beard or side hair isn't.
                    let hx0 = (x0 as f32 - 0.45 * bw).max(0.0) as usize;
                    let hx1 = ((x1 as f32 + 0.45 * bw) as usize).min(w);
                    for y in hy0..hy1 {
                        for x in hx0..hx1 {
                            let i = y * w + x;
                            let skin_here = if sum_w[i] > 0.05 { sum_l[i] / sum_w[i] } else { skin_mid };
                            let darker = smoothstep(0.02, 0.1, skin_here - lab.r[i]);
                            let c = lab.g[i].hypot(lab.b[i]);
                            // Hair is dark, textured and low in colour; lenses and straps
                            // shine or tint, and skin in shadow is smooth.
                            let hairlike = 1.0 - smoothstep(0.1, 0.18, c);
                            candidate[i] = subject[i]
                                * darker
                                * texture[i]
                                * hairlike
                                // The parser often calls a beard clothing: clothing counts
                                // only where it's smooth like fabric.
                                * (1.0 - cloth[i] * (1.0 - texture[i]))
                                * (1.0 - smoothstep(0.02, 0.1, features[i]));
                        }
                    }
                }
                // Opening: keep areas, drop outlines a few pixels thick; soft edges stay.
                let r = (band * 2).max(3);
                let opened = crate::analysis::box_mean(&candidate, w, h, r);
                hair.iter_mut().zip(opened).for_each(|(v, o)| *v = v.max(smoothstep(0.2, 0.45, o)));
            }
        }

        let (width, height) = (w as u32, h as u32);
        let face = Arc::new(MaskData { target: MaskTarget::FaceSkin, width, height, data: face_skin.clone(), infer_ms: 0 });
        {
            let key = face_skin_key(adjustments);
            let cloth_key = face_cloth_key(adjustments);
            let clothing = Arc::new(MaskData { target: MaskTarget::Subject, width, height, data: cloth, infer_ms: 0 });
            let mut cache = loaded.planes.lock().unwrap_or_else(PoisonError::into_inner);
            cache.retain(|(k, _)| *k != key && *k != cloth_key);
            cache.push((key, face));
            cache.push((cloth_key, clothing));
        }
        let md = |target, data| MaskData { target, width, height, data, infer_ms: 0 };
        Ok(vec![
            MaskData { target: MaskTarget::Eyes, width, height, data: eyes, infer_ms },
            md(MaskTarget::Hair, hair),
            md(MaskTarget::Eyebrows, brows),
            md(MaskTarget::Eyelashes, lashes),
            md(MaskTarget::Teeth, teeth),
            md(MaskTarget::FaceSkin, face_skin),
        ])
    }
}

/// Intersection over the smaller box's area.
fn overlap(a: &[usize; 4], b: &[usize; 4]) -> f32 {
    let iw = a[2].min(b[2]).saturating_sub(a[0].max(b[0])) as f32;
    let ih = a[3].min(b[3]).saturating_sub(a[1].max(b[1])) as f32;
    let area = |r: &[usize; 4]| ((r[2] - r[0]) * (r[3] - r[1])) as f32;
    iw * ih / area(a).min(area(b)).max(1.0)
}

/// Face features are parsed at this multiple of the mask resolution.
const FACE_DETAIL: u32 = 2;

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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

/// Face-parsed clothing (around the heads), kept to keep collars out of the Skin mask.
fn face_cloth_key(a: &Adjustments) -> String {
    format!("face-cloth|{:?}", a.lens)
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
/// Square crops around the head of each person-sized subject region: the top of the
/// silhouette. Finds faces that skin colour can't separate, e.g. a face above a
/// skin-coloured jumper, where face and clothes form one skin blob.
pub(crate) fn head_boxes(subject: &[f32], w: usize, h: usize) -> Vec<[usize; 4]> {
    if w < 8 || h < 8 || subject.len() != w * h {
        return Vec::new();
    }
    let ranked = crate::composition::rank_subjects(subject, None, None, w, h);
    let mut out = Vec::new();
    for s in ranked.iter().filter(|s| s.area >= 0.01) {
        let [l, t, r, _] = s.bbox;
        let (x0, x1) = ((l * w as f32) as usize, ((r * w as f32).ceil() as usize).min(w));
        let top = (t * h as f32) as usize;
        // The top of the silhouette: the head (and hair).
        // Tall enough to reach past narrow hair (braids, a bun) to the face.
        let band = ((h as f32 * 0.2) as usize).max(4);
        let (mut widest, mut sum_x, mut count) = (0usize, 0.0f32, 0usize);
        for y in top..(top + band).min(h) {
            let row: Vec<usize> = (x0..x1).filter(|&x| subject[y * w + x] > 0.5).collect();
            widest = widest.max(row.len());
            sum_x += row.iter().sum::<usize>() as f32;
            count += row.len();
        }
        if count == 0 || widest < 6 {
            continue;
        }
        let cx = sum_x / count as f32;
        let side = ((widest as f32 * 2.0).min(w.min(h) as f32)) as usize;
        let bx0 = (cx - side as f32 / 2.0).clamp(0.0, (w - side) as f32) as usize;
        let by0 = (top as f32 - 0.05 * side as f32).clamp(0.0, (h - side) as f32) as usize;
        out.push([bx0, by0, (bx0 + side).min(w), (by0 + side).min(h)]);
    }
    out
}

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
    fn head_boxes_find_the_head_on_top_of_a_person() {
        // A person: narrow braids on top, a wider face, then wide shoulders below.
        let (w, h) = (200, 150);
        let mut subject = vec![0.0f32; w * h];
        for y in 0..h {
            let half = match y {
                20..=27 => 8,  // braids
                28..=60 => 18, // face
                61..=149 => 50, // body
                _ => 0,
            };
            for x in 100 - half..100 + half {
                subject[y * w + x] = 1.0;
            }
        }
        let boxes = head_boxes(&subject, w, h);
        assert_eq!(boxes.len(), 1);
        let [x0, y0, x1, y1] = boxes[0];
        // Wide enough for the face under the braids, and starting at the top.
        assert!(x0 <= 82 && x1 >= 118, "{:?}", boxes[0]);
        assert!(y0 <= 20 && y1 > 60, "{:?}", boxes[0]);
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
