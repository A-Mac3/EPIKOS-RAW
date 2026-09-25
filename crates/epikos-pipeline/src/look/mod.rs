//! Steps 4–5 plus the parametric style engine, run on scene-linear Rec.2020 after
//! exposure.
//!
//! Order inside: Oklab → skin map (from the image before any look) → Step 4 texture
//! (manual + style) → Step 5 manual grade (manual skin protection) → style skin grade
//! (skin only) → style scene grade (style skin protection) → linear → style glow/haze.

mod atmosphere;
mod blur;
mod grade;
mod oklab;
mod skin;
mod style;
mod texture;

use epikos_core::ImageRgbF32;
use epikos_sidecar::{Adjustments, ColorGrade, Texture};
use rayon::prelude::*;

use atmosphere::{apply_atmosphere, AtmosphereParams};
use grade::ColorParams;
use oklab::Oklab;
use texture::{apply_texture, TextureParams};

pub use style::StyleInfo;

/// Every built-in style, in display order.
pub fn styles() -> Vec<StyleInfo> {
    style::all().into_iter().map(|s| s.info).collect()
}

/// Skin likelihood (0–1 per pixel) of a scene-linear Rec.2020 image, as used by
/// retouching and skin protection.
pub fn skin_likelihood(rgb: &ImageRgbF32) -> Vec<f32> {
    let mut lab = rgb.clone();
    Oklab::new().planes_to_lab(&mut lab);
    skin::skin_map(&lab)
}

/// Everything Steps 4–5 and the style ask for, normalised.
struct Look {
    texture: TextureParams,
    manual: ColorParams,
    manual_protection: f32,
    style_skin: ColorParams,
    style_scene: ColorParams,
    style_protection: f32,
    atmosphere: AtmosphereParams,
}

impl Look {
    fn from_adjustments(adj: &Adjustments) -> Self {
        let mut look = Look {
            texture: texture_params(&adj.texture),
            manual: color_params(&adj.color),
            manual_protection: pct(adj.color.skin_protection),
            style_skin: ColorParams::default(),
            style_scene: ColorParams::default(),
            style_protection: 0.0,
            atmosphere: AtmosphereParams::default(),
        };
        if adj.style.is_none() {
            return look;
        }
        // Unknown ids (a sidecar from a newer version) are ignored, not an error.
        if let Some(s) = style::find(&adj.style.id) {
            let k = pct(adj.style.amount);
            look.texture = look.texture.plus(s.texture.scaled(k));
            look.style_skin = s.skin.scaled(k);
            look.style_scene = s.scene.scaled(k);
            look.style_protection = pct(adj.style.skin_protection);
            look.atmosphere = s.atmosphere.scaled(k);
        }
        look
    }

    fn needs_lab(&self) -> bool {
        !self.texture.is_neutral()
            || !self.manual.is_neutral()
            || !self.style_skin.is_neutral()
            || !self.style_scene.is_neutral()
    }

    fn needs_skin(&self) -> bool {
        self.texture.needs_skin()
            || (self.manual_protection > 0.0 && !self.manual.is_neutral())
            || !self.style_skin.is_neutral()
            || (self.style_protection > 0.0 && !self.style_scene.is_neutral())
    }

    fn is_neutral(&self) -> bool {
        !self.needs_lab() && self.atmosphere.is_neutral()
    }
}

/// Whether [`apply_look`] would change the image (lets callers skip the work).
pub fn look_is_active(adj: &Adjustments) -> bool {
    !Look::from_adjustments(adj).is_neutral()
}

/// Apply Steps 4–5 and the style to scene-linear Rec.2020, in place.
pub fn apply_look(rgb: &mut ImageRgbF32, adj: &Adjustments) {
    let look = Look::from_adjustments(adj);
    if look.is_neutral() {
        return;
    }
    if look.needs_lab() {
        let ok = Oklab::new();
        ok.planes_to_lab(rgb);
        let skin = if look.needs_skin() {
            skin::skin_map(rgb)
        } else {
            vec![0.0; rgb.len()]
        };
        apply_texture(rgb, &skin, &look.texture);
        grade_passes(rgb, &skin, &look);
        ok.planes_to_rec2020(rgb);
    }
    apply_atmosphere(rgb, &look.atmosphere);
}

