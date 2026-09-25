//! Steps 4–6 plus the parametric style engine, run on upright scene-linear Rec.2020
//! after exposure.
//!
//! Order inside: Step 3 local adjustments through their masks → Oklab → skin map
//! (from the image before any look, optionally confined to the subject) → Step 4
//! texture and line sculpting (manual + style) → Step 5 manual grade (manual skin
//! protection) → style skin grade (skin only) → style scene grade (style skin
//! protection) → foliage shift and background re-colouration → linear → Step 6 fog,
//! glow (optionally from the subject only), light shafts (manual + style) → Step 7
//! curves and split toning → Step 8 vignette and grain.

mod atmosphere;
mod blur;
mod finish;
mod grade;
mod lights;
mod local;
mod oklab;
mod region;
mod skin;
mod style;
mod texture;
mod tone;

use epikos_core::resize_plane;
use epikos_core::ImageRgbF32;
use epikos_core::LensProfile;
use epikos_sidecar::{
    Adjustments, Atmosphere, BackgroundTint, ColorGrade, Curves, Finishing, HslChannel, LocalAdjustment,
    MaskTarget, SplitToning, Texture,
};
use rayon::prelude::*;

use atmosphere::{apply_atmosphere, AtmosphereParams};
use grade::ColorParams;
use oklab::Oklab;
use texture::{apply_texture, TextureParams};

pub use style::StyleInfo;
pub use tone::bake_tone_curve;

/// Every built-in style, in display order.
pub fn styles() -> Vec<StyleInfo> {
    style::all().into_iter().map(|s| s.info).collect()
}

/// Extra per-image inputs for the develop, from the engine's models and databases.
#[derive(Debug, Clone, Copy, Default)]
pub struct LookInputs<'a> {
    /// Relative depth of the upright frame (1 = near, 0 = far), any resolution.
    pub depth: Option<DepthPlane<'a>>,
    /// Step 3 masks of the upright frame (0–1), any resolution.
    pub masks: &'a [MaskPlane<'a>],
    /// Step 1 lens profile, applied when `adjustments.lens.profile` is on.
    pub lens: Option<&'a LensProfile>,
}

#[derive(Debug, Clone, Copy)]
pub struct MaskPlane<'a> {
    pub target: MaskTarget,
    pub width: u32,
    pub height: u32,
    pub data: &'a [f32],
}

/// The Step 3 masks the look for `adj` would use, so the engine can prepare them.
pub fn look_masks(adj: &Adjustments) -> Vec<MaskTarget> {
    let look = Look::from_adjustments(adj);
    let mut out: Vec<MaskTarget> = look.local.iter().map(|l| l.mask).collect();
    if look.needs_subject() {
        out.push(MaskTarget::Subject);
    }
    // Background is the subject's inverse: the subject model serves both.
    for t in out.iter_mut() {
        if *t == MaskTarget::Background {
            *t = MaskTarget::Subject;
        }
    }
    out.sort_by_key(|t| *t as u8);
    out.dedup();
    out
}

/// Box blur of a plane (radius `r`), for other modules.
pub(crate) fn box_plane(p: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    blur::box_blur(p, w, h, r)
}

/// Edge-aware smoothing of a plane (guided filter), for other modules.
pub(crate) fn guided_plane(p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    blur::guided(p, w, h, r, eps)
}

#[derive(Debug, Clone, Copy)]
pub struct DepthPlane<'a> {
    pub width: u32,
    pub height: u32,
    pub data: &'a [f32],
}

/// Whether the look for `adj` would use a depth map (fog or light shafts).
pub fn look_needs_depth(adj: &Adjustments) -> bool {
    Look::from_adjustments(adj).needs_depth()
}

