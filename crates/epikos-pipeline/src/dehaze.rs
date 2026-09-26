//! PRD Step 2 Dehaze: removes (or adds) atmospheric haze with the dark-channel prior
//! (He, Sun & Tang 2009) on scene-linear Rec.2020.
//!
//! Haze adds the airlight `A` to every surface in proportion to its distance:
//! `I = J·t + A·(1 − t)`. In a clear photo most small patches have a channel near zero
//! (shadow or saturated colour); haze lifts that dark channel, so it estimates `1 − t`.
//! The estimate is made on a small copy, where the patch minimum is cheap, then fitted
//! to the full image's edges with a joint guided filter so no halos form around
//! silhouettes. Negative amounts add a veil of the same airlight instead.
//!
//! Haze also flattens contrast and colour that the model alone doesn't give back in a
//! photo with little haze, so a mid-tone contrast and saturation lift (weighted towards
//! the hazier parts) makes the slider respond on every photo, as in other raw editors.

use epikos_core::{resize_plane, ImageRgbF32};
use rayon::prelude::*;

use crate::look::guided_joint_plane;

/// Long side of the copy the dark channel is measured on.
const WORK_SIDE: usize = 384;
/// Transmission floor: dense haze is thinned, never divided to noise.
const T_MIN: f32 = 0.2;

/// `amount`: −100…100.
pub fn apply_dehaze(rgb: &mut ImageRgbF32, amount: f32) {
    let k = (amount / 100.0).clamp(-1.0, 1.0);
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    if k == 0.0 || w < 4 || h < 4 {
        return;
    }

    // Small copy.
    let scale = (WORK_SIDE as f32 / w.max(h) as f32).min(1.0);
    let (sw, sh) = (((w as f32 * scale).round() as usize).max(2), ((h as f32 * scale).round() as usize).max(2));
    let small = |p: &[f32]| resize_plane(p, w as u32, h as u32, sw as u32, sh as u32);
    let (r, g, b) = (small(&rgb.r), small(&rgb.g), small(&rgb.b));

    // Airlight: the colour of the haziest (highest dark channel) 0.2% of the frame.
    let min_rgb: Vec<f32> = (0..sw * sh).map(|i| r[i].min(g[i]).min(b[i]).max(0.0)).collect();
    let patch = ((sw.max(sh) as f32 * 0.02).round() as usize).max(1);
    let dark = min_filter(&min_rgb, sw, sh, patch);
    let mut order: Vec<usize> = (0..dark.len()).collect();
    let top = (dark.len() / 500).max(1);
    order.select_nth_unstable_by(top, |&x, &y| dark[y].total_cmp(&dark[x]));
    let mut air = [0.0f32; 3];
    for &i in &order[..top] {
        air[0] += r[i];
        air[1] += g[i];
        air[2] += b[i];
    }
    let air = air.map(|v| (v / top as f32).max(1e-3));

    // Haze density 0…1 per pixel: the dark channel of the image normalised by the
    // airlight, at full size, snapped to the photo's edges.
    let norm_small: Vec<f32> = (0..sw * sh)
        .map(|i| (r[i] / air[0]).min(g[i] / air[1]).min(b[i] / air[2]).clamp(0.0, 1.0))
        .collect();
    let haze_small = min_filter(&norm_small, sw, sh, patch);
    let haze = resize_plane(&haze_small, sw as u32, sh as u32, w as u32, h as u32);
    let guide: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|i| (0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(0.0).powf(1.0 / 2.2))
        .collect();
    let radius = ((w.max(h) as f32 * 0.01).round() as usize).max(2);
    let haze = guided_joint_plane(&guide, &haze, w, h, radius, 1e-3);
    drop(guide);

    rgb.r
        .par_iter_mut()
        .zip(rgb.g.par_iter_mut())
        .zip(rgb.b.par_iter_mut())
        .zip(haze.par_iter())
        .for_each(|(((r, g), b), &d)| {
            let d = d.clamp(0.0, 1.0);
            let mut px = [*r, *g, *b];
            if k > 0.0 {
                // Remove up to 95% of the haze at +100.
                let t = (1.0 - 0.95 * k * d).max(T_MIN);
                for (v, a) in px.iter_mut().zip(air) {
                    *v = (*v - a) / t + a;
                }
            } else {
                // Add a veil, thicker where the scene already reads as far or hazy.
                let veil = -k * 0.6 * (0.35 + 0.65 * d);
                for (v, a) in px.iter_mut().zip(air) {
                    *v = *v * (1.0 - veil) + a * veil;
                }
            }
            // Contrast (in stops around mid-grey, easing off at the ends) and colour.
            let weight = k * (0.5 + 0.5 * d);
            let y = (0.2627 * px[0] + 0.678 * px[1] + 0.0593 * px[2]).max(1e-6);
            let e = (y / 0.18).log2();
            let gain = 2f32.powf(0.35 * weight * e / (1.0 + (e / 5.0).powi(2)));
            let sat = (1.0 + 0.3 * weight).max(0.0);
            let y2 = y * gain;
            for v in px.iter_mut() {
                *v = y2 + (*v * gain - y2) * sat;
            }
            (*r, *g, *b) = (px[0], px[1], px[2]);
        });
}

