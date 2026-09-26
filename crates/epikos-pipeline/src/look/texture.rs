//! PRD Step 4: micro-texture and retouching, on Oklab L (and a/b for skin colour).
//!
//! Every radius is a fraction of the image's long side, so the downsampled preview
//! and the full-resolution export act on the same structures.
//!
//! - Clarity: guided-filter local contrast at ~1% of the frame, weighted to the
//!   midtones. The guided filter follows strong edges, so there are no halos.
//! - Micro-texture: the fine band, cored at the band's own noise level so grain isn't
//!   amplified, and eased off on skin (structure up, skin not "dirty").
//! - Blemish smoothing: on skin only, flattens the spot-sized band (between pores and
//!   facial structure). Dark spots are corrected more than light ones, and anything
//!   with strong contrast (eyes, brows, lips, lines) is left alone.
//! - Specular balancing: on and around skin, compresses shine that rises above the
//!   local skin level and pulls the surrounding skin colour back into it.
//! - Character line sculpting: on skin, the line-sized band (wider than pores, finer
//!   than the face's shape: smile lines, furrows, crow's feet) is deepened or softened.
//!   Lines are darker than the skin around them, so the dark side of the band is what
//!   moves; eyes, brows and lips (strong contrast) are left alone.

use epikos_core::ImageRgbF32;
use rayon::prelude::*;

use super::blur::{box_blur, guided, smooth};
use super::{long_side, smoothstep};

/// Normalised Step 4 strengths: clarity and micro −1…1, blemish and specular 0…1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct TextureParams {
    pub clarity: f32,
    pub micro: f32,
    pub blemish: f32,
    pub specular: f32,
    /// Character lines, −1 (soften) … 1 (deepen).
    pub lines: f32,
}

impl TextureParams {
    pub(crate) fn is_neutral(&self) -> bool {
        *self == Self::default()
    }

    pub(crate) fn plus(self, o: Self) -> Self {
        Self {
            clarity: (self.clarity + o.clarity).clamp(-1.0, 1.0),
            micro: (self.micro + o.micro).clamp(-1.0, 1.0),
            blemish: (self.blemish + o.blemish).clamp(0.0, 1.0),
            specular: (self.specular + o.specular).clamp(0.0, 1.0),
            lines: (self.lines + o.lines).clamp(-1.0, 1.0),
        }
    }

    pub(crate) fn scaled(self, k: f32) -> Self {
        Self {
            clarity: self.clarity * k,
            micro: self.micro * k,
            blemish: self.blemish * k,
            specular: self.specular * k,
            lines: self.lines * k,
        }
    }

    pub(crate) fn needs_skin(&self) -> bool {
        self.clarity > 0.0 || self.micro > 0.0 || self.blemish > 0.0 || self.specular > 0.0 || self.lines != 0.0
    }
}

/// Planes of `lab` hold Oklab (r = L, g = a, b = b). `skin` is the skin map.
pub(crate) fn apply_texture(lab: &mut ImageRgbF32, skin: &[f32], p: &TextureParams) {
    let (w, h) = (lab.width as usize, lab.height as usize);
    if p.is_neutral() || w < 4 || h < 4 {
        return;
    }
    let long = long_side(w, h);
    let radius = |frac: f32, min: f32| (frac * long).round().max(min) as usize;

    if p.clarity != 0.0 || p.micro != 0.0 {
        local_contrast(lab, skin, p, radius(0.012, 2.0), radius(0.0015, 1.0));
    }
    if p.blemish > 0.0 {
        smooth_blemishes(lab, skin, p.blemish, radius(0.0015, 1.0), radius(0.006, 2.0));
    }
    if p.specular > 0.0 {
        balance_specular(lab, skin, p.specular, radius(0.01, 2.0));
    }
    if p.lines != 0.0 {
        sculpt_lines(lab, skin, p.lines, radius(0.002, 1.0), radius(0.008, 2.0));
    }
}

fn sculpt_lines(lab: &mut ImageRgbF32, skin: &[f32], amount: f32, r1: usize, r2: usize) {
    let (w, h) = (lab.width as usize, lab.height as usize);
    let fine = smooth(&lab.r, w, h, r1, 2);
    let coarse = smooth(&lab.r, w, h, r2, 2);
    lab.r.par_iter_mut().enumerate().for_each(|(i, l)| {
        let s = skin[i];
        if s <= 0.0 {
            return;
        }
        let band = fine[i] - coarse[i];
        // Features with strong contrast are not lines.
        let keep = 1.0 - smoothstep(0.08, 0.16, band.abs());
        // The dark side carries the line; the light side (its highlight) moves less.
        let side = if band < 0.0 { 1.0 } else { 0.4 };
        let k = if amount > 0.0 { 1.2 * amount } else { 0.9 * amount };
        *l += k * s * keep * side * band;
    });
}

