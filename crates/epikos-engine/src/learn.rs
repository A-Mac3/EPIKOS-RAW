//! AI Style Learning: measure the look of a reference photo (tone, skin, colour) as a
//! [`Signature`], keep learned signatures in a local store, and move another photo's
//! edit towards a signature (the AI Mentor's starting targets).
//!
//! Everything is measured on the photo as it is displayed (the finished look):
//! - **Tone transfer**: black point, white point, median and midtone spread of the
//!   displayed lightness (Oklab L).
//! - **Skin**: its hue, how rich its colour is *within the natural range for its own
//!   depth* (so a learned look never pushes one skin tone towards another), its
//!   brightness relative to the frame, and the share of specular highlights on it.
//! - **Colour**: hue, chroma and share of each of the eight HSL bands, the foliage
//!   (greenery outside the subject) and the background's saturation.
//!
//! Matching is a measure-and-adjust loop over the real render: each pass develops the
//! candidate settings, measures them and moves each control by the remaining gap,
//! damped, so the result is what the sliders actually do rather than a formula.

use std::fs;
use std::path::{Path, PathBuf};

use epikos_core::{ColorSpace, Error, ImageRgbF32, Result};
use epikos_pipeline::{develop_rgb_with, to_display_srgb, DisplayImage};
use epikos_sidecar::{Adjustments, Crop, LocalAdjustment, MaskTarget};
use serde::{Deserialize, Serialize};

use crate::analysis::{self, dominant_colors, Swatch};
use crate::guidance::{self, chroma_band};
use crate::{Engine, Loaded};

/// Measuring size: enough for stable statistics, fast enough to repeat.
const LEARN_SIDE: u32 = 768;
const FIT_SIDE: u32 = 320;
const FIT_PASSES: usize = 3;

/// Oklab hue (degrees) at the centre of each HSL band, as the pipeline defines them.
const BAND_CENTERS: [f32; 8] = [29.0, 55.0, 110.0, 135.0, 195.0, 264.0, 294.0, 328.0];
/// Hue shift of an HSL band at ±100, and of the foliage control (degrees).
const BAND_HUE_SHIFT: f32 = 30.0;
const FOLIAGE_TURN: f32 = 45.0;
/// Stops of gain of Whites / Blacks at ±100 (see the pipeline's Step 2).
const ENDS_STOPS: f32 = 1.5;
/// The Editorial profile's gentlest midtone S-curve.
const EDITORIAL_MIN_CURVE: f32 = 15.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToneSignature {
    /// Oklab L at the 0.5th percentile: how deep the blacks are.
    pub black: f32,
    /// Oklab L at the 99.5th percentile: where white sits.
    pub white: f32,
    pub median: f32,
    /// Interquartile range of L: midtone contrast (the S-curve's slope).
    pub contrast: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinSignature {
    /// Median skin colour (Oklab L, hue in degrees, chroma).
    pub l: f32,
    pub hue: f32,
    pub chroma: f32,
    /// 0…1: where the chroma sits in the natural range for this skin's depth.
    pub richness: f32,
    /// Skin lightness minus the frame's median.
    pub relative_l: f32,
    /// Share of skin that is specular highlight.
    pub specular: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BandStat {
    /// Mean Oklab hue (degrees), median chroma, share of the frame.
    pub hue: f32,
    pub chroma: f32,
    pub share: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    pub tone: ToneSignature,
    pub skin: Option<SkinSignature>,
    /// Red, orange, yellow, green, aqua, blue, purple, magenta.
    pub bands: Vec<BandStat>,
    pub foliage: Option<BandStat>,
    /// Median chroma off the subject.
    pub background_chroma: Option<f32>,
}

/// A look learned from a reference photo.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnedStyle {
    pub id: String,
    pub name: String,
    /// File name of the reference.
    pub source: String,
    /// Seconds since 1970.
    pub created_at: u64,
    pub signature: Signature,
    /// Five dominant colours of the reference, for the library card.
    pub palette: Vec<Swatch>,
}

/// A starting target for the AI Mentor.
pub(crate) struct Target<'a> {
    pub name: String,
    pub signature: &'a Signature,
    /// Match the reference's overall brightness (learned looks); the built-in profile
    /// keeps the photo's own exposure.
    pub match_exposure: bool,
    /// Adjust the HSL bands and the background (learned looks only).
    pub match_colours: bool,
}

