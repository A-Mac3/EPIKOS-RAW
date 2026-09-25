//! Style-driven light effects for PRD Step 6 (atmospheric glow and haze) on linear
//! Rec.2020. Only styles use these for now; Step 6's own controls (light shafts,
//! depth-based fog) come later.

use epikos_core::ImageRgbF32;
use rayon::prelude::*;

use super::blur::smooth;
use super::{long_side, smoothstep};

/// 0…1 strengths.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct AtmosphereParams {
    /// Soft bloom around bright areas.
    pub glow: f32,
    /// 0 = neutral glow, 1 = low-sun gold.
    pub warmth: f32,
    /// Warm veil: lifted blacks, lower contrast.
    pub haze: f32,
}

impl AtmosphereParams {
    pub(crate) fn is_neutral(&self) -> bool {
        self.glow <= 0.0 && self.haze <= 0.0
    }

    pub(crate) fn scaled(self, k: f32) -> Self {
        Self {
            glow: self.glow * k,
            warmth: self.warmth,
            haze: self.haze * k,
        }
    }
}

/// Low-sun gold in linear Rec.2020 (≈ 2 700 K relative to D65), luminance ≈ 1.
const GOLD: [f32; 3] = [1.45, 0.86, 0.42];
/// Scene-linear level of the haze veil at full strength (mid-grey is 0.18).
const HAZE_LEVEL: f32 = 0.03;

pub(crate) fn apply_atmosphere(rgb: &mut ImageRgbF32, p: &AtmosphereParams) {
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    if p.is_neutral() || w < 4 || h < 4 {
        return;
    }
    let tint = GOLD.map(|g| 1.0 + (g - 1.0) * p.warmth.clamp(0.0, 1.0));

    let bloom = (p.glow > 0.0).then(|| {
        // Bright-pass on luminance, then a wide, soft blur.
        let bright: Vec<f32> = (0..rgb.len())
            .into_par_iter()
            .map(|i| {
                let y = 0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i];
                y.max(0.0) * smoothstep(0.25, 1.0, y)
            })
            .collect();
        let r = (0.02 * long_side(w, h)).round().max(2.0) as usize;
        smooth(&bright, w, h, r, 3)
    });

    let glow = 0.8 * p.glow;
    let keep = 1.0 - 0.15 * p.haze;
    let veil = p.haze * HAZE_LEVEL;
    for (c, plane) in [&mut rgb.r, &mut rgb.g, &mut rgb.b].into_iter().enumerate() {
        plane.par_iter_mut().enumerate().for_each(|(i, v)| {
            let add = bloom.as_ref().map_or(0.0, |b| glow * b[i]);
            *v = *v * keep + (veil + add) * tint[c];
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    #[test]
    fn glow_spreads_light_from_a_bright_spot_and_haze_lifts_black() {
        let n = 200u32;
        let mut img = ImageRgbF32::new(n, n, ColorSpace::LinearRec2020);
        for y in 95..105 {
            for x in 95..105 {
                let i = img.index(x, y);
                (img.r[i], img.g[i], img.b[i]) = (4.0, 4.0, 4.0);
            }
        }
        let near = img.index(110, 100);
        apply_atmosphere(&mut img, &AtmosphereParams { glow: 1.0, warmth: 1.0, haze: 1.0 });
        assert!(img.r[near] > 0.03, "no bloom: {}", img.r[near]);
        assert!(img.r[near] > img.b[near], "bloom not warm");
        let corner = img.index(0, 0);
        assert!(img.g[corner] > 0.02, "black not lifted");
    }
}
