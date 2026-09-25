//! Colour-based skin likelihood in Oklab, used by retouching and skin protection.
//!
//! Skin of every depth sits in one narrow hue band (≈30–75° in Oklab for photographed
//! skin) at modest chroma; melanin mostly changes lightness, not hue. So the detector
//! gates on hue, on chroma and on chroma relative to lightness (which separates skin
//! from teeth, beige walls and paper) and only rejects the near-black, which keeps
//! deep skin tones in. Wood and sand still share the band; pairing this with the
//! Step 3 subject mask is the planned refinement.

use epikos_core::ImageRgbF32;
use rayon::prelude::*;

use super::blur::box_blur;
use super::{long_side, smoothstep};

/// Measured on photographed skin from light to very deep (cheeks, foreheads, necks).
const HUE_CENTER: f32 = 46.0;
/// Narrower towards red (lips), wider towards yellow (lighter, yellower skin).
const HUE_SIGMA_RED: f32 = 18.0;
const HUE_SIGMA_YELLOW: f32 = 22.0;

/// Likelihood 0–1 of one Oklab pixel being skin.
pub(crate) fn skin_pixel(l: f32, a: f32, b: f32) -> f32 {
    let c = a.hypot(b);
    let dh = angle_diff(b.atan2(a).to_degrees(), HUE_CENTER);
    let sigma = if dh < 0.0 { HUE_SIGMA_RED } else { HUE_SIGMA_YELLOW };
    let hue = (-0.5 * (dh / sigma).powi(2)).exp();
    let chroma = smoothstep(0.006, 0.02, c) * (1.0 - smoothstep(0.12, 0.18, c));
    // Skin runs at C/L ≈ 0.06–0.12; teeth, beige walls and paper at ≤ 0.05.
    let colourfulness = smoothstep(0.025, 0.055, c / l.max(0.05));
    let light = smoothstep(0.08, 0.18, l);
    hue * chroma * colourfulness * light
}

/// Skin map of an image whose planes hold Oklab, lightly smoothed so single noisy
/// pixels neither switch retouching on nor punch holes into it.
pub(crate) fn skin_map(lab: &ImageRgbF32) -> Vec<f32> {
    let (w, h) = (lab.width as usize, lab.height as usize);
    let raw: Vec<f32> = (0..lab.len())
        .into_par_iter()
        .map(|i| skin_pixel(lab.r[i], lab.g[i], lab.b[i]))
        .collect();
    let r = (0.002 * long_side(w, h)).round().max(1.0) as usize;
    box_blur(&raw, w, h, r)
}

/// Signed smallest difference between two angles in degrees.
pub(crate) fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::oklab::Oklab;

    fn srgb(r: u8, g: u8, b: u8) -> f32 {
        let lin = |c: u8| {
            let v = c as f32 / 255.0;
            if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        // Rec.709 primaries → Rec.2020.
        let (r, g, b) = (lin(r), lin(g), lin(b));
        let rgb = [
            0.627_404 * r + 0.329_283 * g + 0.043_313 * b,
            0.069_097 * r + 0.919_54 * g + 0.011_362 * b,
            0.016_391 * r + 0.088_013 * g + 0.895_595 * b,
        ];
        let [l, a, b] = Oklab::new().lab_from_rec2020(rgb);
        skin_pixel(l, a, b)
    }

    #[test]
    fn skin_of_every_depth_is_detected() {
        // Photographed skin from light to very deep, including a shine patch.
        for (name, c) in [
            ("light", (224, 172, 150)),
            ("medium", (198, 134, 100)),
            ("tan", (160, 126, 86)),
            ("deep", (110, 68, 48)),
            ("very deep", (62, 40, 32)),
            ("shine on deep", (170, 130, 115)),
        ] {
            let s = srgb(c.0, c.1, c.2);
            assert!(s > 0.5, "{name}: {s}");
        }
    }

    #[test]
    fn scene_colours_are_not_skin() {
        for (name, c) in [
            ("sky", (110, 160, 220)),
            ("foliage", (80, 110, 40)),
            ("grey", (128, 128, 128)),
            ("black", (8, 6, 5)),
            ("saturated orange", (255, 128, 0)),
            ("magenta", (200, 40, 160)),
            ("beige wall", (190, 180, 160)),
            ("teeth", (235, 225, 200)),
        ] {
            let s = srgb(c.0, c.1, c.2);
            assert!(s < 0.1, "{name}: {s}");
        }
    }
}