/// The built-in Editorial / High-Fashion profile: rich, anchored blacks, a clean white,
/// a dynamic midtone curve, skin rich and warm within its own range with its specular
/// highlights kept, and foliage calmed towards olive.
pub fn editorial() -> Signature {
    Signature {
        tone: ToneSignature { black: 0.13, white: 0.96, median: 0.55, contrast: 0.25 },
        skin: Some(SkinSignature {
            l: 0.6,
            hue: 58.0,
            chroma: 0.075,
            richness: 0.62,
            relative_l: 0.05,
            specular: 0.04,
        }),
        bands: Vec::new(),
        foliage: Some(BandStat { hue: 122.0, chroma: 0.075, share: 0.1 }),
        background_chroma: None,
    }
}

fn percentile(v: &mut [f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let k = ((v.len() - 1) as f32 * p.clamp(0.0, 1.0)) as usize;
    *v.select_nth_unstable_by(k, f32::total_cmp).1
}

/// Displayed sRGB → Oklab planes (L, a, b).
pub(crate) fn display_lab(d: &DisplayImage) -> ImageRgbF32 {
    let lin = |c: u8| {
        let v = c as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let mut out = ImageRgbF32::new(d.width, d.height, ColorSpace::LinearRec2020);
    for (i, px) in d.rgba.as_chunks::<4>().0.iter().enumerate() {
        let (r, g, b) = (lin(px[0]), lin(px[1]), lin(px[2]));
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        out.r[i] = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
        out.g[i] = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
        out.b[i] = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
    }
    out
}

fn circular_mean(hues: &[f32]) -> f32 {
    let (s, c) = hues.iter().fold((0.0f32, 0.0f32), |(s, c), h| {
        let r = h.to_radians();
        (s + r.sin(), c + r.cos())
    });
    s.atan2(c).to_degrees().rem_euclid(360.0)
}

fn hue_diff(a: f32, b: f32) -> f32 {
    (a - b + 540.0).rem_euclid(360.0) - 180.0
}

/// Greenery by colour (as the pipeline's foliage control sees it).
fn is_foliage(hue: f32, chroma: f32) -> bool {
    (95.0..175.0).contains(&hue) && chroma > 0.03
}

/// Measure the displayed look. Masks (0–1) are at the display image's size.
pub(crate) fn measure(lab: &ImageRgbF32, skin: Option<&[f32]>, subject: Option<&[f32]>) -> Signature {
    let n = lab.len().max(1);
    let mut l = lab.r.clone();
    let tone = ToneSignature {
        black: percentile(&mut l, 0.005),
        white: percentile(&mut l, 0.995),
        median: percentile(&mut l, 0.5),
        contrast: percentile(&mut l, 0.75) - percentile(&mut l, 0.25),
    };
    let hue_of = |i: usize| lab.b[i].atan2(lab.g[i]).to_degrees().rem_euclid(360.0);
    let chroma_of = |i: usize| lab.g[i].hypot(lab.b[i]);

    // Skin.
    let skin = skin.and_then(|m| {
        let on: Vec<usize> = (0..n).filter(|&i| m[i] >= 0.6).collect();
        if on.len() < n / 400 || on.len() < 30 {
            return None;
        }
        let mut ls: Vec<f32> = on.iter().map(|&i| lab.r[i]).collect();
        let mut a: Vec<f32> = on.iter().map(|&i| lab.g[i]).collect();
        let mut b: Vec<f32> = on.iter().map(|&i| lab.b[i]).collect();
        let (sl, sa, sb) = (percentile(&mut ls, 0.5), percentile(&mut a, 0.5), percentile(&mut b, 0.5));
        let chroma = sa.hypot(sb);
        let (lo, hi) = chroma_band(sl);
        let specular = on.iter().filter(|&&i| lab.r[i] > sl + 0.18).count() as f32 / on.len() as f32;
        Some(SkinSignature {
            l: sl,
            hue: sb.atan2(sa).to_degrees().rem_euclid(360.0),
            chroma,
            richness: ((chroma - lo) / (hi - lo)).clamp(0.0, 1.0),
            relative_l: sl - tone.median,
            specular,
        })
    });

    // Colour bands: coloured pixels of the whole frame, so a reference without people
    // maps too.
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); 8];
    let mut foliage = Vec::new();
    for i in 0..n {
        let c = chroma_of(i);
        if c < 0.03 {
            continue;
        }
        let h = hue_of(i);
        let band = (0..8).min_by(|&x, &y| hue_diff(h, BAND_CENTERS[x]).abs().total_cmp(&hue_diff(h, BAND_CENTERS[y]).abs())).unwrap_or(0);
        members[band].push(i);
        if is_foliage(h, c) && subject.is_none_or(|s| s[i] < 0.5) {
            foliage.push(i);
        }
    }
    let stat = |idx: &[usize]| {
        let hues: Vec<f32> = idx.iter().map(|&i| hue_of(i)).collect();
        let mut cs: Vec<f32> = idx.iter().map(|&i| chroma_of(i)).collect();
        BandStat { hue: circular_mean(&hues), chroma: percentile(&mut cs, 0.5), share: idx.len() as f32 / n as f32 }
    };
    let bands = members.iter().map(|m| stat(m)).collect();
    let foliage = (foliage.len() > n / 50).then(|| stat(&foliage));
    let background_chroma = subject.and_then(|s| {
        let mut cs: Vec<f32> = (0..n).filter(|&i| s[i] < 0.5).map(chroma_of).collect();
        (cs.len() > n / 10).then(|| percentile(&mut cs, 0.5))
    });
    Signature { tone, skin, bands, foliage, background_chroma }
}

