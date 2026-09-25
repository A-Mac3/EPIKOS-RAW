//! PRD Step 5 regional colour, on Oklab planes, through the Step 3 subject mask:
//!
//! - **Foliage shift**: greenery (Oklab hue ≈ 95–175°, with some chroma) is turned
//!   towards teal (−) or autumn gold (+), and its saturation and lightness adjusted.
//!   The subject is excluded when its mask is known, so a green dress stays green.
//! - **Background re-colouration**: everything outside the subject is tinted towards
//!   a hue and its saturation and lightness adjusted. Needs the subject mask.

use epikos_core::ImageRgbF32;
use epikos_sidecar::{BackgroundTint, HslChannel};
use rayon::prelude::*;

use super::oklab::hsv_hue_to_oklab;
use super::smoothstep;

/// Largest foliage hue rotation, degrees.
const FOLIAGE_TURN: f32 = 45.0;
/// Chroma added by a full-strength background tint.
const BG_TINT_CHROMA: f32 = 0.06;

pub(crate) fn foliage_is_neutral(f: &HslChannel) -> bool {
    f.hue == 0.0 && f.saturation == 0.0 && f.luminance == 0.0
}

/// How much a pixel reads as greenery, 0–1.
fn foliage_weight(a: f32, b: f32) -> f32 {
    let c = a.hypot(b);
    let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
    let band = smoothstep(85.0, 105.0, hue) * (1.0 - smoothstep(165.0, 185.0, hue));
    band * smoothstep(0.012, 0.035, c)
}

pub(crate) fn apply_foliage(lab: &mut ImageRgbF32, f: &HslChannel, subject: Option<&[f32]>) {
    if foliage_is_neutral(f) {
        return;
    }
    let pct = |v: f32| (v / 100.0).clamp(-1.0, 1.0);
    // Positive hue = towards gold = smaller Oklab hue angle.
    let (turn, sat, lum) = (-pct(f.hue) * FOLIAGE_TURN.to_radians(), pct(f.saturation), pct(f.luminance));
    lab.r
        .par_iter_mut()
        .zip(lab.g.par_iter_mut())
        .zip(lab.b.par_iter_mut())
        .enumerate()
        .for_each(|(i, ((l, a), b))| {
            let w = foliage_weight(*a, *b) * subject.map_or(1.0, |s| 1.0 - s[i]);
            if w <= 0.0 {
                return;
            }
            let (sin, cos) = (turn * w).sin_cos();
            let k = (1.0 + sat * w).max(0.0);
            let (na, nb) = ((*a * cos - *b * sin) * k, (*a * sin + *b * cos) * k);
            (*a, *b) = (na, nb);
            // Lightness scaled, so black stays black.
            *l *= 1.0 + 0.35 * lum * w;
        });
}

pub(crate) fn apply_background(lab: &mut ImageRgbF32, t: &BackgroundTint, subject: &[f32]) {
    if t.is_neutral() || subject.len() != lab.len() {
        return;
    }
    let pct = |v: f32| (v / 100.0).clamp(-1.0, 1.0);
    let hue = hsv_hue_to_oklab(t.hue).to_radians();
    let push = (t.amount / 100.0).clamp(0.0, 1.0) * BG_TINT_CHROMA;
    let (da, db) = (push * hue.cos(), push * hue.sin());
    let (sat, lum) = (pct(t.saturation), pct(t.luminance));
    lab.r
        .par_iter_mut()
        .zip(lab.g.par_iter_mut())
        .zip(lab.b.par_iter_mut())
        .enumerate()
        .for_each(|(i, ((l, a), b))| {
            let w = 1.0 - subject[i];
            if w <= 0.0 {
                return;
            }
            let k = (1.0 + sat * w).max(0.0);
            // Tint scaled by lightness so shadows aren't lifted into colour.
            let depth = smoothstep(0.0, 0.5, *l);
            *a = *a * k + da * w * depth;
            *b = *b * k + db * w * depth;
            *l *= 1.0 + 0.4 * lum * w;
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    /// Left: foliage green; right: skin orange (Oklab).
    fn scene() -> ImageRgbF32 {
        let mut lab = ImageRgbF32::new(4, 1, ColorSpace::LinearRec2020);
        let green = (0.55f32, 140f32.to_radians());
        let orange = (0.65f32, 55f32.to_radians());
        for (i, (l, h)) in [green, green, orange, orange].into_iter().enumerate() {
            (lab.r[i], lab.g[i], lab.b[i]) = (l, 0.08 * h.cos(), 0.08 * h.sin());
        }
        lab
    }

    #[test]
    fn foliage_shift_turns_greens_towards_gold_and_leaves_skin() {
        let mut lab = scene();
        let before = lab.clone();
        apply_foliage(&mut lab, &HslChannel { hue: 100.0, saturation: 0.0, luminance: 0.0 }, None);
        let hue = |im: &ImageRgbF32, i: usize| im.b[i].atan2(im.g[i]).to_degrees();
        assert!(hue(&lab, 0) < hue(&before, 0) - 30.0, "{} → {}", hue(&before, 0), hue(&lab, 0));
        assert_eq!((lab.g[2], lab.b[2]), (before.g[2], before.b[2]));
        // Masked as subject: untouched.
        let mut masked = before.clone();
        apply_foliage(&mut masked, &HslChannel { hue: 100.0, ..Default::default() }, Some(&[1.0, 0.0, 0.0, 0.0]));
        assert_eq!(masked.g[0], before.g[0]);
        assert_ne!(masked.g[1], before.g[1]);
    }

    #[test]
    fn background_tint_skips_the_subject() {
        let mut lab = scene();
        let before = lab.clone();
        let t = BackgroundTint { hue: 220.0, amount: 100.0, saturation: 0.0, luminance: -50.0 };
        apply_background(&mut lab, &t, &[0.0, 0.0, 1.0, 1.0]);
        assert!(lab.r[0] < before.r[0]);
        assert_eq!((lab.r[2], lab.g[2], lab.b[2]), (before.r[2], before.g[2], before.b[2]));
    }
}