/// Minimum over a (2r+1)² window, separable (small copies only).
fn min_filter(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut rows = vec![0.0; src.len()];
    rows.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let s = &src[y * w..(y + 1) * w];
        for (x, o) in row.iter_mut().enumerate() {
            *o = s[x.saturating_sub(r)..(x + r + 1).min(w)].iter().copied().fold(f32::INFINITY, f32::min);
        }
    });
    let mut out = vec![0.0; src.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(h));
        for (x, o) in row.iter_mut().enumerate() {
            *o = (y0..y1).map(|yy| rows[yy * w + x]).fold(f32::INFINITY, f32::min);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    /// Left half near, right half far: the far half is washed towards a grey airlight.
    fn hazy() -> ImageRgbF32 {
        let (w, h) = (64u32, 32u32);
        let mut img = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
        for i in 0..img.len() {
            let x = i as u32 % w;
            let y = i as u32 / w;
            // A textured scene of saturated colours.
            let j = [(0.30, 0.05, 0.02), (0.02, 0.2, 0.05), (0.03, 0.05, 0.25)][((x / 4 + y / 4) % 3) as usize];
            let t = if x < w / 2 { 0.9 } else { 0.35 };
            let air = 0.7;
            (img.r[i], img.g[i], img.b[i]) = (j.0 * t + air * (1.0 - t), j.1 * t + air * (1.0 - t), j.2 * t + air * (1.0 - t));
        }
        img
    }

    fn contrast(img: &ImageRgbF32, x0: u32, x1: u32) -> f32 {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for y in 4..28 {
            for x in x0..x1 {
                let i = img.index(x, y);
                let m = img.r[i].min(img.g[i]).min(img.b[i]);
                lo = lo.min(m);
                hi = hi.max(img.r[i].max(img.g[i]).max(img.b[i]));
            }
        }
        hi - lo
    }

    #[test]
    fn dehaze_restores_contrast_where_it_is_hazy() {
        let before = hazy();
        let mut img = before.clone();
        apply_dehaze(&mut img, 80.0);
        let gain = |x0, x1| contrast(&img, x0, x1) / contrast(&before, x0, x1);
        assert!(gain(40, 60) > 1.5, "far half contrast ×{}", gain(40, 60));
        assert!(gain(40, 60) > gain(4, 24), "far {} vs near {}", gain(40, 60), gain(4, 24));
        assert!(img.r.iter().chain(&img.g).chain(&img.b).all(|v| v.is_finite()));
    }

    #[test]
    fn negative_dehaze_adds_haze_and_zero_is_identity() {
        let before = hazy();
        let mut img = before.clone();
        apply_dehaze(&mut img, 0.0);
        assert_eq!(img.g, before.g);
        apply_dehaze(&mut img, -80.0);
        assert!(contrast(&img, 4, 24) < contrast(&before, 4, 24));
    }
}
