//! PRD Step 7: creative tone curves and split toning, the last look stage.
//!
//! Curves work on a gamma-2.2 encoding of scene-linear light, which is close to how
//! the tones are seen, through one 4096-entry table per channel (master RGB curve,
//! then the channel's own). Values above 1 (highlights the display shoulder will roll
//! off) continue the curve with slope 1, so nothing clips here.
//!
//! The curve shape is shared with the UI's curve graph (`apps/desktop/src/curve.ts`);
//! keep the two in step.

use epikos_core::ImageRgbF32;
use epikos_sidecar::{Curves, SplitToning, ToneCurve};
use rayon::prelude::*;

use super::oklab::{hsv_hue_to_oklab, Oklab};
use super::smoothstep;

const GAMMA: f32 = 2.2;
const LUT_SIZE: usize = 4096;
/// Largest bend of a region at ±100, in encoded units.
const REGION_AMPLITUDE: f32 = 0.12;
/// Black / white point range at ±100.
const POINT_RANGE: f32 = 0.15;
/// Oklab a/b offset of a split tone at saturation 100.
const SPLIT_CHROMA: f32 = 0.07;

/// Region `i` (1…4) of the degree-5 Bernstein basis, scaled to peak at 1. Zero at
/// both ends, so black and white stay put.
fn region(i: i32, x: f32) -> f32 {
    let binom = [1.0, 5.0, 10.0, 10.0, 5.0, 1.0][i as usize];
    let b = |x: f32| binom * x.powi(i) * (1.0 - x).powi(5 - i);
    let peak = b(i as f32 / 5.0);
    b(x) / peak
}

/// The curve on [0, 1] (encoded in, encoded out). Not yet forced monotonic.
fn shape(c: &ToneCurve, x: f32) -> f32 {
    let k = |v: f32| (v / 100.0).clamp(-1.0, 1.0);
    // Input remap: crush pulls the black point in, a positive white clips earlier.
    let crush = POINT_RANGE * (-k(c.black)).max(0.0);
    let clip = POINT_RANGE * k(c.white).max(0.0);
    let x = ((x - crush) / (1.0 - crush - clip)).clamp(0.0, 1.0);
    let bends = [c.shadows, c.darks, c.lights, c.highlights];
    let mut y = x;
    for (i, v) in bends.iter().enumerate() {
        y += REGION_AMPLITUDE * k(*v) * region(i as i32 + 1, x);
    }
    // Output range: matte lifts the floor, a negative white lowers the ceiling.
    let lo = POINT_RANGE * k(c.black).max(0.0);
    let hi = 1.0 - POINT_RANGE * (-k(c.white)).max(0.0);
    lo + y.clamp(0.0, 1.0) * (hi - lo)
}

/// Monotonic lookup table of `c` on [0, 1].
fn table(c: &ToneCurve) -> Vec<f32> {
    let mut out: Vec<f32> = (0..LUT_SIZE)
        .map(|i| shape(c, i as f32 / (LUT_SIZE - 1) as f32))
        .collect();
    // A curve that turns back on itself would solarise; hold it flat instead.
    for i in 1..out.len() {
        out[i] = out[i].max(out[i - 1]);
    }
    out
}

fn lookup(lut: &[f32], e: f32) -> f32 {
    if e >= 1.0 {
        return lut[LUT_SIZE - 1] + (e - 1.0);
    }
    let x = e.max(0.0) * (LUT_SIZE - 1) as f32;
    let i = x as usize;
    let t = x - i as f32;
    lut[i] * (1.0 - t) + lut[(i + 1).min(LUT_SIZE - 1)] * t
}

pub(crate) fn apply_curves(rgb: &mut ImageRgbF32, curves: &Curves) {
    if curves.is_identity() {
        return;
    }
    let master = table(&curves.rgb);
    for (plane, curve) in [
        (&mut rgb.r, &curves.red),
        (&mut rgb.g, &curves.green),
        (&mut rgb.b, &curves.blue),
    ] {
        let own = table(curve);
        // Compose master then channel into one table.
        let lut: Vec<f32> = master.iter().map(|&m| lookup(&own, m)).collect();
        plane.par_iter_mut().for_each(|v| {
            if *v > 0.0 {
                *v = lookup(&lut, v.powf(1.0 / GAMMA)).max(0.0).powf(GAMMA);
            } else {
                // Out-of-gamut negatives and black: only the floor (matte) applies.
                *v += lookup(&lut, 0.0).powf(GAMMA);
            }
        });
    }
}

