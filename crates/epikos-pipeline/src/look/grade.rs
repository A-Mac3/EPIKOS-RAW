//! PRD Step 5: base colour grading in Oklab LCh.
//!
//! One pass = HSL bands → saturation / vibrance → contrast → (monochrome) → colour
//! wheels. Each pass carries a per-pixel weight, which is how skin protection works:
//! the change a pass makes is scaled by `1 − protection × skin`. Monochrome conversion
//! is exempt, so a black-and-white style never leaves colour faces behind.

use super::oklab::hsv_hue_to_oklab;
use super::skin::angle_diff;
use super::smoothstep;

/// Oklab hue (degrees) at the centre of each HSL band: sRGB red, orange, yellow, a
/// foliage green, aqua, blue, purple and magenta.
const BAND_CENTERS: [f32; 8] = [29.0, 55.0, 110.0, 135.0, 195.0, 264.0, 294.0, 328.0];
/// Hue shift at ±100.
const MAX_HUE_SHIFT: f32 = 30.0;
/// Oklab a/b offset of a colour wheel at amount 100.
const WHEEL_CHROMA: f32 = 0.06;
/// Oklab L offset of a wheel's luminance at ±100.
const WHEEL_LIGHTNESS: f32 = 0.1;

/// Normalised grade: HSL and wheel luminance −1…1, wheel amounts 0…1, hues in degrees.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct ColorParams {
    /// Per band: hue shift, saturation, luminance.
    pub hsl: [[f32; 3]; 8],
    /// Shadows, midtones, highlights: HSV hue (degrees), amount, luminance.
    pub wheels: [[f32; 3]; 3],
    pub saturation: f32,
    /// Saturation that favours muted colours.
    pub vibrance: f32,
    /// S-curve on L around mid-grey, −1…1.
    pub contrast: f32,
    /// 0…1 conversion to black and white (after HSL, so HSL luminance is the mixer).
    pub mono: f32,
}

impl ColorParams {
    pub(crate) fn is_neutral(&self) -> bool {
        self.hsl.iter().flatten().all(|v| *v == 0.0)
            && self.wheels.iter().all(|w| w[1] == 0.0 && w[2] == 0.0)
            && self.saturation == 0.0
            && self.vibrance == 0.0
            && self.contrast == 0.0
            && self.mono == 0.0
    }

    /// Every strength times `k`; wheel hues stay put.
    pub(crate) fn scaled(mut self, k: f32) -> Self {
        self.hsl.iter_mut().flatten().for_each(|v| *v *= k);
        for w in &mut self.wheels {
            w[1] *= k;
            w[2] *= k;
        }
        self.saturation *= k;
        self.vibrance *= k;
        self.contrast *= k;
        self.mono = (self.mono * k).clamp(0.0, 1.0);
        self
    }

    /// Sum of already-scaled grades (a style fusion). Wheel tints add as vectors on
    /// the colour wheel, so warm + cool partly cancel instead of averaging hues.
    pub(crate) fn sum(parts: &[ColorParams]) -> Self {
        let mut out = ColorParams::default();
        let mut tint = [(0.0f32, 0.0f32); 3];
        for p in parts {
            for (o, v) in out.hsl.iter_mut().flatten().zip(p.hsl.iter().flatten()) {
                *o += v;
            }
            for (i, w) in p.wheels.iter().enumerate() {
                let h = w[0].to_radians();
                tint[i].0 += w[1] * h.cos();
                tint[i].1 += w[1] * h.sin();
                out.wheels[i][2] += w[2];
            }
            out.saturation += p.saturation;
            out.vibrance += p.vibrance;
            out.contrast += p.contrast;
            out.mono += p.mono;
        }
        for (w, (x, y)) in out.wheels.iter_mut().zip(tint) {
            w[0] = y.atan2(x).to_degrees().rem_euclid(360.0);
            w[1] = x.hypot(y).min(1.0);
        }
        out.mono = out.mono.clamp(0.0, 1.0);
        out
    }

