//! PRD Step 3 local adjustments: exposure, contrast, saturation, warmth and clarity
//! inside an AI mask, on scene-linear Rec.2020. The mask (0–1, already fitted to the
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
            if saturation != 0.0 {
                let y = 0.2627 * pr + 0.678 * pg + 0.0593 * pb;
                let s = (1.0 + m * saturation).max(0.0);
                (pr, pg, pb) = (y + (pr - y) * s, y + (pg - y) * s, y + (pb - y) * s);
            }
            (*r, *g, *b) = (pr, pg, pb);
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
    fn warmth_warms_and_saturation_desaturates() {
        let mut img = ImageRgbF32::new(4, 4, ColorSpace::LinearRec2020);
        img.r.fill(0.3);
        img.g.fill(0.2);
        img.b.fill(0.1);
        let mask = vec![1.0; 16];
        let mut warm = img.clone();
        apply_local(&mut warm, &LocalAdjustment { warmth: 60.0, ..Default::default() }, &mask);
        assert!(warm.r[0] / warm.b[0] > img.r[0] / img.b[0]);
        let mut grey = img.clone();
        apply_local(&mut grey, &LocalAdjustment { saturation: -100.0, ..Default::default() }, &mask);
        assert!((grey.r[0] - grey.b[0]).abs() < 1e-5);
    }
}