/// The image in Oklab (planes r, g, b hold L, a, b), for analysis.
pub fn oklab_planes(rgb: &ImageRgbF32) -> ImageRgbF32 {
    let mut lab = rgb.clone();
    Oklab::new().planes_to_lab(&mut lab);
    lab
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
    curves: Curves,
    split: SplitToning,
    finishing: Finishing,
    lights: Vec<epikos_sidecar::VirtualLight>,
    local: Vec<LocalAdjustment>,
    retouch_subject_only: bool,
    foliage: HslChannel,
    background: BackgroundTint,
    glow_subject_only: bool,
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
            atmosphere: atmosphere_params(&adj.atmosphere),
            curves: adj.curves.clone(),
            split: adj.split_toning,
            finishing: adj.finishing,
            lights: adj.atmosphere.lights.clone(),
            local: adj.local.iter().filter(|l| !l.is_neutral()).copied().collect(),
            retouch_subject_only: adj.texture.retouch_subject_only,
            foliage: adj.color.foliage,
            background: adj.color.background,
            glow_subject_only: adj.atmosphere.glow_subject_only,
        };
        if adj.style.is_none() {
            return look;
        }
        // One style, or a fusion blend with weights normalised to 1. Unknown ids (a
        // sidecar from a newer version) are ignored, not an error.
        let entries: Vec<(style::Style, f32)> = if adj.style.blend.is_empty() {
            style::find(&adj.style.id).map(|s| (s, 1.0)).into_iter().collect()
        } else {
            let total: f32 = adj.style.blend.iter().map(|b| b.weight.max(0.0)).sum();
            adj.style
                .blend
                .iter()
                .filter(|b| b.weight > 0.0 && total > 0.0)
                .filter_map(|b| style::find(&b.id).map(|s| (s, b.weight / total)))
                .collect()
        };
        if entries.is_empty() {
            return look;
        }
        let amount = pct(adj.style.amount);
        let (mut skins, mut scenes) = (Vec::new(), Vec::new());
        for (s, weight) in entries {
            let k = amount * weight;
            look.texture = look.texture.plus(s.texture.scaled(k));
            skins.push(s.skin.scaled(k));
            scenes.push(s.scene.scaled(k));
            look.atmosphere = look.atmosphere.plus(s.atmosphere.scaled(k));
        }
        look.style_skin = ColorParams::sum(&skins);
        look.style_scene = ColorParams::sum(&scenes);
        look.style_protection = pct(adj.style.skin_protection);
        look
    }

    fn needs_lab(&self) -> bool {
        !self.texture.is_neutral()
            || !self.manual.is_neutral()
            || !self.style_skin.is_neutral()
            || !self.style_scene.is_neutral()
            || !region::foliage_is_neutral(&self.foliage)
            || !self.background.is_neutral()
    }

    /// Whether any setting reads the subject mask (beyond local adjustments).
    fn needs_subject(&self) -> bool {
        (self.retouch_subject_only && self.texture.needs_skin())
            || !self.background.is_neutral()
            || !region::foliage_is_neutral(&self.foliage)
            || (self.glow_subject_only && self.atmosphere.glow > 0.0)
    }

    fn needs_skin(&self) -> bool {
        self.texture.needs_skin()
            || (self.manual_protection > 0.0 && !self.manual.is_neutral())
            || !self.style_skin.is_neutral()
            || (self.style_protection > 0.0 && !self.style_scene.is_neutral())
    }

    fn needs_depth(&self) -> bool {
        self.atmosphere.needs_depth() || self.lights.iter().any(|l| l.intensity > 0.0)
    }

    fn is_neutral(&self) -> bool {
        self.local.is_empty()
            && !self.needs_lab()
            && self.atmosphere.is_neutral()
            && !self.lights.iter().any(|l| l.intensity > 0.0)
            && self.curves.is_identity()
            && self.split.is_neutral()
    }
}

/// Whether [`apply_look`] would change the image (lets callers skip the work).
pub fn look_is_active(adj: &Adjustments) -> bool {
    !Look::from_adjustments(adj).is_neutral()
}

/// Apply Step 3 local adjustments, Steps 4–8 and the style to upright scene-linear
/// Rec.2020, in place.
pub fn apply_look(rgb: &mut ImageRgbF32, adj: &Adjustments, inputs: &LookInputs) {
    let look = Look::from_adjustments(adj);
    if look.is_neutral() {
        return;
    }
    // Masks fitted to this image once, on the photo before any look.
    let mut fitted: Vec<(MaskTarget, Option<Vec<f32>>)> = Vec::new();
    let mut mask = |rgb: &ImageRgbF32, target: MaskTarget| -> Option<Vec<f32>> {
        let base = if target == MaskTarget::Background { MaskTarget::Subject } else { target };
        if !fitted.iter().any(|(t, _)| *t == base) {
            let plane = inputs.masks.iter().find(|m| m.target == base).and_then(|m| {
                fit_to_image(rgb, m.data, m.width, m.height, 0.004)
            });
            fitted.push((base, plane));
        }
        let plane = fitted.iter().find(|(t, _)| *t == base)?.1.clone()?;
        Some(if target == MaskTarget::Background { plane.iter().map(|v| 1.0 - v).collect() } else { plane })
    };
    let subject = look.needs_subject().then(|| mask(rgb, MaskTarget::Subject)).flatten();

    for l in &look.local {
        // A mask whose model isn't installed does nothing, rather than everything.
        if let Some(m) = mask(rgb, l.mask) {
            local::apply_local(rgb, l, &m);
        }
    }
    if look.needs_lab() {
        let ok = Oklab::new();
        ok.planes_to_lab(rgb);
        let mut skin = if look.needs_skin() {
            skin::skin_map(rgb)
        } else {
            vec![0.0; rgb.len()]
        };
        if look.retouch_subject_only {
            if let Some(s) = &subject {
                skin.iter_mut().zip(s).for_each(|(k, s)| *k *= s);
            }
        }
        apply_texture(rgb, &skin, &look.texture);
        grade_passes(rgb, &skin, &look);
        region::apply_foliage(rgb, &look.foliage, subject.as_deref());
        if let Some(s) = &subject {
            region::apply_background(rgb, &look.background, s);
        }
        ok.planes_to_rec2020(rgb);
    }
    let depth = look
        .needs_depth()
        .then_some(inputs.depth)
        .flatten()
        .map(|d| fit_depth(rgb, d));
    let glow_mask = if look.glow_subject_only { subject.as_deref() } else { None };
    apply_atmosphere(rgb, &look.atmosphere, depth.as_deref(), glow_mask);
    lights::apply_lights(rgb, &look.lights, depth.as_deref());
    tone::apply_tone(rgb, &look.curves, &look.split);
    finish::apply_finishing(rgb, &look.finishing);
}

