//! The AI Mentor's measured suggestions beyond global tone: skin balance for every
//! skin tone, subject / background separation, and composition (crop and straighten).
//!
//! Skin balance is judged against the range natural skin occupies on the Monk Skin
//! Tone scale, for the skin's own depth, never against one "ideal" skin colour: deep
//! skin that has gone grey or ashy is warmed back, fair skin that has turned pink or
//! orange is calmed, and a correction is only proposed when the skin is outside that
//! range. The correction is found by trying warmth and tint on the skin pixels
//! themselves (with the same local-adjustment code the render uses), so the advice is
//! what the slider will actually do.

use epikos_core::{ColorSpace, ImageRgbF32};
use epikos_pipeline::{apply_local_adjustment, oklab_planes};
use epikos_sidecar::{Crop, LocalAdjustment, MaskTarget};
use serde::Serialize;

/// Median colour of the skin in the current edit.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SkinColour {
    /// Oklab lightness 0–1.
    pub l: f32,
    /// Oklab hue in degrees.
    pub hue: f32,
    /// Oklab chroma.
    pub chroma: f32,
    /// Share of skin pixels at or near clipping, and near black.
    pub clipped: f32,
    pub crushed: f32,
}

/// A proposed skin balance: local warmth, tint and saturation on the Skin mask, and a
/// local exposure when skin detail is lost at either end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SkinFix {
    pub warmth: f32,
    pub tint: f32,
    pub saturation: f32,
    pub exposure: f32,
    pub problem: SkinProblem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkinProblem {
    /// Too little colour: grey, ashy.
    Ashy,
    /// Too much colour: orange, over-warm.
    Orange,
    /// Hue towards magenta / red.
    Magenta,
    /// Hue towards yellow-green.
    Green,
    /// Colour fine; only detail lost at an end.
    Exposure,
}

impl SkinProblem {
    pub fn words(self) -> &'static str {
        match self {
            SkinProblem::Ashy => "grey / ashy (too little colour)",
            SkinProblem::Orange => "orange (too much colour)",
            SkinProblem::Magenta => "pink / magenta",
            SkinProblem::Green => "yellow-green",
            SkinProblem::Exposure => "natural in colour",
        }
    }
}

/// Natural skin, measured on photographed skin from very fair to very deep: Oklab hue
/// 47°–75° and chroma 0.033–0.092, at every depth. The band leaves margin around that;
/// grey / ashy skin measures below 0.015, orange above 0.12, pink below 15°. Very deep
/// skin carries less colour, so its floor is lower.
pub(crate) fn chroma_band(l: f32) -> (f32, f32) {
    let floor = if l < 0.3 { 0.02 } else { 0.028 };
    let ceiling = if l > 0.85 { 0.1 } else { 0.11 };
    (floor, ceiling)
}
const HUE_BAND: (f32, f32) = (40.0, 82.0);

/// How far a skin colour is outside the natural range (0 = inside).
fn skin_cost(l: f32, hue: f32, chroma: f32) -> f32 {
    let (lo, hi) = chroma_band(l);
    let c = (lo - chroma).max(0.0).max(chroma - hi) / 0.01;
    // Hue means little on nearly colourless skin.
    let h = if chroma < 0.012 { 0.0 } else { (HUE_BAND.0 - hue).max(0.0).max(hue - HUE_BAND.1) / 8.0 };
    c + h
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mid = v.len() / 2;
    *v.select_nth_unstable_by(mid, f32::total_cmp).1
}

/// Up to `max` skin pixels (mask ≥ 0.6) of `rgb`, as an N×2 image (each pixel twice:
/// the local-adjustment code needs two rows).
fn skin_sample(rgb: &ImageRgbF32, mask: &[f32], max: usize) -> Option<ImageRgbF32> {
    let on: Vec<usize> = (0..rgb.len()).filter(|&i| mask[i] >= 0.6).collect();
    if on.len() < rgb.len() / 400 || on.len() < 50 {
        return None;
    }
    let step = (on.len() / max).max(1);
    let picked: Vec<usize> = on.into_iter().step_by(step).collect();
    let mut out = ImageRgbF32::new(picked.len() as u32, 2, ColorSpace::LinearRec2020);
    for (k, &i) in picked.iter().chain(&picked).enumerate() {
        (out.r[k], out.g[k], out.b[k]) = (rgb.r[i], rgb.g[i], rgb.b[i]);
    }
    Some(out)
}

