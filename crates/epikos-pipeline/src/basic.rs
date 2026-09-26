//! PRD Step 2 tone controls on scene-linear Rec.2020, right after exposure.
//!
//! Tone moves are gains on luminance, in stops, applied equally to R, G and B so hues
//! don't shift:
//! - **Highlights / Shadows** act on a smoothed log luminance (an edge-aware base), so a
//!   shadow lift brightens a dark region without flattening the texture inside it.
//! - **Whites / Blacks** move the ends of the range per pixel.
//! - **Contrast** steepens (or flattens) log luminance around mid-grey, easing off
//!   towards the extremes so it never clips outright.
//! - **Saturation** scales colour around luminance; **Vibrance** favours muted colours
//!   and spares skin.
//! - **Dehaze** runs first (see [`crate::dehaze`]): haze is a property of the scene
//!   light, so it is removed before any tone move.

use epikos_core::ImageRgbF32;
use epikos_sidecar::Tone;
use rayon::prelude::*;

use crate::dehaze::apply_dehaze;
use crate::look::{guided_plane, skin_likelihood};

const MID_GREY: f32 = 0.18;
/// Stops of gain at ±100 for highlights/shadows and whites/blacks: strong enough that
/// a small move is visible straight away, as in other raw editors.
const REGION_STOPS: f32 = 2.5;
const ENDS_STOPS: f32 = 1.5;
/// Contrast slope at ±100: log-luminance steepens by this factor around mid-grey.
const CONTRAST_SLOPE: f32 = 0.8;