/// Move `v` by `delta` (whole steps, within `lo..=hi`); whether it moved.
fn nudge(v: &mut f32, delta: f32, lo: f32, hi: f32) -> bool {
    let next = (*v + delta).clamp(lo, hi).round();
    let moved = (next - *v).abs() >= 1.0;
    if moved {
        *v = next;
    }
    moved
}

/// One damped step of `rec` towards `target`, given the measurement `now` of `rec`.
/// Returns whether anything changed noticeably.
pub(crate) fn step_towards(rec: &mut Adjustments, now: &Signature, target: &Target, skin_crushed: bool) -> bool {
    let t = &target.signature.tone;
    let mut moved = false;
    // The built-in profile only adds depth and calm; a learned look may go either way.
    let learned = target.match_colours;
    let deepest = if learned { -70.0 } else { -45.0 };
    // Exposure, for learned looks only: lightness goes roughly as the cube root of
    // light, so a ratio r in L is 3·log2(r) stops.
    if target.match_exposure && now.tone.median > 0.05 {
        let ev = (3.0 * (t.median / now.tone.median).log2() * 0.6).clamp(-0.5, 0.5);
        if ev.abs() >= 0.05 {
            rec.exposure = ((rec.exposure + ev).clamp(-5.0, 5.0) * 100.0).round() / 100.0;
            moved = true;
        }
    }
    // Black point: rich blacks, but never at the cost of skin detail.
    let stops = |target: f32, now: f32| 3.0 * (target.max(0.02) / now.max(0.02)).log2();
    let black = stops(t.black, now.tone.black) / ENDS_STOPS * 100.0 * 0.7;
    if !(skin_crushed && black < 0.0) {
        moved |= nudge(&mut rec.tone.blacks, black, deepest, 30.0);
    } else if rec.tone.blacks < 0.0 {
        moved |= nudge(&mut rec.tone.blacks, 8.0, deepest, 30.0);
    }
    // White point.
    let white = stops(t.white, now.tone.white) / ENDS_STOPS * 100.0 * 0.7;
    moved |= nudge(&mut rec.tone.whites, white, -60.0, 40.0);
    // Midtone contrast: the S-curve adds it; flattening goes through Contrast.
    let gap = (t.contrast - now.tone.contrast) / now.tone.contrast.max(0.05);
    let s = &mut rec.curves.s_curve;
    if gap > 0.03 || (gap < 0.0 && s.enabled && s.amount > 0.0) {
        let before = if s.enabled { s.amount } else { 0.0 };
        let next = (before + gap * 110.0).clamp(0.0, 80.0).round();
        if (next - before).abs() >= 1.0 {
            s.enabled = next > 0.0;
            s.amount = next.max(if s.enabled { 1.0 } else { 50.0 });
            if !s.enabled {
                s.amount = 50.0;
            }
            moved = true;
        }
    } else if gap < -0.06 && learned {
        moved |= nudge(&mut rec.tone.contrast, gap * 60.0, -40.0, 40.0);
    }
    // Editorial always carries some midtone curve: dimension, not flat HDR.
    let s = &mut rec.curves.s_curve;
    if !learned && (!s.enabled || s.amount < EDITORIAL_MIN_CURVE) {
        (s.enabled, s.amount) = (true, EDITORIAL_MIN_CURVE);
        moved = true;
    }
    // Specular highlights on skin: kept, never smoothed away.
    if let (Some(ts), Some(ns)) = (&target.signature.skin, &now.skin) {
        if ns.specular < 0.6 * ts.specular && rec.texture.specular_balance > 0.0 {
            rec.texture.specular_balance = 0.0;
            moved = true;
        }
    }
    // Foliage: hue towards the target (+ = towards gold, a smaller Oklab hue), neon
    // greens calmed.
    if let (Some(tf), Some(nf)) = (&target.signature.foliage, &now.foliage) {
        let f = &mut rec.color.foliage;
        let turn = hue_diff(nf.hue, tf.hue) / FOLIAGE_TURN * 100.0 * 0.8;
        // Editorial: a slight turn towards olive, and only calming.
        let (hue_lo, hue_hi, sat_lo, sat_hi) = if learned { (-60.0, 60.0, -60.0, 40.0) } else { (0.0, 20.0, -30.0, 0.0) };
        moved |= nudge(&mut f.hue, turn, hue_lo, hue_hi);
        if nf.chroma > tf.chroma * 1.08 || (learned && nf.chroma < tf.chroma * 0.92) {
            moved |= nudge(&mut f.saturation, (tf.chroma / nf.chroma - 1.0) * 100.0 * 0.8, sat_lo, sat_hi);
        }
    }
    if target.match_colours {
        let bands = rec.color.hsl.bands_mut();
        for (i, band) in bands.into_iter().enumerate() {
            // Orange is skin's band: skin is matched on its own, within its depth.
            let (Some(tb), Some(nb)) = (target.signature.bands.get(i), now.bands.get(i)) else { continue };
            if i == 1 || tb.share < 0.02 || nb.share < 0.02 {
                continue;
            }
            moved |= nudge(&mut band.hue, hue_diff(tb.hue, nb.hue) / BAND_HUE_SHIFT * 100.0 * 0.7, -50.0, 50.0);
            moved |= nudge(&mut band.saturation, (tb.chroma / nb.chroma.max(1e-3) - 1.0) * 100.0 * 0.7, -50.0, 50.0);
        }
        if let (Some(tb), Some(nb)) = (target.signature.background_chroma, now.background_chroma) {
            moved |= nudge(&mut rec.color.background.saturation, (tb / nb.max(1e-3) - 1.0) * 100.0 * 0.7, -35.0, 35.0);
        }
    }
    moved
}