    pub(crate) fn prepare(&self) -> Prepared {
        Prepared {
            p: *self,
            dirs: self.wheels.map(|w| {
                let h = hsv_hue_to_oklab(w[0]).to_radians();
                (h.cos(), h.sin())
            }),
        }
    }
}

/// [`ColorParams`] with wheel directions resolved once per image.
pub(crate) struct Prepared {
    p: ColorParams,
    dirs: [(f32, f32); 3],
}

impl Prepared {
    /// Grade one Oklab pixel. `weight` scales everything but monochrome conversion.
    pub(crate) fn apply(&self, [l0, a0, b0]: [f32; 3], weight: f32) -> [f32; 3] {
        let p = &self.p;
        let (mut l, mut a, mut b) = (l0, a0, b0);

        // HSL bands.
        let c = a.hypot(b);
        let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
        let (i, j, t) = band_pair(hue);
        let mix = |k: usize| p.hsl[i][k] * (1.0 - t) + p.hsl[j][k] * t;
        // Hue of near-neutral pixels is noise: hue and luminance changes fade out there.
        let gate = smoothstep(0.005, 0.05, c);
        let (dh, ds, dl) = (mix(0) * MAX_HUE_SHIFT * gate, mix(1), mix(2) * gate);
        let mut chroma = c * (1.0 + ds).max(0.0);
        // Vibrance: strongest on muted colours, little on already saturated ones.
        chroma *= (1.0 + p.saturation).max(0.0)
            * (1.0 + p.vibrance * (1.0 - smoothstep(0.0, 0.2, chroma))).max(0.0);
        let h = (hue + dh).to_radians();
        a = chroma * h.cos();
        b = chroma * h.sin();
        l *= 2f32.powf(0.7 * dl);

        if p.contrast != 0.0 && (0.0..=1.0).contains(&l) {
            // Fixed ends, pivot 0.5; monotonic for |k| ≤ 0.5.
            let k = 0.45 * p.contrast;
            l += k * (l - 0.5) * 4.0 * l * (1.0 - l);
        }
        let (l, a, b) = blend((l0, a0, b0), (l, a, b), weight);

        // Monochrome: always full strength.
        let (a, b) = (a * (1.0 - p.mono), b * (1.0 - p.mono));

        // Colour wheels, weighted by tonal range (Bernstein basis: sums to 1).
        let t = l.clamp(0.0, 1.0);
        let tones = [(1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t];
        let (mut l2, mut a2, mut b2) = (l, a, b);
        // Tinting pure black would create out-of-gamut colour: fade in from black.
        let tint_room = (l / 0.2).clamp(0.0, 1.0);
        for ((w, (dx, dy)), tone) in p.wheels.iter().zip(self.dirs).zip(tones) {
            a2 += tone * w[1] * WHEEL_CHROMA * dx * tint_room;
            b2 += tone * w[1] * WHEEL_CHROMA * dy * tint_room;
            l2 += tone * w[2] * WHEEL_LIGHTNESS;
        }
        let (l, a, b) = blend((l, a, b), (l2, a2, b2), weight);
        [l.max(0.0), a, b]
    }
}

fn blend(from: (f32, f32, f32), to: (f32, f32, f32), w: f32) -> (f32, f32, f32) {
    (
        from.0 + (to.0 - from.0) * w,
        from.1 + (to.1 - from.1) * w,
        from.2 + (to.2 - from.2) * w,
    )
}

/// The two bands around `hue` and the smoothed position between them (0 = first).
fn band_pair(hue: f32) -> (usize, usize, f32) {
    let n = BAND_CENTERS.len();
    for (i, &center) in BAND_CENTERS.iter().enumerate() {
        let j = (i + 1) % n;
        let span = angle_diff(BAND_CENTERS[j], center).rem_euclid(360.0);
        let off = (hue - center).rem_euclid(360.0);
        if off < span {
            let t = off / span;
            return (i, j, t * t * (3.0 - 2.0 * t));
        }
    }
    (0, 0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab_of_hue(h: f32, c: f32) -> [f32; 3] {
        let r = h.to_radians();
        [0.6, c * r.cos(), c * r.sin()]
    }

    fn hue_of(p: [f32; 3]) -> f32 {
        p[2].atan2(p[1]).to_degrees().rem_euclid(360.0)
    }

    #[test]
    fn neutral_params_are_identity() {
        let g = ColorParams::default().prepare();
        for p in [[0.5, 0.05, -0.02], [0.1, 0.0, 0.0], [1.3, -0.1, 0.1]] {
            let out = g.apply(p, 1.0);
            for (x, y) in out.iter().zip(p) {
                assert!((x - y).abs() < 1e-6, "{p:?} → {out:?}");
            }
        }
    }

    #[test]
    fn bands_partition_the_hue_circle() {
        for h in (0..360).map(|d| d as f32) {
            let (i, j, t) = band_pair(h);
            assert!(j == (i + 1) % 8 && (0.0..=1.0).contains(&t), "{h}: {i} {j} {t}");
        }
        assert_eq!(band_pair(135.0).0, 3);
        assert_eq!(band_pair(10.0), (7, 0, band_pair(10.0).2));
    }

    #[test]
    fn hsl_touches_only_its_band() {
        let mut p = ColorParams::default();
        p.hsl[5] = [0.0, -1.0, 0.0]; // blue: fully desaturated
        let g = p.prepare();
        let blue = g.apply(lab_of_hue(264.0, 0.15), 1.0);
        assert!(blue[1].hypot(blue[2]) < 1e-4, "{blue:?}");
        let orange = lab_of_hue(55.0, 0.1);
        let out = g.apply(orange, 1.0);
        assert!(out.iter().zip(orange).all(|(x, y)| (x - y).abs() < 1e-6), "{out:?}");
    }

    #[test]
    fn hue_shift_rotates_and_weight_scales_the_change() {
        let mut p = ColorParams::default();
        p.hsl[3] = [1.0, 0.0, 0.0];
        let g = p.prepare();
        let green = lab_of_hue(135.0, 0.1);
        assert!((hue_of(g.apply(green, 1.0)) - 165.0).abs() < 0.5);
        assert!((hue_of(g.apply(green, 0.5)) - 150.0).abs() < 1.5);
        assert_eq!(g.apply(green, 0.0), green);
    }

    #[test]
    fn monochrome_ignores_protection_and_wheels_tone_it() {
        let mut p = ColorParams { mono: 1.0, ..Default::default() };
        let skin = lab_of_hue(58.0, 0.06);
        let out = p.prepare().apply(skin, 0.0);
        assert!(out[1].abs() < 1e-6 && out[2].abs() < 1e-6);

        p.wheels[2] = [45.0, 1.0, 0.0]; // warm highlights
        let out = p.prepare().apply([0.8, 0.0, 0.0], 1.0);
        let h = hue_of(out);
        assert!(out[1].hypot(out[2]) > 0.02 && (60.0..100.0).contains(&h), "{out:?} hue {h}");
    }

    #[test]
    fn fused_wheels_add_as_vectors() {
        let warm = ColorParams { wheels: [[0.0; 3], [0.0; 3], [40.0, 0.6, 0.1]], ..Default::default() };
        let cool = ColorParams { wheels: [[0.0; 3], [0.0; 3], [220.0, 0.6, 0.1]], ..Default::default() };
        let both = ColorParams::sum(&[warm, cool]);
        assert!(both.wheels[2][1] < 0.01, "opposite tints cancel: {:?}", both.wheels[2]);
        assert!((both.wheels[2][2] - 0.2).abs() < 1e-6, "luminance adds");
        let same = ColorParams::sum(&[warm, warm]);
        assert!((same.wheels[2][0] - 40.0).abs() < 1e-3 && (same.wheels[2][1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn contrast_keeps_black_white_and_pivot() {
        let g = ColorParams { contrast: 1.0, ..Default::default() }.prepare();
        for l in [0.0, 0.5, 1.0] {
            assert!((g.apply([l, 0.0, 0.0], 1.0)[0] - l).abs() < 1e-6);
        }
        assert!(g.apply([0.3, 0.0, 0.0], 1.0)[0] < 0.3);
        assert!(g.apply([0.7, 0.0, 0.0], 1.0)[0] > 0.7);
    }
}
