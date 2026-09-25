//! Oklab (Björn Ottosson, 2020) for the linear Rec.2020 working space.
//!
//! Steps 4–5 work here: L is perceptually even, so one local-contrast strength reads
//! alike in shadows and highlights, and hue angle is stable under lightness changes,
//! which HSL bands and skin detection rely on. Values above 1 (scene highlights) and
//! out-of-gamut negatives pass through: the cube root is taken with sign.

use epikos_core::ImageRgbF32;
use rayon::prelude::*;

use crate::matrix::{apply, invert, mul, Mat3, REC2020_TO_XYZ};

/// CIE XYZ (D65) → Oklab LMS.
const XYZ_TO_LMS: Mat3 = [
    [0.818_933, 0.361_866_74, -0.128_859_71],
    [0.032_984_544, 0.929_311_9, 0.036_145_64],
    [0.048_200_3, 0.264_366_27, 0.633_851_7],
];
/// Linear sRGB → Oklab LMS (Ottosson's published matrix, for UI colours).
const SRGB_TO_LMS: Mat3 = [
    [0.412_221_46, 0.536_332_55, 0.051_445_995],
    [0.211_903_5, 0.680_699_5, 0.107_396_96],
    [0.088_302_46, 0.281_718_85, 0.629_978_7],
];
/// Non-linear LMS → Lab.
const LMS_TO_LAB: Mat3 = [
    [0.210_454_26, 0.793_617_8, -0.004_072_047],
    [1.977_998_5, -2.428_592_2, 0.450_593_7],
    [0.025_904_037, 0.782_771_77, -0.808_675_77],
];

pub(crate) struct Oklab {
    to_lms: Mat3,
    from_lms: Mat3,
    from_lab: Mat3,
}

impl Oklab {
    pub(crate) fn new() -> Self {
        let to_lms = mul(&XYZ_TO_LMS, &REC2020_TO_XYZ);
        Self {
            to_lms,
            from_lms: invert(&to_lms).expect("Oklab LMS matrix is invertible"),
            from_lab: invert(&LMS_TO_LAB).expect("Oklab Lab matrix is invertible"),
        }
    }

    pub(crate) fn lab_from_rec2020(&self, rgb: [f32; 3]) -> [f32; 3] {
        apply(&LMS_TO_LAB, apply(&self.to_lms, rgb).map(f32::cbrt))
    }

    pub(crate) fn to_rec2020(&self, lab: [f32; 3]) -> [f32; 3] {
        apply(&self.from_lms, apply(&self.from_lab, lab).map(|v| v * v * v))
    }

    /// Planes hold linear Rec.2020 on entry and (L, a, b) on return.
    pub(crate) fn planes_to_lab(&self, img: &mut ImageRgbF32) {
        for_each_pixel(img, |p| self.lab_from_rec2020(p));
    }

    pub(crate) fn planes_to_rec2020(&self, img: &mut ImageRgbF32) {
        for_each_pixel(img, |p| self.to_rec2020(p));
    }
}

fn for_each_pixel(img: &mut ImageRgbF32, f: impl Fn([f32; 3]) -> [f32; 3] + Sync) {
    img.r
        .par_iter_mut()
        .zip(img.g.par_iter_mut())
        .zip(img.b.par_iter_mut())
        .for_each(|((r, g), b)| [*r, *g, *b] = f([*r, *g, *b]));
}

/// Oklab hue angle (degrees) of a fully saturated HSV hue on the usual colour wheel,
/// so wheel pucks drawn in sRGB tint the image in the direction they show.
pub(crate) fn hsv_hue_to_oklab(hue_deg: f32) -> f32 {
    let h = hue_deg.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let srgb = match h as u32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    };
    let linear = srgb.map(|v: f32| {
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    let [_, a, b] = apply(&LMS_TO_LAB, apply(&SRGB_TO_LMS, linear).map(f32::cbrt));
    b.atan2(a).to_degrees().rem_euclid(360.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_greys_have_no_chroma_and_round_trip() {
        let ok = Oklab::new();
        for v in [0.0, 0.01, 0.18, 1.0, 4.0] {
            let [l, a, b] = ok.lab_from_rec2020([v, v, v]);
            assert!(a.abs() < 2e-4 && b.abs() < 2e-4, "{v}: {a} {b}");
            assert!((l - f32::cbrt(v)).abs() < 1e-3, "{v}: L {l}");
            let back = ok.to_rec2020([l, a, b]);
            assert!(back.iter().all(|c| (c - v).abs() < 1e-4 * v.max(1.0)), "{back:?}");
        }
    }

    #[test]
    fn colours_round_trip() {
        let ok = Oklab::new();
        for rgb in [[0.5, 0.2, 0.1], [0.05, 0.3, 0.6], [1.5, 0.9, 0.2], [-0.01, 0.2, 0.3]] {
            let back = ok.to_rec2020(ok.lab_from_rec2020(rgb));
            for (x, y) in back.iter().zip(rgb) {
                assert!((x - y).abs() < 1e-4, "{rgb:?} → {back:?}");
            }
        }
    }

    #[test]
    fn wheel_hues_follow_the_colour_wheel() {
        // Pure sRGB red, yellow, blue in Oklab: ≈29°, ≈110°, ≈264°.
        assert!((hsv_hue_to_oklab(0.0) - 29.2).abs() < 1.0);
        assert!((hsv_hue_to_oklab(60.0) - 109.8).abs() < 1.0);
        assert!((hsv_hue_to_oklab(240.0) - 264.1).abs() < 1.0);
    }
}