fn local_contrast(lab: &mut ImageRgbF32, skin: &[f32], p: &TextureParams, r_clarity: usize, r_micro: usize) {
    let (w, h) = (lab.width as usize, lab.height as usize);
    let l = &lab.r;
    let base_c = (p.clarity != 0.0).then(|| guided(l, w, h, r_clarity, 0.1 * 0.1));
    let base_m = (p.micro != 0.0).then(|| guided(l, w, h, r_micro, 0.04 * 0.04));
    // Core the fine band at its own noise level (MAD) so boosting doesn't lift grain.
    let core = base_m.as_ref().map_or(0.0, |b| noise_sigma(l, b));

    // Gain so that a small move shows: ±100 is strong local contrast.
    let clarity = p.clarity * if p.clarity > 0.0 { 1.6 } else { 1.2 };
    let micro = p.micro * if p.micro > 0.0 { 1.4 } else { 1.0 };
    lab.r.par_iter_mut().enumerate().for_each(|(i, v)| {
        let l = *v;
        let s = skin[i];
        let mut out = l;
        if let Some(base) = &base_c {
            let mid = smoothstep(0.02, 0.3, l) * (1.0 - smoothstep(0.8, 1.05, l));
            // Positive clarity is harsh on faces; ease it off where there is skin.
            let k = if clarity > 0.0 { clarity * (1.0 - 0.5 * s) } else { clarity };
            out += k * mid * (l - base[i]);
        }
        if let Some(base) = &base_m {
            let d = l - base[i];
            if micro > 0.0 {
                out += micro * (1.0 - 0.6 * s) * soft_threshold(d, core);
            } else {
                out += micro * d;
            }
        }
        *v = out;
    });
}

fn smooth_blemishes(lab: &mut ImageRgbF32, skin: &[f32], strength: f32, r1: usize, r2: usize) {
    let (w, h) = (lab.width as usize, lab.height as usize);
    // Spot-sized band = (fine blur) − (coarse blur) on L, a and b.
    let band = |plane: &[f32]| -> Vec<f32> {
        let fine = smooth(plane, w, h, r1, 2);
        let coarse = smooth(plane, w, h, r2, 2);
        fine.iter().zip(&coarse).map(|(f, c)| f - c).collect()
    };
    let (bl, ba, bb) = (band(&lab.r), band(&lab.g), band(&lab.b));
    lab.r
        .par_iter_mut()
        .zip(lab.g.par_iter_mut())
        .zip(lab.b.par_iter_mut())
        .enumerate()
        .for_each(|(i, ((l, a), b))| {
            let s = skin[i];
            if s <= 0.0 {
                return;
            }
            let mid = bl[i];
            // Strong local contrast is structure (eyes, brows, lips, lines): keep it.
            let keep = 1.0 - smoothstep(0.05, 0.12, mid.abs());
            let k = strength * s * keep;
            // Dark spots are what reads as a blemish; light ones are often highlights.
            *l -= k * if mid < 0.0 { 1.0 } else { 0.5 } * mid;
            // Even out red / brown patches.
            *a -= 0.8 * k * ba[i];
            *b -= 0.8 * k * bb[i];
        });
}

fn balance_specular(lab: &mut ImageRgbF32, skin: &[f32], strength: f32, r: usize) {
    let (w, h) = (lab.width as usize, lab.height as usize);
    // Shine is desaturated, so it scores low as skin itself: use the skin around it.
    let zone = box_blur(skin, w, h, r);
    let base = smooth(&lab.r, w, h, r, 2);
    let (ma, mb) = (smooth(&lab.g, w, h, r, 2), smooth(&lab.b, w, h, r, 2));
    lab.r
        .par_iter_mut()
        .zip(lab.g.par_iter_mut())
        .zip(lab.b.par_iter_mut())
        .enumerate()
        .for_each(|(i, ((l, a), b))| {
            let z = smoothstep(0.1, 0.5, zone[i]);
            let excess = (*l - base[i] - 0.03).max(0.0);
            if z <= 0.0 || excess <= 0.0 {
                return;
            }
            let k = strength * z;
            *l -= 0.7 * k * excess;
            let t = k * smoothstep(0.0, 0.12, excess);
            *a += t * (ma[i] - *a);
            *b += t * (mb[i] - *b);
        });
}

fn soft_threshold(d: f32, t: f32) -> f32 {
    d.signum() * (d.abs() - t).max(0.0)
}