fn colour_of(sample: &ImageRgbF32) -> SkinColour {
    let lab = oklab_planes(sample);
    let n = sample.len();
    let mut l = lab.r.clone();
    let mut a = lab.g.clone();
    let mut b = lab.b.clone();
    let clipped = (0..n).filter(|&i| sample.r[i].max(sample.g[i]).max(sample.b[i]) >= 0.97).count() as f32 / n as f32;
    let crushed = (0..n).filter(|&i| lab.r[i] < 0.12).count() as f32 / n as f32;
    let (l, a, b) = (median(&mut l), median(&mut a), median(&mut b));
    SkinColour { l, hue: b.atan2(a).to_degrees().rem_euclid(360.0), chroma: a.hypot(b), clipped, crushed }
}

#[cfg(test)]
/// The skin's colour in `rgb` (scene-linear, the current edit), from the Skin mask.
pub(crate) fn skin_colour(rgb: &ImageRgbF32, mask: &[f32]) -> Option<SkinColour> {
    skin_sample(rgb, mask, 3000).map(|s| colour_of(&s))
}

/// The smallest warmth / tint (and, when detail is lost, exposure) on the Skin mask
/// that brings the skin into the natural range, or `None` when it already is.
pub(crate) fn skin_balance(rgb: &ImageRgbF32, mask: &[f32], current: Option<&LocalAdjustment>) -> Option<SkinFix> {
    let sample = skin_sample(rgb, mask, 1500)?;
    let now = colour_of(&sample);
    let base = skin_cost(now.l, now.hue, now.chroma);
    // Detail lost at the ends: deep skin crushed to black, fair skin clipped.
    let exposure = if now.crushed > 0.25 {
        0.3
    } else if now.clipped > 0.15 {
        -0.3
    } else {
        0.0
    };
    if base < 0.3 {
        return (exposure != 0.0)
            .then_some(SkinFix { warmth: 0.0, tint: 0.0, saturation: 0.0, exposure, problem: SkinProblem::Exposure });
    }
    let (w0, t0, s0) = current.map_or((0.0, 0.0, 0.0), |c| (c.warmth, c.tint, c.saturation));
    let mut best = (base, 0.0f32, 0.0f32, 0.0f32);
    let full = vec![1.0; sample.len()];
    for ds in [-40.0f32, -30.0, -20.0, -10.0, 0.0, 10.0, 20.0, 30.0] {
        for dw in (-8..=8).map(|k| k as f32 * 5.0) {
            for dt in (-8..=8).map(|k| k as f32 * 5.0) {
                if dw == 0.0 && dt == 0.0 && ds == 0.0 {
                    continue;
                }
                let mut s = sample.clone();
                let adj = LocalAdjustment { mask: MaskTarget::Skin, warmth: dw, tint: dt, saturation: ds, ..Default::default() };
                apply_local_adjustment(&mut s, &adj, &full);
                let c = colour_of(&s);
                // Prefer the gentlest correction that works.
                let cost = skin_cost(c.l, c.hue, c.chroma) + 0.004 * (dw.abs() + dt.abs() + ds.abs());
                if cost < best.0 {
                    best = (cost, dw, dt, ds);
                }
            }
        }
    }
    let (cost, dw, dt, ds) = best;
    if cost > 0.6 * base || (dw == 0.0 && dt == 0.0 && ds == 0.0) {
        return None;
    }
    let (lo, hi) = chroma_band(now.l);
    let problem = if now.chroma < lo {
        SkinProblem::Ashy
    } else if now.chroma > hi {
        SkinProblem::Orange
    } else if now.hue < HUE_BAND.0 {
        SkinProblem::Magenta
    } else {
        SkinProblem::Green
    };
    Some(SkinFix {
        warmth: (w0 + dw).clamp(-100.0, 100.0),
        tint: (t0 + dt).clamp(-100.0, 100.0),
        saturation: (s0 + ds).clamp(-100.0, 100.0),
        exposure,
        problem,
    })
}