/// A learned style for the reference whose display is `lab`.
pub(crate) fn learned_style(name: &str, source: &Path, sig: Signature, lab: &ImageRgbF32) -> LearnedStyle {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("reference");
    let name = if name.trim().is_empty() { stem.to_string() } else { name.trim().to_string() };
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    LearnedStyle {
        id: format!("learned-{}-{created_at}", if slug.is_empty() { "style" } else { &slug }),
        name,
        source: source.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        created_at,
        signature: sig,
        palette: dominant_colors(lab),
    }
}

/// A render (scene-linear), its display as Oklab, and the Skin and Subject masks at
/// its size.
type Measured = (ImageRgbF32, ImageRgbF32, Option<Vec<f32>>, Option<Vec<f32>>);

/// The displayed look of `adjustments` at up to `side` px, as Oklab, with the Skin
/// and Subject masks at its size. The whole upright frame: the masks are made for it.
fn render_measured(
    engine: &Engine,
    loaded: &Loaded,
    adjustments: &Adjustments,
    side: u32,
) -> Result<Measured> {
    let whole = Adjustments { crop: Crop::default(), ..adjustments.clone() };
    let base = loaded.base(side, side);
    let prepared = engine.prepare(loaded, &whole)?;
    let rgb = prepared.with_inputs(|inputs| develop_rgb_with((*base).clone(), &loaded.raw.profile, &whole, inputs))?;
    let lab = display_lab(&to_display_srgb(&rgb));
    let (w, h) = (lab.width, lab.height);
    let mask = |t: MaskTarget| {
        engine
            .mask_plane(loaded, &whole, t)
            .ok()
            .flatten()
            .map(|m| analysis::fit(&m.data, m.width, m.height, w, h))
    };
    Ok((rgb, lab, mask(MaskTarget::Skin), mask(MaskTarget::Subject)))
}