/// Resize the model's depth to the image and snap its edges to the photo's, so fog
/// doesn't halo around a subject.
fn fit_depth(rgb: &ImageRgbF32, d: DepthPlane) -> Vec<f32> {
    fit_to_image(rgb, d.data, d.width, d.height, 0.006)
        .unwrap_or_else(|| vec![0.5; rgb.len()])
}

/// Upsample a low-resolution model output (mask or depth, 0–1) to `rgb`'s size and snap
/// its edges to the photo's own with a joint guided filter. `radius` is a fraction of
/// the long side. `None` if `plane` doesn't match its stated size.
pub fn fit_to_image(rgb: &ImageRgbF32, plane: &[f32], width: u32, height: u32, radius: f32) -> Option<Vec<f32>> {
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    if width == 0 || height == 0 || plane.len() != (width * height) as usize {
        return None;
    }
    let up = resize_plane(plane, width, height, rgb.width, rgb.height);
    // Perceptual lightness as the guide.
    let guide: Vec<f32> = (0..rgb.len())
        .into_par_iter()
        .map(|i| (0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(0.0).cbrt())
        .collect();
    let r = (radius * long_side(w, h)).round().max(2.0) as usize;
    let mut fitted = blur::guided_joint(&guide, &up, w, h, r, 1e-3);
    fitted.par_iter_mut().for_each(|v| *v = v.clamp(0.0, 1.0));
    Some(fitted)
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
        lines: signed_pct(t.character_lines),
    }
}

fn atmosphere_params(a: &Atmosphere) -> AtmosphereParams {
    AtmosphereParams {
        glow: pct(a.glow),
        // 0…100 → 0.5–5% of the long side.
        glow_radius: 0.005 + 0.045 * pct(a.glow_size),
        glow_warmth: signed_pct(a.glow_warmth),
        fog: pct(a.fog),
        fog_start: pct(a.fog_start),
        fog_warmth: signed_pct(a.fog_warmth),
        shafts: pct(a.shafts),
        shaft_length: pct(a.shaft_length),
        shaft_warmth: signed_pct(a.shaft_warmth),
        light: (!a.shaft_auto).then(|| (a.shaft_x.clamp(0.0, 1.0), a.shaft_y.clamp(0.0, 1.0))),
        ..Default::default()
    }
}