/// Local Skin warmth, tint and saturation that bring the skin in `rgb` to `hue` and
/// to `richness` (0…1) within the natural chroma range *for this skin's own depth*, so
/// a learned look never moves one skin tone towards another. `None` when it's close.
pub(crate) fn skin_towards(
    rgb: &ImageRgbF32,
    mask: &[f32],
    current: Option<&LocalAdjustment>,
    hue: f32,
    richness: f32,
) -> Option<(f32, f32, f32)> {
    let sample = skin_sample(rgb, mask, 1200)?;
    let now = colour_of(&sample);
    let (lo, hi) = chroma_band(now.l);
    let target_c = lo + richness.clamp(0.0, 1.0) * (hi - lo);
    let cost = |c: &SkinColour| {
        let dh = ((c.hue - hue + 540.0).rem_euclid(360.0) - 180.0).abs();
        dh / 6.0 + (c.chroma - target_c).abs() / 0.006
    };
    let base = cost(&now);
    if base < 0.8 {
        return None;
    }
    let full = vec![1.0; sample.len()];
    let mut best = (base, 0.0f32, 0.0f32, 0.0f32);
    for ds in [-30.0f32, -20.0, -10.0, 0.0, 10.0, 20.0, 30.0] {
        for dw in (-6..=6).map(|k| k as f32 * 5.0) {
            for dt in (-6..=6).map(|k| k as f32 * 5.0) {
                let mut s = sample.clone();
                let adj = LocalAdjustment { mask: MaskTarget::Skin, warmth: dw, tint: dt, saturation: ds, ..Default::default() };
                apply_local_adjustment(&mut s, &adj, &full);
                let c = cost(&colour_of(&s)) + 0.01 * (dw.abs() + dt.abs() + ds.abs()) / 5.0;
                if c < best.0 {
                    best = (c, dw, dt, ds);
                }
            }
        }
    }
    let (c, dw, dt, ds) = best;
    if c > 0.7 * base {
        return None;
    }
    let (w0, t0, s0) = current.map_or((0.0, 0.0, 0.0), |l| (l.warmth, l.tint, l.saturation));
    Some(((w0 + dw).clamp(-100.0, 100.0), (t0 + dt).clamp(-100.0, 100.0), (s0 + ds).clamp(-100.0, 100.0)))
}

/// Mean log luminance (stops) of `rgb` where `weight` is high, and where it's low.
pub(crate) fn inside_outside_ev(rgb: &ImageRgbF32, weight: &[f32]) -> Option<(f32, f32)> {
    let (mut si, mut wi, mut so, mut wo) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (i, w) in weight.iter().enumerate().take(rgb.len()) {
        let y = (0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(1e-5);
        let e = (y / 0.18).log2() as f64;
        let m = w.clamp(0.0, 1.0) as f64;
        si += e * m;
        wi += m;
        so += e * (1.0 - m);
        wo += 1.0 - m;
    }
    let n = rgb.len() as f64;
    (wi > 0.01 * n && wo > 0.1 * n).then(|| ((si / wi) as f32, (so / wo) as f32))
}

/// Mean Oklab b (yellow + / blue −) where `weight` is low: the background's warmth.
pub(crate) fn outside_warmth(rgb: &ImageRgbF32, weight: &[f32]) -> f32 {
    let lab = oklab_planes(rgb);
    let (mut s, mut w) = (0.0f64, 0.0f64);
    for (b, k) in lab.b.iter().zip(weight) {
        let m = 1.0 - k.clamp(0.0, 1.0);
        s += (b * m) as f64;
        w += m as f64;
    }
    if w > 0.0 { (s / w) as f32 } else { 0.0 }
}

/// A suggested crop and straighten, with the reason in words.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CropAdvice {
    pub crop: Crop,
    /// Straighten, degrees (the Step 1 value to use with this crop).
    pub rotation: f32,
    pub reason: String,
}

struct Composition {
    /// Subject centroid and 2–98% extent, fractions of the frame.
    subject: Option<([f32; 2], [f32; 4])>,
    /// Horizon height, fraction of the frame from the top.
    horizon: Option<f32>,
}

fn composition(subject: Option<&[f32]>, sky: Option<&[f32]>, w: usize, h: usize) -> Composition {
    let n = w * h;
    let subject = subject.and_then(|m| {
        let on: Vec<usize> = (0..n).filter(|&i| m[i] > 0.5).collect();
        let share = on.len() as f32 / n as f32;
        if !(0.003..=0.45).contains(&share) {
            return None;
        }
        let mut xs: Vec<f32> = on.iter().map(|&i| (i % w) as f32 / w as f32).collect();
        let mut ys: Vec<f32> = on.iter().map(|&i| (i / w) as f32 / h as f32).collect();
        let c = [xs.iter().sum::<f32>() / xs.len() as f32, ys.iter().sum::<f32>() / ys.len() as f32];
        xs.sort_unstable_by(f32::total_cmp);
        ys.sort_unstable_by(f32::total_cmp);
        let q = |v: &[f32], p: f32| v[((v.len() - 1) as f32 * p) as usize];
        Some((c, [q(&xs, 0.02), q(&ys, 0.02), q(&xs, 0.98), q(&ys, 0.98)]))
    });
    let horizon = sky.and_then(|m| {
        let share = m.iter().filter(|&&v| v > 0.5).count() as f32 / n as f32;
        if !(0.05..=0.8).contains(&share) {
            return None;
        }
        // The sky's lower edge per column (where the column has sky from the top).
        let mut bottoms: Vec<f32> = (0..w)
            .filter(|&x| m[x] > 0.5)
            .map(|x| (0..h).take_while(|&y| m[y * w + x] > 0.5).count() as f32 / h as f32)
            .collect();
        if bottoms.len() < w / 2 {
            return None;
        }
        let mid = median(&mut bottoms);
        let mut dev: Vec<f32> = bottoms.iter().map(|b| (b - mid).abs()).collect();
        // A flat, clear line: buildings or trees breaking it make no horizon.
        (median(&mut dev) < 0.03).then_some(mid)
    });
    Composition { subject, horizon }
}