/// Share of skin at or near black.
fn skin_crushed(lab: &ImageRgbF32, skin: Option<&[f32]>) -> bool {
    let Some(m) = skin else { return false };
    let on: Vec<usize> = (0..lab.len()).filter(|&i| m[i] >= 0.6).collect();
    !on.is_empty() && on.iter().filter(|&&i| lab.r[i] < 0.12).count() as f32 / on.len() as f32 > 0.2
}

impl Engine {
    /// Learn the look of the photo at `path` as edited by `adjustments` (a finished JPEG
    /// needs none) and keep it in the learned-styles store.
    pub fn learn_style(&self, path: &Path, adjustments: &Adjustments, name: &str) -> Result<LearnedStyle> {
        let loaded = self.load(path)?;
        let (_, lab, skin, subject) = render_measured(self, &loaded, adjustments, LEARN_SIDE)?;
        let signature = measure(&lab, skin.as_deref(), subject.as_deref());
        let style = learned_style(name, path, signature, &lab);
        let _guard = self.store_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut styles = load_learned(&self.data_dir)?;
        styles.push(style.clone());
        save_learned(&self.data_dir, styles)?;
        Ok(style)
    }

    /// Learned styles, oldest first.
    pub fn learned_styles(&self) -> Result<Vec<LearnedStyle>> {
        load_learned(&self.data_dir)
    }

    pub fn delete_learned_style(&self, id: &str) -> Result<()> {
        let _guard = self.store_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let styles = load_learned(&self.data_dir)?.into_iter().filter(|s| s.id != id).collect();
        save_learned(&self.data_dir, styles)
    }

    /// Custom presets, oldest first.
    pub fn presets(&self) -> Result<Vec<Preset>> {
        load_presets(&self.data_dir)
    }

    /// Save the look of `adjustments` as a preset named `name`.
    pub fn save_preset(&self, name: &str, adjustments: &Adjustments) -> Result<Preset> {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let name = if name.trim().is_empty() { "My preset".to_string() } else { name.trim().to_string() };
        let preset = Preset { id: format!("preset-{created_at}"), name, created_at: created_at / 1_000_000_000, adjustments: look_of(adjustments) };
        let _guard = self.store_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut presets = load_presets(&self.data_dir)?;
        presets.push(preset.clone());
        save_presets(&self.data_dir, presets)?;
        Ok(preset)
    }

    pub fn delete_preset(&self, id: &str) -> Result<()> {
        let _guard = self.store_lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let presets = load_presets(&self.data_dir)?.into_iter().filter(|p| p.id != id).collect();
        save_presets(&self.data_dir, presets)
    }

    /// Move `start` towards `target` by measuring its render and adjusting (tone, skin,
    /// foliage, and for learned looks exposure, HSL bands and background).
    pub(crate) fn fit_to_target(&self, loaded: &Loaded, start: &Adjustments, target: &Target) -> Result<Adjustments> {
        let mut rec = start.clone();
        for pass in 0..FIT_PASSES {
            let (rgb, lab, skin, subject) = render_measured(self, loaded, &rec, FIT_SIDE)?;
            let now = measure(&lab, skin.as_deref(), subject.as_deref());
            if pass == 0 {
                self.fit_skin(&mut rec, &rgb, skin.as_deref(), &now, target);
            }
            if !step_towards(&mut rec, &now, target, skin_crushed(&lab, skin.as_deref())) && pass > 0 {
                break;
            }
        }
        Ok(rec)
    }

