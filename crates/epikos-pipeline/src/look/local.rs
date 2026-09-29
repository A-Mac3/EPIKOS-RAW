//! PRD Step 3 local adjustments: exposure, contrast, highlights, shadows, whites,
//! blacks, saturation, warmth, tint and clarity inside an AI mask, on scene-linear Rec.2020. The mask (0–1, already fitted to the
//! photo's edges) scales every change, so soft mask edges give soft transitions.

use epikos_core::ImageRgbF32;
use epikos_sidecar::LocalAdjustment;
use rayon::prelude::*;

use super::atmosphere::tint;
use super::blur::smooth;
use super::long_side;

const MID_GREY: f32 = 0.18;

pub(crate) fn apply_local(rgb: &mut ImageRgbF32, adj: &LocalAdjustment, mask: &[f32]) {
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    if adj.is_neutral() || mask.len() != rgb.len() || w < 2 || h < 2 {
        return;
    }
    let pct = |v: f32| (v / 100.0).clamp(-1.0, 1.0);
    let (contrast, saturation, warmth, clarity) =
        (pct(adj.contrast), pct(adj.saturation), pct(adj.warmth), pct(adj.clarity));
    let exposure = adj.exposure.clamp(-3.0, 3.0);
    let (highlights, shadows, whites, blacks) =
        (pct(adj.highlights), pct(adj.shadows), pct(adj.whites), pct(adj.blacks));
    let ranges = highlights != 0.0 || shadows != 0.0 || whites != 0.0 || blacks != 0.0;

    let ev: Vec<f32> = (0..rgb.len())
        .into_par_iter()
        .map(|i| ((0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(1e-6) / MID_GREY).log2())
        .collect();
    // Clarity: log-luminance detail against a ~1% blur.
    let detail = (clarity != 0.0).then(|| {
        let r = ((0.01 * long_side(w, h)).round() as usize).max(2);
        let base = smooth(&ev, w, h, r, 2);
        ev.iter().zip(&base).map(|(e, b)| e - b).collect::<Vec<f32>>()
    });
    // Tint at full warmth, blended per pixel by the mask.
    let hue = tint(0.5 * warmth);
    // Green (−) / magenta (+) at full strength, luminance kept.
    let magenta = 0.15 * pct(adj.tint);
    let tint_gain = {
        let g = [1.0 + magenta, 1.0 - magenta, 1.0 + magenta];
        let y = 0.2627 * g[0] + 0.678 * g[1] + 0.0593 * g[2];
        g.map(|v| v / y)
    };

    rgb.r
        .par_iter_mut()
        .zip(rgb.g.par_iter_mut())
        .zip(rgb.b.par_iter_mut())
        .enumerate()
        .for_each(|(i, ((r, g), b))| {
            let m = mask[i];
            if m <= 0.0 {
                return;
            }
            let e = ev[i];
            let mut stops = exposure;
            if contrast != 0.0 {
                stops += contrast * 0.5 * e / (1.0 + (e / 5.0).powi(2));
            }
            if ranges {
                // Stops from mid-grey: display white sits near +2.5, deep black near −5.
                // Each range fades in smoothly, so its edges never band.
                stops += highlights * smoothstep(-0.5, 2.0, e)
                    + shadows * 1.2 * (1.0 - smoothstep(-3.0, 0.5, e))
                    + whites * 0.7 * smoothstep(1.2, 2.8, e)
                    + blacks * 0.8 * (1.0 - smoothstep(-5.5, -2.5, e));
            }
            if let Some(d) = &detail {
                stops += clarity * 0.8 * d[i].clamp(-2.0, 2.0);
            }
            let k = 2f32.powf((m * stops).clamp(-4.0, 4.0));
            let (mut pr, mut pg, mut pb) = (*r * k, *g * k, *b * k);
            if warmth != 0.0 {
                pr *= 1.0 + m * (hue[0] - 1.0);
                pg *= 1.0 + m * (hue[1] - 1.0);
                pb *= 1.0 + m * (hue[2] - 1.0);
            }
            if magenta != 0.0 {
                pr *= 1.0 + m * (tint_gain[0] - 1.0);
                pg *= 1.0 + m * (tint_gain[1] - 1.0);
                pb *= 1.0 + m * (tint_gain[2] - 1.0);
            }
            if saturation != 0.0 {
                let y = 0.2627 * pr + 0.678 * pg + 0.0593 * pb;
                let s = (1.0 + m * saturation).max(0.0);
                (pr, pg, pb) = (y + (pr - y) * s, y + (pg - y) * s, y + (pb - y) * s);
            }
            (*r, *g, *b) = (pr, pg, pb);
        });
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Teeth whitening through the Teeth mask: the yellow / orange cast is pulled towards
/// neutral (blue lifted towards the red–green level) and the teeth brighten a little,
/// most on their highlights. `amount` 0…100.
pub(crate) fn whiten_teeth(rgb: &mut ImageRgbF32, amount: f32, mask: &[f32]) {
    let k = (amount / 100.0).clamp(0.0, 1.0);
    if k == 0.0 || mask.len() != rgb.len() {
        return;
    }
    rgb.r
        .par_iter_mut()
        .zip(rgb.g.par_iter_mut())
        .zip(rgb.b.par_iter_mut())
        .zip(mask.par_iter())
        .for_each(|(((r, g), b), &m)| {
            if m <= 0.0 {
                return;
            }
            let w = k * m;
            // Yellow / orange: blue below the smaller of red and green.
            let yellow = (r.min(*g) - *b).max(0.0);
            *b += 0.85 * w * yellow;
            let y = 0.2627 * *r + 0.678 * *g + 0.0593 * *b;
            let lift = 1.0 + w * (0.1 + 0.12 * (y / 0.5).clamp(0.0, 1.0));
            (*r, *g, *b) = (*r * lift, *g * lift, *b * lift);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;
    use epikos_sidecar::MaskTarget;

    #[test]
    fn exposure_acts_only_inside_the_mask() {
        let mut img = ImageRgbF32::new(8, 4, ColorSpace::LinearRec2020);
        img.r.fill(0.2);
        img.g.fill(0.2);
        img.b.fill(0.2);
        let mask: Vec<f32> = (0..32).map(|i| if i % 8 < 4 { 1.0 } else { 0.0 }).collect();
        let adj = LocalAdjustment { mask: MaskTarget::Subject, exposure: 1.0, ..Default::default() };
        apply_local(&mut img, &adj, &mask);
        assert!((img.g[0] - 0.4).abs() < 1e-5);
        assert!((img.g[7] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn tonal_ranges_move_their_own_tones() {
        // Dark, mid and bright patches.
        let levels = [0.01f32, 0.18, 0.9];
        let mut img = ImageRgbF32::new(3, 2, ColorSpace::LinearRec2020);
        for i in 0..6 {
            let v = levels[i % 3];
            (img.r[i], img.g[i], img.b[i]) = (v, v, v);
        }
        let mask = vec![1.0; 6];
        let run = |adj: LocalAdjustment| {
            let mut out = img.clone();
            apply_local(&mut out, &adj, &mask);
            out.g
        };
        let s = run(LocalAdjustment { shadows: 100.0, ..Default::default() });
        assert!(s[0] > levels[0] * 1.8 && (s[2] / levels[2] - 1.0).abs() < 0.05, "shadows lift darks: {s:?}");
        let h = run(LocalAdjustment { highlights: -100.0, ..Default::default() });
        assert!(h[2] < levels[2] * 0.7 && (h[0] / levels[0] - 1.0).abs() < 0.05, "highlights pull brights: {h:?}");
        let b = run(LocalAdjustment { blacks: -100.0, ..Default::default() });
        assert!(b[0] < levels[0] && (b[1] / levels[1] - 1.0).abs() < 0.02, "blacks deepen: {b:?}");
        let w = run(LocalAdjustment { whites: 100.0, ..Default::default() });
        assert!(w[2] > levels[2] && (w[1] / levels[1] - 1.0).abs() < 0.02, "whites lift: {w:?}");
    }

    #[test]
    fn warmth_warms_and_saturation_desaturates() {
        let mut img = ImageRgbF32::new(4, 4, ColorSpace::LinearRec2020);
        img.r.fill(0.3);
        img.g.fill(0.2);
        img.b.fill(0.1);
        let mask = vec![1.0; 16];
        let mut warm = img.clone();
        apply_local(&mut warm, &LocalAdjustment { warmth: 60.0, ..Default::default() }, &mask);
        assert!(warm.r[0] / warm.b[0] > img.r[0] / img.b[0]);
        let mut magenta = img.clone();
        apply_local(&mut magenta, &LocalAdjustment { tint: 50.0, ..Default::default() }, &mask);
        assert!(magenta.g[0] / magenta.r[0] < img.g[0] / img.r[0]);
        let y = |i: &ImageRgbF32| 0.2627 * i.r[0] + 0.678 * i.g[0] + 0.0593 * i.b[0];
        assert!((y(&magenta) / y(&img) - 1.0).abs() < 0.02, "tint keeps brightness");
        // Teeth whitening: less yellow, a touch brighter.
        let mut teeth = img.clone();
        whiten_teeth(&mut teeth, 100.0, &mask);
        assert!(teeth.b[0] / teeth.r[0] > img.b[0] / img.r[0], "less yellow");
        let mut grey = img.clone();
        apply_local(&mut grey, &LocalAdjustment { saturation: -100.0, ..Default::default() }, &mask);
        assert!((grey.r[0] - grey.b[0]).abs() < 1e-5);
    }
}