fn grade_passes(lab: &mut ImageRgbF32, skin: &[f32], look: &Look) {
    let passes: Vec<_> = [
        (&look.manual, Weight::Protect(look.manual_protection)),
        (&look.style_skin, Weight::SkinOnly),
        (&look.style_scene, Weight::Protect(look.style_protection)),
    ]
    .into_iter()
    .filter(|(p, _)| !p.is_neutral())
    .map(|(p, w)| (p.prepare(), w))
    .collect();
    if passes.is_empty() {
        return;
    }
    lab.r
        .par_iter_mut()
        .zip(lab.g.par_iter_mut())
        .zip(lab.b.par_iter_mut())
        .enumerate()
        .for_each(|(i, ((l, a), b))| {
            let mut px = [*l, *a, *b];
            for (g, w) in &passes {
                let weight = match w {
                    Weight::Protect(p) => 1.0 - p * skin[i],
                    Weight::SkinOnly => skin[i],
                };
                px = g.apply(px, weight);
            }
            [*l, *a, *b] = px;
        });
}

enum Weight {
    /// Everywhere, with skin shielded by this much (0–1).
    Protect(f32),
    /// Only on skin.
    SkinOnly,
}

fn pct(v: f32) -> f32 {
    (v / 100.0).clamp(0.0, 1.0)
}

fn signed_pct(v: f32) -> f32 {
    (v / 100.0).clamp(-1.0, 1.0)
}

fn texture_params(t: &Texture) -> TextureParams {
    TextureParams {
        clarity: signed_pct(t.clarity),
        micro: signed_pct(t.micro_texture),
        blemish: pct(t.blemish_smoothing),
        specular: pct(t.specular_balance),
    }
}

fn color_params(c: &ColorGrade) -> ColorParams {
    let w = |w: &epikos_sidecar::ColorWheel| [w.hue, pct(w.amount), signed_pct(w.luminance)];
    ColorParams {
        hsl: c
            .hsl
            .bands()
            .map(|b| [signed_pct(b.hue), signed_pct(b.saturation), signed_pct(b.luminance)]),
        wheels: [w(&c.wheels.shadows), w(&c.wheels.midtones), w(&c.wheels.highlights)],
        ..Default::default()
    }
}

pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub(crate) fn long_side(w: usize, h: usize) -> f32 {
    w.max(h) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    /// Left half skin-coloured, right half sky blue, linear Rec.2020.
    fn portrait() -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(64, 32, ColorSpace::LinearRec2020);
        for i in 0..img.len() {
            let x = i % 64;
            let px = if x < 32 { [0.30, 0.17, 0.10] } else { [0.12, 0.25, 0.55] };
            (img.r[i], img.g[i], img.b[i]) = (px[0], px[1], px[2]);
        }
        img
    }

    #[test]
    fn default_adjustments_leave_the_image_alone() {
        let mut img = portrait();
        let before = img.clone();
        assert!(!look_is_active(&Adjustments::default()));
        apply_look(&mut img, &Adjustments::default());
        assert_eq!(img.r, before.r);
    }

    #[test]
    fn skin_protection_shields_skin_from_a_style_but_not_the_sky() {
        let mut adj = Adjustments::default();
        adj.style.id = "volumetric-golden-hour".into();
        adj.style.amount = 100.0;
        let shift = |protection: f32| {
            let mut a = adj.clone();
            a.style.skin_protection = protection;
            let mut img = portrait();
            let before = img.clone();
            apply_look(&mut img, &a);
            let d = |i: usize| (img.b[i] - before.b[i]).abs() + (img.r[i] - before.r[i]).abs();
            (d(img.index(8, 16)), d(img.index(56, 16)))
        };
        let (skin_off, sky_off) = shift(0.0);
        let (skin_on, sky_on) = shift(100.0);
        assert!(skin_on < 0.6 * skin_off, "skin {skin_off} → {skin_on}");
        assert!((sky_on - sky_off).abs() < 1e-3, "sky {sky_off} vs {sky_on}");
    }

    #[test]
    fn silver_and_charcoal_is_neutral_everywhere() {
        let mut adj = Adjustments::default();
        adj.style.id = "silver-charcoal".into();
        adj.style.skin_protection = 100.0;
        let mut img = portrait();
        apply_look(&mut img, &adj);
        for i in [img.index(8, 16), img.index(56, 16)] {
            let (r, g, b) = (img.r[i], img.g[i], img.b[i]);
            assert!((r - g).abs() < 0.01 * g.max(0.05) && (b - g).abs() < 0.01 * g.max(0.05), "{r} {g} {b}");
        }
    }

    #[test]
    fn unknown_style_is_ignored() {
        let mut adj = Adjustments::default();
        adj.style.id = "from-the-future".into();
        assert!(!look_is_active(&adj));
    }
}