/// Planes hold Oklab. Tints are added to a/b, so lightness is kept.
pub(crate) fn apply_split_toning(lab: &mut ImageRgbF32, st: &SplitToning) {
    if st.is_neutral() {
        return;
    }
    let dir = |hue: f32| {
        let h = hsv_hue_to_oklab(hue).to_radians();
        (h.cos(), h.sin())
    };
    let (hd, sd) = (dir(st.highlight_hue), dir(st.shadow_hue));
    let (hs, ss) = (
        SPLIT_CHROMA * (st.highlight_saturation / 100.0).clamp(0.0, 1.0),
        SPLIT_CHROMA * (st.shadow_saturation / 100.0).clamp(0.0, 1.0),
    );
    // Positive balance moves the split down, so more of the image gets the highlight tint.
    let pivot = 0.5 - 0.3 * (st.balance / 100.0).clamp(-1.0, 1.0);
    lab.r
        .par_iter()
        .zip(lab.g.par_iter_mut())
        .zip(lab.b.par_iter_mut())
        .for_each(|((&l, a), b)| {
            let wh = smoothstep(pivot - 0.3, pivot + 0.3, l);
            // Pure black stays black.
            let room = (l / 0.15).clamp(0.0, 1.0);
            *a += room * (wh * hs * hd.0 + (1.0 - wh) * ss * sd.0);
            *b += room * (wh * hs * hd.1 + (1.0 - wh) * ss * sd.1);
        });
}

/// Step 7 on scene-linear Rec.2020: curves, then split toning.
pub(crate) fn apply_tone(rgb: &mut ImageRgbF32, curves: &Curves, st: &SplitToning) {
    apply_curves(rgb, curves);
    if !st.is_neutral() {
        let ok = Oklab::new();
        ok.planes_to_lab(rgb);
        apply_split_toning(rgb, st);
        ok.planes_to_rec2020(rgb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    fn grey(v: f32) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(1, 1, ColorSpace::LinearRec2020);
        (img.r[0], img.g[0], img.b[0]) = (v, v, v);
        img
    }

    #[test]
    fn identity_curve_is_identity() {
        let lut = table(&ToneCurve::default());
        for e in [0.0, 0.1, 0.5, 0.9, 1.0, 1.7] {
            assert!((lookup(&lut, e) - e).abs() < 1e-4, "{e}");
        }
    }

    #[test]
    fn regions_bend_their_own_tones_and_keep_the_ends() {
        let c = ToneCurve {
            shadows: 100.0,
            ..Default::default()
        };
        assert!((shape(&c, 0.2) - 0.32).abs() < 1e-4);
        assert!(shape(&c, 0.8) - 0.8 < 0.01);
        assert_eq!(shape(&c, 0.0), 0.0);
        assert_eq!(shape(&c, 1.0), 1.0);
        let c = ToneCurve {
            highlights: -100.0,
            ..Default::default()
        };
        assert!(shape(&c, 0.8) < 0.7);
    }

    #[test]
    fn matte_lifts_black_and_crush_sinks_shadows() {
        let matte = ToneCurve {
            black: 100.0,
            ..Default::default()
        };
        assert!((shape(&matte, 0.0) - 0.15).abs() < 1e-6);
        let crush = ToneCurve {
            black: -100.0,
            ..Default::default()
        };
        assert_eq!(shape(&crush, 0.1), 0.0);
        let faded = ToneCurve {
            white: -100.0,
            ..Default::default()
        };
        assert!((shape(&faded, 1.0) - 0.85).abs() < 1e-6);
    }

    #[test]
    fn extreme_settings_stay_monotonic() {
        let c = ToneCurve {
            shadows: -100.0,
            darks: 100.0,
            lights: -100.0,
            highlights: 100.0,
            ..Default::default()
        };
        let lut = table(&c);
        assert!(lut.windows(2).all(|w| w[1] >= w[0]));
    }

    #[test]
    fn channel_curve_tints_and_master_keeps_grey_neutral() {
        let mut curves = Curves::default();
        curves.rgb.darks = 50.0;
        let mut img = grey(0.1);
        apply_curves(&mut img, &curves);
        assert!(img.r[0] > 0.1 && img.r[0] == img.g[0] && img.g[0] == img.b[0]);

        curves.blue.lights = 60.0;
        let mut img = grey(0.18);
        apply_curves(&mut img, &curves);
        assert!(img.b[0] > img.g[0], "{} {}", img.b[0], img.g[0]);
    }

    #[test]
    fn split_toning_warms_highlights_and_cools_shadows() {
        let st = SplitToning {
            highlight_saturation: 100.0,
            shadow_saturation: 100.0,
            ..Default::default()
        };
        let mut img = ImageRgbF32::new(2, 1, ColorSpace::LinearRec2020);
        (img.r[0], img.g[0], img.b[0]) = (0.8, 0.8, 0.8);
        (img.r[1], img.g[1], img.b[1]) = (0.02, 0.02, 0.02);
        apply_tone(&mut img, &Curves::default(), &st);
        assert!(img.r[0] > img.b[0] * 1.1, "highlight not warm");
        assert!(img.b[1] > img.r[1] * 1.1, "shadow not cool");
    }
}