const THIRDS: [f32; 2] = [1.0 / 3.0, 2.0 / 3.0];

/// How well a crop (x, y, w, h) is composed; lower is better.
fn crop_score(c: &Composition, [x, y, cw, ch]: [f32; 4]) -> f32 {
    // Keep as much of the frame as the composition allows.
    let mut score = 0.6 * (1.0 - (cw * ch).sqrt());
    if let Some(([sx, sy], [l, t, r, b])) = c.subject {
        let (u, v) = ((sx - x) / cw, (sy - y) / ch);
        let d = THIRDS
            .iter()
            .flat_map(|&tx| THIRDS.iter().map(move |&ty| (u - tx).hypot(v - ty)))
            .fold(f32::MAX, f32::min);
        score += d;
        // Never cut into the subject.
        let inside = ((r.min(x + cw) - l.max(x)).max(0.0) * (b.min(y + ch) - t.max(y)).max(0.0))
            / ((r - l) * (b - t)).max(1e-6);
        score += 4.0 * (1.0 - inside);
    }
    if let Some(hy) = c.horizon {
        let v = (hy - y) / ch;
        if (0.0..=1.0).contains(&v) {
            score += 0.5 * THIRDS.iter().map(|t| (v - t).abs()).fold(f32::MAX, f32::min);
        }
    }
    score
}