fn color_params(c: &ColorGrade) -> ColorParams {
    let w = |w: &epikos_sidecar::ColorWheel| [w.hue, pct(w.amount), signed_pct(w.luminance)];
    ColorParams {
        hsl: c.hsl.bands().map(|b| {
            [
                signed_pct(b.hue),
                signed_pct(b.saturation),
                signed_pct(b.luminance),
            ]
        }),
        wheels: [
            w(&c.wheels.shadows),
            w(&c.wheels.midtones),
            w(&c.wheels.highlights),
        ],
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
            let px = if x < 32 {
                [0.30, 0.17, 0.10]
            } else {
                [0.12, 0.25, 0.55]
            };
            (img.r[i], img.g[i], img.b[i]) = (px[0], px[1], px[2]);
        }
        img
    }

    #[test]
    fn default_adjustments_leave_the_image_alone() {
        let mut img = portrait();
        let before = img.clone();
        assert!(!look_is_active(&Adjustments::default()));
        apply_look(&mut img, &Adjustments::default(), &LookInputs::default());
        assert_eq!(img.r, before.r);
    }

    #[test]
    fn skin_protection_shields_skin_from_the_grade_but_not_the_sky() {
        let mut adj = Adjustments::default();
        adj.color.wheels.midtones = epikos_sidecar::ColorWheel {
            hue: 200.0,
            amount: 80.0,
            luminance: 0.0,
        };
        adj.color.hsl.orange.saturation = 60.0;
        let shift = |protection: f32| {
            let mut a = adj.clone();
            a.color.skin_protection = protection;
            let mut img = portrait();
            let before = img.clone();
            apply_look(&mut img, &a, &LookInputs::default());
            let d = |i: usize| (img.b[i] - before.b[i]).abs() + (img.r[i] - before.r[i]).abs();
            (d(img.index(8, 16)), d(img.index(56, 16)))
        };
        let (skin_off, sky_off) = shift(0.0);
        let (skin_on, sky_on) = shift(100.0);
        assert!(skin_on < 0.3 * skin_off, "skin {skin_off} → {skin_on}");
        assert!((sky_on - sky_off).abs() < 1e-4, "sky {sky_off} vs {sky_on}");
    }

    #[test]
    fn styles_protect_skin_too() {
        let mut adj = Adjustments::default();
        adj.style.id = "volumetric-golden-hour".into();
        let hue_shift = |protection: f32| {
            let mut a = adj.clone();
            a.style.skin_protection = protection;
            let mut img = portrait();
            apply_look(&mut img, &a, &LookInputs::default());
            let i = img.index(8, 16);
            img.r[i] / img.b[i]
        };
        // Warmer (higher R/B) without protection than with it.
        assert!(hue_shift(0.0) > hue_shift(100.0) * 1.02);
    }

    #[test]
    fn silver_and_charcoal_is_neutral_everywhere() {
        let mut adj = Adjustments::default();
        adj.style.id = "silver-charcoal".into();
        adj.style.skin_protection = 100.0;
        let mut img = portrait();
        apply_look(&mut img, &adj, &LookInputs::default());
        for i in [img.index(8, 16), img.index(56, 16)] {
            let (r, g, b) = (img.r[i], img.g[i], img.b[i]);
            assert!(
                (r - g).abs() < 0.01 * g.max(0.05) && (b - g).abs() < 0.01 * g.max(0.05),
                "{r} {g} {b}"
            );
        }
    }

    #[test]
    fn fog_uses_depth_when_given() {
        let mut adj = Adjustments::default();
        adj.atmosphere.fog = 100.0;
        adj.atmosphere.fog_start = 0.0;
        assert!(look_needs_depth(&adj));
        // Left half near, right half far.
        let depth: Vec<f32> = (0..64 * 32)
            .map(|i| if i % 64 < 32 { 1.0 } else { 0.0 })
            .collect();
        let inputs = LookInputs {
            depth: Some(DepthPlane {
                width: 64,
                height: 32,
                data: &depth,
            }),
            ..Default::default()
        };
        let mut img = portrait();
        let before = img.clone();
        apply_look(&mut img, &adj, &inputs);
        let change = |x: u32| (img.g[img.index(x, 16)] - before.g[before.index(x, 16)]).abs();
        assert!(
            change(56) > 5.0 * change(8).max(1e-4),
            "near {} far {}",
            change(8),
            change(56)
        );
    }

    #[test]
    fn local_adjustments_follow_their_mask_and_skip_missing_ones() {
        let mut adj = Adjustments::default();
        adj.local.push(LocalAdjustment { mask: MaskTarget::Background, exposure: 1.0, ..Default::default() });
        adj.local.push(LocalAdjustment { mask: MaskTarget::Eyes, exposure: 2.0, ..Default::default() });
        assert_eq!(look_masks(&adj), vec![MaskTarget::Subject, MaskTarget::Eyes]);
        // Subject = left half; no eye mask available.
        let subject: Vec<f32> = (0..64 * 32).map(|i| if i % 64 < 32 { 1.0 } else { 0.0 }).collect();
        let masks = [MaskPlane { target: MaskTarget::Subject, width: 64, height: 32, data: &subject }];
        let inputs = LookInputs { masks: &masks, ..Default::default() };
        let mut img = portrait();
        let before = img.clone();
        apply_look(&mut img, &adj, &inputs);
        let ratio = |x: u32| img.g[img.index(x, 16)] / before.g[before.index(x, 16)];
        assert!((ratio(4) - 1.0).abs() < 0.05, "subject {}", ratio(4));
        assert!((ratio(60) - 2.0).abs() < 0.1, "background {}", ratio(60));
    }

    #[test]
    fn subject_settings_ask_for_the_subject_mask() {
        let mut adj = Adjustments::default();
        assert!(look_masks(&adj).is_empty());
        adj.color.background.amount = 40.0;
        assert_eq!(look_masks(&adj), vec![MaskTarget::Subject]);
        let mut adj = Adjustments::default();
        adj.atmosphere.glow = 50.0;
        adj.atmosphere.glow_subject_only = true;
        assert_eq!(look_masks(&adj), vec![MaskTarget::Subject]);
    }

    #[test]
    fn unknown_style_is_ignored() {
        let mut adj = Adjustments::default();
        adj.style.id = "from-the-future".into();
        assert!(!look_is_active(&adj));
    }
}