    /// Skin towards the target's hue and richness for its own depth, and its brightness
    /// relative to the frame, gently (deep skin is never lifted towards light skin).
    fn fit_skin(&self, rec: &mut Adjustments, rgb: &ImageRgbF32, skin: Option<&[f32]>, now: &Signature, target: &Target) {
        let (Some(ts), Some(mask), Some(ns)) = (&target.signature.skin, skin, &now.skin) else { return };
        // The mask was made at the display size, which is the render's size.
        if mask.len() != rgb.len() {
            return;
        }
        let current = rec.local.iter().find(|l| l.mask == MaskTarget::Skin).copied();
        // A step at a time: grey skin is warmed back, not repainted.
        let richness = ts.richness.min(ns.richness + 0.35);
        let colour = guidance::skin_towards(rgb, mask, current.as_ref(), ts.hue, richness)
            .map(|(w, t, s)| (w.clamp(-40.0, 40.0), t.clamp(-25.0, 25.0), s.clamp(-25.0, 25.0)));
        let gap = ns.relative_l - ts.relative_l;
        let exposure = if gap > 0.08 {
            -0.2
        } else if gap < -0.1 {
            if ns.l < 0.45 { 0.1 } else { 0.15 }
        } else {
            0.0
        };
        if colour.is_none() && exposure == 0.0 {
            return;
        }
        let l = match rec.local.iter_mut().find(|l| l.mask == MaskTarget::Skin) {
            Some(l) => l,
            None => {
                rec.local.push(LocalAdjustment { mask: MaskTarget::Skin, ..Default::default() });
                rec.local.last_mut().expect("just pushed")
            }
        };
        if let Some((w, t, s)) = colour {
            (l.warmth, l.tint, l.saturation) = (w, t, s);
        }
        if exposure != 0.0 {
            l.exposure = (l.exposure + exposure).clamp(-0.5, 0.5);
        }
    }
}

// ---- Persistent stores in the user's data folder.

/// `~/.epikos` (or `$EPIKOS_DATA_DIR`): learned styles and custom presets, kept across
/// sessions and app updates.
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("EPIKOS_DATA_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".epikos")
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LearnedFile {
    version: u32,
    styles: Vec<LearnedStyle>,
}

/// A saved look: the look settings of an edit (no framing, white balance or exposure,
/// which belong to each photo).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub adjustments: Adjustments,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PresetFile {
    version: u32,
    presets: Vec<Preset>,
}