/// A crop at the frame's own aspect that puts the subject on a third and the horizon
/// on a third line, if it composes clearly better than the full frame.
pub(crate) fn suggest_crop(
    subject: Option<&[f32]>,
    sky: Option<&[f32]>,
    w: usize,
    h: usize,
    rotation: f32,
) -> Option<CropAdvice> {
    let comp = composition(subject, sky, w, h);
    if comp.subject.is_none() && comp.horizon.is_none() {
        return None;
    }
    let full = crop_score(&comp, [0.0, 0.0, 1.0, 1.0]);
    let mut best = (full, [0.0, 0.0, 1.0, 1.0]);
    for s in [0.95f32, 0.9, 0.85, 0.8, 0.75] {
        let mut xs = vec![(1.0 - s) / 2.0, 0.0, 1.0 - s];
        let mut ys = xs.clone();
        if let Some(([sx, sy], _)) = comp.subject {
            for t in THIRDS {
                xs.push(sx - t * s);
                ys.push(sy - t * s);
            }
        }
        if let Some(hy) = comp.horizon {
            ys.extend(THIRDS.map(|t| hy - t * s));
        }
        for &x in &xs {
            for &y in &ys {
                let r = [x.clamp(0.0, 1.0 - s), y.clamp(0.0, 1.0 - s), s, s];
                let score = crop_score(&comp, r);
                if score < best.0 {
                    best = (score, r);
                }
            }
        }
    }
    let (score, [x, y, cw, ch]) = best;
    if score > full - 0.06 || cw >= 1.0 {
        return None;
    }
    let mut why = Vec::new();
    if let Some(([sx, sy], _)) = comp.subject {
        let (u, v) = ((sx - x) / cw, (sy - y) / ch);
        let side = |p: f32, a: &'static str, b: &'static str| if p < 0.5 { a } else { b };
        why.push(format!("puts the subject on the {}-{} third", side(v, "upper", "lower"), side(u, "left", "right")));
    }
    if let Some(hy) = comp.horizon {
        let v = (hy - y) / ch;
        if (0.0..=1.0).contains(&v) {
            why.push(format!("sets the horizon on the {} third line", if v < 0.5 { "upper" } else { "lower" }));
        }
    }
    why.push(format!("keeps {:.0}% of the frame", 100.0 * cw * ch));
    Some(CropAdvice {
        crop: Crop { x, y, width: cw, height: ch, aspect: "original".into() },
        rotation,
        reason: why.join(", "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(rgb: [f32; 3], n: usize) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(n as u32 / 20, 20, ColorSpace::LinearRec2020);
        for i in 0..n {
            (img.r[i], img.g[i], img.b[i]) = (rgb[0], rgb[1], rgb[2]);
        }
        img
    }

    /// A display sRGB colour as linear Rec.2020 (what the pipeline works in).
    fn srgb_to_linear(hex: &str) -> [f32; 3] {
        let c = |i: usize| {
            let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap() as f32 / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        let (r, g, b) = (c(0), c(2), c(4));
        [
            0.6274 * r + 0.3293 * g + 0.0433 * b,
            0.0691 * r + 0.9195 * g + 0.0114 * b,
            0.0164 * r + 0.0880 * g + 0.8956 * b,
        ]
    }

    #[test]
    fn natural_skin_of_every_depth_needs_no_fix() {
        // Photographed skin, fair to deep, under neutral light.
        for hex in ["ffe0bd", "f1c6a7", "eac086", "e0ac8a", "c68863", "8d5a3b", "5c3a24", "3b2417", "2d1b12"] {
            let img = patch(srgb_to_linear(hex), 400);
            let mask = vec![1.0; img.len()];
            let c = skin_colour(&img, &mask).unwrap();
            assert!(skin_cost(c.l, c.hue, c.chroma) < 0.3, "{hex}: {c:?}");
            assert_eq!(skin_balance(&img, &mask, None), None, "{hex}");
        }
    }

    #[test]
    fn ashy_deep_skin_is_warmed_and_pink_fair_skin_is_calmed() {
        // Deep skin gone grey under cool light.
        let ashy = patch(srgb_to_linear("4a4644"), 400);
        let mask = vec![1.0; ashy.len()];
        let fix = skin_balance(&ashy, &mask, None).expect("ashy deep skin needs a fix");
        assert_eq!(fix.problem, SkinProblem::Ashy);
        assert!(fix.warmth > 0.0, "{fix:?}");
        let mut fixed = ashy.clone();
        let adj = LocalAdjustment { mask: MaskTarget::Skin, warmth: fix.warmth, tint: fix.tint, ..Default::default() };
        apply_local_adjustment(&mut fixed, &adj, &mask);
        let c = colour_of(&fixed);
        assert!(skin_cost(c.l, c.hue, c.chroma) < skin_cost(0.4, 60.0, 0.005), "{c:?}");
        // Brightness is kept: the fix changes colour, not depth.
        assert!((c.l - colour_of(&ashy).l).abs() < 0.03);

        // Fair skin turned pink.
        let pink = patch(srgb_to_linear("f5b8c0"), 400);
        let fix = skin_balance(&pink, &mask, None).expect("pink fair skin needs a fix");
        assert_eq!(fix.problem, SkinProblem::Magenta);
        assert!(fix.tint < 0.0, "{fix:?}");

        // Over-warm, orange skin.
        let orange = patch(srgb_to_linear("d07030"), 400);
        let fix = skin_balance(&orange, &mask, None).expect("orange skin needs a fix");
        assert_eq!(fix.problem, SkinProblem::Orange);
        assert!(fix.saturation < 0.0 || fix.warmth < 0.0, "{fix:?}");
    }

    #[test]
    fn crop_puts_a_centred_subject_on_a_third_and_never_cuts_it() {
        let (w, h) = (90, 60);
        // A small subject dead centre, a flat horizon through the middle.
        let subject: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                ((42..48).contains(&x) && (26..34).contains(&y)) as u8 as f32
            })
            .collect();
        let sky: Vec<f32> = (0..w * h).map(|i| (i / w < 30) as u8 as f32).collect();
        let advice = suggest_crop(Some(&subject), Some(&sky), w, h, 0.0).expect("a crop");
        let c = &advice.crop;
        let (u, v) = ((0.5 - c.x) / c.width, (0.5 - c.y) / c.height);
        let off_third = THIRDS.iter().flat_map(|&tx| THIRDS.iter().map(move |&ty| (u - tx).hypot(v - ty))).fold(f32::MAX, f32::min);
        assert!(off_third < 0.12, "subject at {u} {v}");
        assert!(c.x <= 42.0 / 90.0 && c.x + c.width >= 48.0 / 90.0, "{c:?}");
        assert!(advice.reason.contains("third"), "{}", advice.reason);

        // Already well composed: no advice.
        let placed: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                ((28..32).contains(&x) && (18..22).contains(&y)) as u8 as f32
            })
            .collect();
        assert!(suggest_crop(Some(&placed), None, w, h, 0.0).is_none());
    }
}