pub fn apply_tone(rgb: &mut ImageRgbF32, t: &Tone) {
    if t.is_neutral() || rgb.width < 2 || rgb.height < 2 {
        return;
    }
    if t.dehaze != 0.0 {
        apply_dehaze(rgb, t.dehaze);
    }
    if (Tone { dehaze: 0.0, ..*t }).is_neutral() {
        return;
    }
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    let n = rgb.len();
    let pct = |v: f32| (v / 100.0).clamp(-1.0, 1.0);
    let (contrast, highlights, shadows) = (pct(t.contrast), pct(t.highlights), pct(t.shadows));
    let (whites, blacks) = (pct(t.whites), pct(t.blacks));

    let luma: Vec<f32> = (0..n)
        .into_par_iter()
        .map(|i| 0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i])
        .collect();
    let ev: Vec<f32> = luma.par_iter().map(|&y| (y.max(1e-6) / MID_GREY).log2()).collect();
    let base = (highlights != 0.0 || shadows != 0.0).then(|| {
        // ~2% of the frame; edges over about a stop stay sharp.
        let r = ((0.02 * w.max(h) as f32).round() as usize).max(2);
        guided_plane(&ev, w, h, r, 1.0)
    });

    let gain: Vec<f32> = (0..n)
        .into_par_iter()
        .map(|i| {
            let e = ev[i];
            let b = base.as_ref().map_or(e, |b| b[i]);
            let mut d = 0.0;
            if shadows != 0.0 {
                d += shadows * REGION_STOPS * (1.0 - smoothstep(-5.0, 0.5, b));
            }
            if highlights != 0.0 {
                d += highlights * REGION_STOPS * smoothstep(-0.5, 2.5, b);
            }
            if whites != 0.0 {
                d += whites * ENDS_STOPS * smoothstep(0.5, 2.5, e);
            }
            if blacks != 0.0 {
                d += blacks * ENDS_STOPS * (1.0 - smoothstep(-6.5, -3.0, e));
            }
            if contrast != 0.0 {
                // Steeper around mid-grey, easing off past ±5 stops.
                d += contrast * CONTRAST_SLOPE * e / (1.0 + (e / 5.0).powi(2));
            }
            if luma[i] <= 0.0 {
                return 1.0;
            }
            2f32.powf(d.clamp(-4.0, 3.0))
        })
        .collect();

    let (vibrance, saturation) = (pct(t.vibrance), pct(t.saturation));
    let skin = (vibrance > 0.0).then(|| skin_likelihood(rgb));
    rgb.r
        .par_iter_mut()
        .zip(rgb.g.par_iter_mut())
        .zip(rgb.b.par_iter_mut())
        .enumerate()
        .for_each(|(i, ((r, g), b))| {
            let k = gain[i];
            let (mut pr, mut pg, mut pb) = (*r * k, *g * k, *b * k);
            if vibrance != 0.0 || saturation != 0.0 {
                let y = luma[i] * k;
                let max = pr.max(pg).max(pb);
                let chroma = if max > 1e-6 { (max - pr.min(pg).min(pb)) / max } else { 0.0 };
                let mut s = 1.0 + saturation;
                if vibrance != 0.0 {
                    let muted = 1.0 - smoothstep(0.0, 0.7, chroma);
                    let spare = skin.as_ref().map_or(1.0, |sk| 1.0 - 0.7 * sk[i]);
                    s *= 1.0 + vibrance * muted * spare;
                }
                let s = s.max(0.0);
                (pr, pg, pb) = (y + (pr - y) * s, y + (pg - y) * s, y + (pb - y) * s);
            }
            (*r, *g, *b) = (pr, pg, pb);
        });
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Suggested exposure and Step 2 settings for an image developed with none: mid-tones
/// to mid-grey (median luminance, ignoring the extremes), shadows lifted when a large
/// part of the frame is deep in shadow, highlights pulled back when they would clip.
/// Skin keeps its own depth: the target is the whole frame's median, not the skin's,
/// so deep skin isn't pushed towards a light-skin exposure.
pub fn auto_tone(rgb: &ImageRgbF32) -> (f32, Tone) {
    let n = rgb.len();
    if n == 0 {
        return (0.0, Tone::default());
    }
    let stride = (n / 200_000).max(1);
    let mut ev: Vec<f32> = (0..n)
        .step_by(stride)
        .map(|i| (0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(1e-6))
        .map(|y| (y / MID_GREY).log2())
        .collect();
    ev.sort_unstable_by(f32::total_cmp);
    let pct = |p: f32| ev[((ev.len() - 1) as f32 * p) as usize];
    // Mid-tones: the mean of the 25th–75th percentiles is steadier than the median.
    let mid = (pct(0.25) + pct(0.5) + pct(0.75)) / 3.0;
    let exposure = (-mid).clamp(-3.0, 3.0);
    let (p02, p995) = (pct(0.02) + exposure, pct(0.995) + exposure);
    let mut tone = Tone::default();
    // Clipping starts at 1.0 linear ≈ +2.47 stops over mid-grey.
    // Slider points are per stop of gain, so these follow REGION_STOPS / ENDS_STOPS.
    let (region, ends) = (100.0 / REGION_STOPS, 100.0 / ENDS_STOPS);
    if p995 > 2.2 {
        // About 0.7 stop of pull per stop past the clipping margin.
        tone.highlights = -((p995 - 2.2) * 0.72 * region).clamp(0.0, 1.28 * region);
    }
    let dark_share = ev.iter().filter(|&&e| e + exposure < -3.0).count() as f32 / ev.len() as f32;
    if dark_share > 0.1 {
        tone.shadows = (dark_share * 1.92 * region).clamp(0.0, 0.96 * region);
    }
    if p02 > -4.0 {
        // Nothing near black: set a black point.
        tone.blacks = -((p02 + 4.0) * 0.12 * ends).clamp(0.0, 0.3 * ends);
    }
    tone.vibrance = 10.0;
    ((exposure * 100.0).round() / 100.0, round_tone(tone))
}

fn round_tone(t: Tone) -> Tone {
    Tone::from_array(t.to_array().map(|v| v.round()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    fn ramp(w: u32, h: u32) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
        for i in 0..img.len() {
            // Two stops per column band, from −7 to +2 around mid-grey.
            let x = (i as u32 % w) as f32 / (w - 1) as f32;
            let v = MID_GREY * 2f32.powf(-7.0 + 9.0 * x);
            (img.r[i], img.g[i], img.b[i]) = (v, 0.8 * v, 0.6 * v);
        }
        img
    }

    fn at(img: &ImageRgbF32, x: u32) -> f32 {
        img.g[img.index(x, 2)]
    }

    #[test]
    fn neutral_tone_is_identity() {
        let mut img = ramp(64, 4);
        let before = img.clone();
        apply_tone(&mut img, &Tone::default());
        assert_eq!(img.r, before.r);
    }

    #[test]
    fn shadows_lift_darks_and_leave_highlights() {
        let before = ramp(64, 4);
        let mut img = before.clone();
        apply_tone(&mut img, &Tone { shadows: 80.0, ..Default::default() });
        assert!(at(&img, 5) > 1.8 * at(&before, 5), "darks {} → {}", at(&before, 5), at(&img, 5));
        assert!((at(&img, 63) / at(&before, 63) - 1.0).abs() < 0.05);
    }

    #[test]
    fn highlights_recover_brights_and_leave_shadows() {
        let before = ramp(64, 4);
        let mut img = before.clone();
        apply_tone(&mut img, &Tone { highlights: -80.0, ..Default::default() });
        assert!(at(&img, 63) < 0.6 * at(&before, 63));
        assert!((at(&img, 3) / at(&before, 3) - 1.0).abs() < 0.05);
    }

    #[test]
    fn contrast_keeps_mid_grey_and_spreads_the_rest() {
        let before = ramp(64, 4);
        let mut img = before.clone();
        apply_tone(&mut img, &Tone { contrast: 60.0, ..Default::default() });
        // Column of mid-grey: −7 + 9x = 0 → x = 7/9.
        let mid = (63.0 * 7.0 / 9.0f32).round() as u32;
        assert!((at(&img, mid) / at(&before, mid) - 1.0).abs() < 0.1);
        assert!(at(&img, 10) < at(&before, 10));
        assert!(at(&img, 63) > at(&before, 63));
    }

    #[test]
    fn saturation_keeps_luminance_and_hue_direction() {
        let before = ramp(16, 2);
        let mut img = before.clone();
        apply_tone(&mut img, &Tone { saturation: 50.0, ..Default::default() });
        let i = img.index(10, 1);
        let y = |im: &ImageRgbF32| 0.2627 * im.r[i] + 0.678 * im.g[i] + 0.0593 * im.b[i];
        assert!((y(&img) - y(&before)).abs() < 1e-4 * y(&before).max(1e-3));
        assert!(img.r[i] - img.b[i] > before.r[i] - before.b[i]);
        let mut grey = before.clone();
        apply_tone(&mut grey, &Tone { saturation: -100.0, ..Default::default() });
        assert!((grey.r[i] - grey.b[i]).abs() < 1e-5);
    }

    #[test]
    fn auto_tone_brings_a_dark_frame_up() {
        let mut img = ramp(64, 4);
        for v in img.r.iter_mut().chain(img.g.iter_mut()).chain(img.b.iter_mut()) {
            *v *= 0.1;
        }
        let (ev, tone) = auto_tone(&img);
        assert!(ev > 1.5, "{ev}");
        assert!(tone.shadows >= 0.0 && tone.highlights <= 0.0);
    }
}