fn read_json<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| Error::InvalidImage {
            reason: format!("{} is not readable ({e}); it was left as it is", path.display()),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

/// Write beside, then rename: a crash never leaves a half-written store.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.partial");
    let text = serde_json::to_string_pretty(value).map_err(|e| Error::InvalidImage { reason: e.to_string() })?;
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn learned_path(dir: &Path) -> PathBuf {
    dir.join("learned_styles.json")
}

pub(crate) fn presets_path(dir: &Path) -> PathBuf {
    dir.join("presets.json")
}

pub(crate) fn load_learned(dir: &Path) -> Result<Vec<LearnedStyle>> {
    Ok(read_json::<LearnedFile>(&learned_path(dir))?.styles)
}

pub(crate) fn save_learned(dir: &Path, styles: Vec<LearnedStyle>) -> Result<()> {
    write_json(&learned_path(dir), &LearnedFile { version: 1, styles })
}

pub(crate) fn load_presets(dir: &Path) -> Result<Vec<Preset>> {
    Ok(read_json::<PresetFile>(&presets_path(dir))?.presets)
}

pub(crate) fn save_presets(dir: &Path, presets: Vec<Preset>) -> Result<()> {
    write_json(&presets_path(dir), &PresetFile { version: 1, presets })
}

/// The look part of an edit, for a preset.
pub(crate) fn look_of(a: &Adjustments) -> Adjustments {
    let mut look = Adjustments {
        tone: a.tone,
        local: a.local.clone(),
        texture: a.texture.clone(),
        color: a.color.clone(),
        atmosphere: a.atmosphere.clone(),
        curves: a.curves.clone(),
        split_toning: a.split_toning,
        finishing: a.finishing,
        style: a.style.clone(),
        lut: a.lut.clone(),
        ..Adjustments::default()
    };
    // Placed lights belong to one photo's composition.
    look.atmosphere.lights.clear();
    look.atmosphere.shaft_auto = true;
    look
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab_image(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let [l, a, b] = f(x, y);
                (img.r[i], img.g[i], img.b[i]) = (l, a, b);
            }
        }
        img
    }

    #[test]
    fn signature_reads_tone_foliage_and_skin() {
        // Left half: a lightness ramp. Right half: greenery, and a skin patch.
        let lab = lab_image(100, 50, |x, y| {
            if x < 50 {
                [x as f32 / 50.0, 0.0, 0.0]
            } else if y < 25 {
                let h = 140f32.to_radians();
                [0.5, 0.1 * h.cos(), 0.1 * h.sin()]
            } else {
                let h = 60f32.to_radians();
                [0.6, 0.07 * h.cos(), 0.07 * h.sin()]
            }
        });
        let skin: Vec<f32> = (0..lab.len()).map(|i| ((i % 100) >= 50 && i / 100 >= 25) as u8 as f32).collect();
        let s = measure(&lab, Some(&skin), None);
        assert!(s.tone.black < 0.02 && s.tone.white > 0.55, "{:?}", s.tone);
        let f = s.foliage.expect("foliage");
        assert!((f.hue - 140.0).abs() < 1.0 && (f.chroma - 0.1).abs() < 0.005, "{f:?}");
        let k = s.skin.expect("skin");
        assert!((k.hue - 60.0).abs() < 1.0 && (k.chroma - 0.07).abs() < 0.005, "{k:?}");
        assert!(k.richness > 0.3 && k.richness < 0.7, "{k:?}");
    }

    #[test]
    fn a_step_deepens_washed_blacks_calms_neon_foliage_and_adds_midtone_curve() {
        let mut rec = Adjustments::default();
        let mut now = editorial();
        now.tone.black = 0.25; // washed out
        now.tone.contrast = 0.15; // flat
        now.foliage = Some(BandStat { hue: 145.0, chroma: 0.14, share: 0.2 }); // neon green
        let target = editorial();
        let t = Target { name: "Editorial".into(), signature: &target, match_exposure: false, match_colours: false };
        assert!(step_towards(&mut rec, &now, &t, false));
        assert!(rec.tone.blacks < -20.0, "blacks {}", rec.tone.blacks);
        assert!(rec.curves.s_curve.enabled && rec.curves.s_curve.amount > 20.0, "{:?}", rec.curves.s_curve);
        assert!(rec.color.foliage.hue > 0.0, "towards olive / gold: {}", rec.color.foliage.hue);
        assert!(rec.color.foliage.saturation < 0.0, "neon calmed: {}", rec.color.foliage.saturation);
        assert_eq!(rec.tone.shadows, 0.0, "no flat shadow lift");

        // Skin already crushed: blacks are not deepened.
        let mut guarded = Adjustments::default();
        step_towards(&mut guarded, &now, &t, true);
        assert!(guarded.tone.blacks >= 0.0, "{}", guarded.tone.blacks);
    }

    #[test]
    fn stores_round_trip_and_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("epikos-learn-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(load_learned(&dir).unwrap().is_empty());
        let lab = lab_image(20, 20, |x, _| [x as f32 / 20.0, 0.02, 0.03]);
        let style = learned_style("Soft Editorial ", Path::new("/photos/ref.JPG"), editorial(), &lab);
        assert_eq!(style.name, "Soft Editorial");
        assert!(style.id.starts_with("learned-soft-editorial-"));
        save_learned(&dir, vec![style.clone()]).unwrap();
        let back = load_learned(&dir).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].signature, style.signature);
        assert!(!dir.join("learned_styles.json.partial").exists());

        let a = Adjustments {
            exposure: 1.5,
            tone: epikos_sidecar::Tone { blacks: -20.0, ..Default::default() },
            ..Default::default()
        };
        let preset = Preset { id: "p1".into(), name: "Mine".into(), created_at: 1, adjustments: look_of(&a) };
        save_presets(&dir, vec![preset]).unwrap();
        let p = load_presets(&dir).unwrap();
        assert_eq!(p[0].adjustments.tone.blacks, -20.0);
        assert_eq!(p[0].adjustments.exposure, 0.0, "exposure belongs to each photo");
        fs::remove_dir_all(&dir).unwrap();
    }
}