/// Robust σ of `plane − base` (MAD / 0.6745) from a strided sample.
fn noise_sigma(plane: &[f32], base: &[f32]) -> f32 {
    let stride = (plane.len() / 65_536).max(1);
    let mut d: Vec<f32> = (0..plane.len())
        .step_by(stride)
        .map(|i| (plane[i] - base[i]).abs())
        .filter(|v| v.is_finite())
        .collect();
    if d.is_empty() {
        return 0.0;
    }
    let mid = d.len() / 2;
    let (_, m, _) = d.select_nth_unstable_by(mid, f32::total_cmp);
    *m / 0.6745
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    /// Oklab planes: uniform skin-coloured patch with a dark spot (1% of the frame
    /// across, like a blemish in a head-and-shoulders portrait) in the middle.
    fn skin_patch(n: u32) -> (ImageRgbF32, Vec<f32>) {
        let mut img = ImageRgbF32::new(n, n, ColorSpace::LinearRec2020);
        let c = n as f32 / 2.0;
        for y in 0..n {
            for x in 0..n {
                let i = img.index(x, y);
                let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
                img.r[i] = if d < n as f32 * 0.005 { 0.45 } else { 0.5 };
                img.g[i] = 0.04;
                img.b[i] = 0.05;
            }
        }
        (img, vec![1.0; (n * n) as usize])
    }

    #[test]
    fn neutral_params_change_nothing() {
        let (mut img, skin) = skin_patch(64);
        let before = img.clone();
        apply_texture(&mut img, &skin, &TextureParams::default());
        assert_eq!(img.r, before.r);
    }

    /// Uniform skin with a thin dark line (a crease ~0.5% of the frame wide).
    fn crease(n: u32) -> (ImageRgbF32, Vec<f32>) {
        let mut img = ImageRgbF32::new(n, n, ColorSpace::LinearRec2020);
        for y in 0..n {
            for x in 0..n {
                let i = img.index(x, y);
                let d = (x as f32 - n as f32 / 2.0).abs();
                img.r[i] = if d < n as f32 * 0.0025 { 0.46 } else { 0.5 };
            }
        }
        (img, vec![1.0; (n * n) as usize])
    }

    #[test]
    fn character_lines_deepen_and_soften_a_crease() {
        let n = 800;
        let at = |img: &ImageRgbF32| 0.5 - img.r[img.index(n / 2, n / 2)];
        let (base, skin) = crease(n);
        let depth = at(&base);
        let mut deeper = base.clone();
        apply_texture(&mut deeper, &skin, &TextureParams { lines: 1.0, ..Default::default() });
        let mut softer = base.clone();
        apply_texture(&mut softer, &skin, &TextureParams { lines: -1.0, ..Default::default() });
        assert!(at(&deeper) > 1.3 * depth, "{depth} → {}", at(&deeper));
        assert!(at(&softer) < 0.8 * depth, "{depth} → {}", at(&softer));
        // No skin, no change.
        let mut bare = base.clone();
        apply_texture(&mut bare, &vec![0.0; skin.len()], &TextureParams { lines: 1.0, ..Default::default() });
        assert_eq!(bare.r, base.r);
    }

    #[test]
    fn blemish_smoothing_fills_a_dark_spot_on_skin() {
        let (mut img, skin) = skin_patch(1000);
        let centre = img.index(500, 500);
        let before = 0.5 - img.r[centre];
        apply_texture(&mut img, &skin, &TextureParams { blemish: 1.0, ..Default::default() });
        let after = 0.5 - img.r[centre];
        assert!(after < 0.5 * before, "spot depth {before} → {after}");

        // Not skin: untouched.
        let (mut img, _) = skin_patch(1000);
        let no_skin = vec![0.0; img.len()];
        apply_texture(&mut img, &no_skin, &TextureParams { blemish: 1.0, ..Default::default() });
        assert_eq!(img.r[centre], 0.45);
    }

    #[test]
    fn specular_balancing_tames_shine_on_skin() {
        let (mut img, skin) = skin_patch(400);
        // Turn the spot into a bright, desaturated shine.
        let centre = img.index(200, 200);
        for i in 0..img.len() {
            if img.r[i] < 0.5 {
                img.r[i] = 0.75;
                img.g[i] = 0.01;
                img.b[i] = 0.01;
            }
        }
        apply_texture(&mut img, &skin, &TextureParams { specular: 1.0, ..Default::default() });
        assert!(img.r[centre] < 0.7, "L {}", img.r[centre]);
        assert!(img.g[centre] > 0.015, "chroma not restored: {}", img.g[centre]);
    }

    #[test]
    fn micro_texture_boosts_detail_above_the_noise() {
        let n = 256u32;
        let mut img = ImageRgbF32::new(n, n, ColorSpace::LinearRec2020);
        let mut seed = 1u32;
        for i in 0..img.len() {
            let x = i as u32 % n;
            // Faint noise everywhere; a 4-px stripe texture of amplitude 0.02 in one band.
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = 0.002 * ((seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0);
            let stripe = if (64..128).contains(&x) {
                if (x / 2).is_multiple_of(2) { 0.02 } else { -0.02 }
            } else {
                0.0
            };
            img.r[i] = 0.5 + stripe + noise;
        }
        let skin = vec![0.0; img.len()];
        let amp = |im: &ImageRgbF32| im.r[im.index(100, 100)] - im.r[im.index(102, 100)];
        let before = amp(&img);
        apply_texture(&mut img, &skin, &TextureParams { micro: 1.0, ..Default::default() });
        assert!(amp(&img) > 1.3 * before, "{before} → {}", amp(&img));
    }
}
