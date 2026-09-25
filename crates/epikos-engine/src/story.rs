//! PRD Section 2.2: smart batch and event intelligence.
//!
//! - **Story-arc grouping**: a fast pass over a folder (embedded previews and EXIF, no
//!   RAW decode) splits the shoot into groups taken under similar conditions: a new
//!   group starts at a long time gap, a change of light (scene brightness from the
//!   exposure settings, or colour cast), or a change of place (GPS). Each group gets a
//!   hero frame (the one closest to the group's average) and a hero palette.
//! - **Hero sync with per-frame calibration**: the hero's look (Step 2 contrast and
//!   colour, Step 3 mask edits, Steps 4–8 and style) is copied to every frame in the
//!   group, while each frame keeps its own white balance, lens, framing and noise
//!   settings and is adapted individually: exposure is matched
//!   on skin (or the whole frame), the hero's white-balance *shift* is carried over,
//!   skin protection rises for frames with more skin, and micro-contrast is scaled to
//!   each frame's own detail. Every changed sidecar is backed up so a sync can be undone.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use epikos_core::{CaptureMetadata, ColorSpace, ImageRgbF32, Result};
use epikos_decode::{embedded_thumbnail, read_metadata};
use epikos_pipeline::{oklab_planes, skin_likelihood};
use epikos_sidecar::{sidecar_json_path, sidecar_xmp_path, Adjustments, WbMode};
use rayon::prelude::*;
use serde::Serialize;

use crate::analysis::{dominant_colors, Swatch};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryArc {
    pub groups: Vec<ShotGroup>,
    pub frames: Vec<FrameSummary>,
    pub analysis_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotGroup {
    pub id: usize,
    /// e.g. "12:04–12:31 · 8 photos".
    pub label: String,
    pub frames: Vec<String>,
    pub hero: String,
    pub palette: Vec<Swatch>,
    /// Why this group starts here (empty for the first).
    pub split_reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSummary {
    pub path: String,
    pub name: String,
    pub captured: Option<String>,
    /// Scene brightness from the exposure settings (EV at ISO 100).
    pub scene_ev: Option<f32>,
    /// Mean Oklab lightness and colour cast of the camera preview.
    pub lightness: f32,
    pub cast: [f32; 2],
    pub gps: Option<[f64; 2]>,
    /// % of the preview detected as skin.
    pub skin: f32,
    pub group: usize,
    /// Set when the file couldn't be summarised; it then joins no group.
    pub error: Option<String>,
}

/// A new group starts after this long without a shot.
const GAP_S: i64 = 30 * 60;
/// …or when scene brightness changes by this many stops,
const EV_JUMP: f32 = 2.5;
/// …or the colour cast by this much (Oklab a/b),
const CAST_JUMP: f32 = 0.035;
/// …or the camera moves this far.
const MOVE_KM: f64 = 2.0;

struct Sample {
    summary: FrameSummary,
    time_s: Option<i64>,
    pixels: Vec<[f32; 3]>,
}

pub(crate) fn story_arc(files: &[PathBuf]) -> StoryArc {
    let t = Instant::now();
    let mut samples: Vec<Sample> = files.par_iter().map(|p| summarize(p)).collect();
    // Shooting order: capture time, then file name for frames without one.
    samples.sort_by(|a, b| match (a.time_s, b.time_s) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.summary.name.cmp(&b.summary.name),
    });

    let mut groups: Vec<(Vec<usize>, String)> = Vec::new();
    let mut prev: Option<usize> = None;
    for i in 0..samples.len() {
        if samples[i].summary.error.is_some() {
            continue;
        }
        let reason = prev.and_then(|p| split_reason(&samples[p], &samples[i]));
        match (prev, reason) {
            (None, _) => groups.push((vec![i], String::new())),
            (Some(_), Some(r)) => groups.push((vec![i], r)),
            (Some(_), None) => groups.last_mut().expect("a group exists").0.push(i),
        }
        prev = Some(i);
    }

    // A shoot spread over several days gets dates in its group labels.
    let mut days: Vec<&str> = samples
        .iter()
        .filter_map(|x| x.summary.captured.as_deref()?.get(0..10))
        .collect();
    days.dedup();
    let multi_day = days.len() > 1;
    let mut out_groups = Vec::new();
    for (id, (members, reason)) in groups.into_iter().enumerate() {
        for &m in &members {
            samples[m].summary.group = id;
        }
        let hero = hero_of(&samples, &members);
        let mut pooled: Vec<[f32; 3]> = Vec::new();
        for &m in &members {
            pooled.extend(samples[m].pixels.iter().step_by((members.len() / 4).max(1)));
        }
        let mut lab = ImageRgbF32::new(pooled.len().max(1) as u32, 1, ColorSpace::LinearRec2020);
        for (k, p) in pooled.iter().enumerate() {
            (lab.r[k], lab.g[k], lab.b[k]) = (p[0], p[1], p[2]);
        }
        let times: Vec<&str> = members
            .iter()
            .filter_map(|&m| samples[m].summary.captured.as_deref())
            .collect();
        let clock = |s: &str| s.get(11..16).unwrap_or("").to_string();
        let date = |s: &str| {
            if multi_day {
                format!("{} ", s.get(0..10).unwrap_or("").replace(':', "-"))
            } else {
                String::new()
            }
        };
        let span = match (times.first(), times.last()) {
            (Some(a), Some(b)) if a != b => format!("{}{}–{} · ", date(a), clock(a), clock(b)),
            (Some(a), _) => format!("{}{} · ", date(a), clock(a)),
            _ => String::new(),
        };
        out_groups.push(ShotGroup {
            id,
            label: format!(
                "{span}{} photo{}",
                members.len(),
                if members.len() == 1 { "" } else { "s" }
            ),
            frames: members
                .iter()
                .map(|&m| samples[m].summary.path.clone())
                .collect(),
            hero: samples[hero].summary.path.clone(),
            palette: dominant_colors(&lab),
            split_reason: reason,
        });
    }
    StoryArc {
        groups: out_groups,
        frames: samples.into_iter().map(|s| s.summary).collect(),
        analysis_ms: t.elapsed().as_millis() as u64,
    }
}

fn summarize(path: &Path) -> Sample {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let meta = read_metadata(path).unwrap_or_default();
    let thumb = embedded_thumbnail(path, 256);
    let mut summary = FrameSummary {
        path: path.to_string_lossy().into_owned(),
        name,
        captured: meta.date_time_original.clone(),
        scene_ev: scene_ev(&meta),
        lightness: 0.0,
        cast: [0.0, 0.0],
        gps: gps_degrees(&meta),
        skin: 0.0,
        group: 0,
        error: None,
    };
    let mut pixels = Vec::new();
    match thumb {
        Ok(img) => {
            let rgb = srgb8_to_rec2020(&img);
            let lab = oklab_planes(&rgb);
            let skin = skin_likelihood(&rgb);
            let n = lab.r.len() as f32;
            summary.lightness = lab.r.iter().sum::<f32>() / n;
            summary.cast = [lab.g.iter().sum::<f32>() / n, lab.b.iter().sum::<f32>() / n];
            summary.skin = 100.0 * skin.iter().filter(|&&s| s > 0.5).count() as f32 / n;
            pixels = (0..lab.r.len())
                .step_by(16)
                .map(|i| [lab.r[i], lab.g[i], lab.b[i]])
                .collect();
        }
        Err(e) => summary.error = Some(format!("no preview: {e}")),
    }
    Sample {
        time_s: meta.date_time_original.as_deref().and_then(exif_seconds),
        summary,
        pixels,
    }
}

fn split_reason(a: &Sample, b: &Sample) -> Option<String> {
    if let (Some(x), Some(y)) = (a.time_s, b.time_s) {
        if y - x > GAP_S {
            return Some(format!("{} without shooting", duration_words(y - x)));
        }
    }
    if let (Some(x), Some(y)) = (a.summary.scene_ev, b.summary.scene_ev) {
        if (y - x).abs() > EV_JUMP {
            return Some(format!("light changed by {:.1} stops", (y - x).abs()));
        }
    }
    let [a1, b1] = a.summary.cast;
    let [a2, b2] = b.summary.cast;
    if (a1 - a2).hypot(b1 - b2) > CAST_JUMP {
        return Some("colour of the light changed".into());
    }
    if let (Some(p), Some(q)) = (a.summary.gps, b.summary.gps) {
        let km = haversine_km(p, q);
        if km > MOVE_KM {
            return Some(format!("moved {km:.1} km"));
        }
    }
    None
}

/// The frame closest to the group's average look (lightness, cast, scene brightness).
fn hero_of(samples: &[Sample], members: &[usize]) -> usize {
    let n = members.len() as f32;
    let mean = |f: &dyn Fn(&FrameSummary) -> f32| {
        members.iter().map(|&m| f(&samples[m].summary)).sum::<f32>() / n
    };
    let (ml, ma, mb) = (
        mean(&|s| s.lightness),
        mean(&|s| s.cast[0]),
        mean(&|s| s.cast[1]),
    );
    let mev = mean(&|s| s.scene_ev.unwrap_or(0.0));
    *members
        .iter()
        .min_by(|&&x, &&y| {
            let d = |s: &FrameSummary| {
                ((s.lightness - ml) / 0.1).powi(2)
                    + ((s.cast[0] - ma) / 0.02).powi(2)
                    + ((s.cast[1] - mb) / 0.02).powi(2)
                    + ((s.scene_ev.unwrap_or(0.0) - mev) / 2.0).powi(2)
            };
            d(&samples[x].summary).total_cmp(&d(&samples[y].summary))
        })
        .expect("groups are never empty")
}

/// EV at ISO 100 from aperture, shutter and ISO: higher means a brighter scene.
fn scene_ev(m: &CaptureMetadata) -> Option<f32> {
    let n = m.f_number.map(|(a, b)| a as f32 / b.max(1) as f32)?;
    let t = m.exposure_time.map(|(a, b)| a as f32 / b.max(1) as f32)?;
    let iso = m.iso? as f32;
    (n > 0.0 && t > 0.0 && iso > 0.0).then(|| (n * n / t).log2() - (iso / 100.0).log2())
}

fn gps_degrees(m: &CaptureMetadata) -> Option<[f64; 2]> {
    let g = m.gps.as_ref()?;
    let deg = |v: [(u32, u32); 3], r: &str, neg: &str| {
        let f = |(n, d): (u32, u32)| if d == 0 { 0.0 } else { n as f64 / d as f64 };
        let x = f(v[0]) + f(v[1]) / 60.0 + f(v[2]) / 3600.0;
        if r.trim() == neg {
            -x
        } else {
            x
        }
    };
    Some([
        deg(g.latitude?, g.latitude_ref.as_deref()?, "S"),
        deg(g.longitude?, g.longitude_ref.as_deref()?, "W"),
    ])
}

/// "45 min", "3 h", "12 days".
fn duration_words(seconds: i64) -> String {
    let min = seconds / 60;
    match min {
        0..=119 => format!("{min} min"),
        120..=2879 => format!("{} h", min / 60),
        _ => format!("{} days", min / 1440),
    }
}

fn haversine_km(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (la1, lo1, la2, lo2) = (
        a[0].to_radians(),
        a[1].to_radians(),
        b[0].to_radians(),
        b[1].to_radians(),
    );
    let h = ((la2 - la1) / 2.0).sin().powi(2)
        + la1.cos() * la2.cos() * ((lo2 - lo1) / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.sqrt().asin()
}

/// EXIF "YYYY:MM:DD HH:MM:SS" → seconds (local clock; only differences matter).
pub(crate) fn exif_seconds(s: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, se) = (num(11..13)?, num(14..16)?, num(17..19)?);
    // Days from civil (Howard Hinnant).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se)
}

/// Camera JPEG (sRGB) → linear Rec.2020 planes.
fn srgb8_to_rec2020(img: &image::RgbImage) -> ImageRgbF32 {
    let (w, h) = img.dimensions();
    let mut out = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
    let lin = |c: u8| {
        let v = c as f32 / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    for (i, p) in img.pixels().enumerate() {
        let (r, g, b) = (lin(p[0]), lin(p[1]), lin(p[2]));
        out.r[i] = 0.627_404 * r + 0.329_283 * g + 0.043_313 * b;
        out.g[i] = 0.069_097 * r + 0.919_54 * g + 0.011_362 * b;
        out.b[i] = 0.016_391 * r + 0.088_013 * g + 0.895_595 * b;
    }
    out
}

// ---- Hero sync ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub frames: Vec<SyncedFrame>,
    pub skipped: Vec<SkippedFrame>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncedFrame {
    pub path: String,
    /// Exposure change applied to match the hero, in stops.
    pub exposure_delta: f32,
    /// Whether the hero's white-balance shift was carried over.
    pub white_balance_shifted: bool,
    /// Factor applied to clarity and micro-texture for this frame's own detail.
    pub texture_scale: f32,
    /// Style skin protection used for this frame.
    pub skin_protection: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedFrame {
    pub path: String,
    pub reason: String,
}

/// What per-frame calibration measures on a frame (Steps 1–2 develop).
pub(crate) struct FrameStats {
    /// Median EV of skin if there is enough, else of the whole frame.
    pub key_ev: f32,
    pub detail: f32,
    pub skin: f32,
    pub as_shot: Option<(f32, f32)>,
}

/// The hero's look carried to `target`, adapted to `target`'s own measurements.
pub(crate) fn synced_adjustments(
    hero: &Adjustments,
    hero_stats: &FrameStats,
    target: &Adjustments,
    target_stats: &FrameStats,
) -> (Adjustments, SyncedFrame) {
    let mut a = target.clone();
    // The look: Steps 4–8 and the style. Placed lights and a placed shaft source
    // belong to the hero's composition, not to other frames.
    a.texture = hero.texture.clone();
    a.color = hero.color.clone();
    a.atmosphere = hero.atmosphere.clone();
    a.atmosphere.lights.clear();
    a.atmosphere.shaft_auto = true;
    a.curves = hero.curves.clone();
    a.split_toning = hero.split_toning;
    a.finishing = hero.finishing;
    a.style = hero.style.clone();
    // Step 2's look (contrast, ends, colour) travels; highlight and shadow recovery are
    // about each frame's own light, and straightening about its own framing.
    a.tone.contrast = hero.tone.contrast;
    a.tone.whites = hero.tone.whites;
    a.tone.blacks = hero.tone.blacks;
    a.tone.vibrance = hero.tone.vibrance;
    a.tone.saturation = hero.tone.saturation;
    // Step 3 edits name a region ("eyes", "background"), which each frame finds itself.
    a.local = hero.local.clone();

    // Exposure: bring the frame's key (skin, else the whole frame) to the hero's.
    let delta = (hero_stats.key_ev - target_stats.key_ev).clamp(-2.0, 2.0);
    a.exposure = (target.exposure + delta).clamp(-5.0, 5.0);

    // White balance: carry the hero's shift from its own as-shot (in mireds), so a
    // warmed-up hero warms every frame by the same amount of light change.
    let mut shifted = false;
    if hero.white_balance.mode == WbMode::Custom {
        if let (Some((ht, htint)), Some((tt, ttint))) = (hero_stats.as_shot, target_stats.as_shot) {
            let mired = |k: f32| 1e6 / k.max(1000.0);
            let shift = mired(hero.white_balance.temperature) - mired(ht);
            a.white_balance.mode = WbMode::Custom;
            a.white_balance.temperature = (1e6 / (mired(tt) + shift)).clamp(2000.0, 25_000.0);
            a.white_balance.tint = (ttint + (hero.white_balance.tint - htint)).clamp(-150.0, 150.0);
            shifted = true;
        }
    }

    // Skin preservation: frames with more skin get at least a firm protection.
    if target_stats.skin > 2.0 {
        a.style.skin_protection = a.style.skin_protection.max(70.0);
        a.color.skin_protection = a.color.skin_protection.max(40.0);
    }

    // Micro-contrast: frames with more native detail need less boost, and vice versa.
    let scale = if target_stats.detail > 1e-5 {
        (hero_stats.detail / target_stats.detail).clamp(0.6, 1.5)
    } else {
        1.0
    };
    for v in [&mut a.texture.clarity, &mut a.texture.micro_texture] {
        if *v > 0.0 {
            *v = (*v * scale).min(100.0);
        }
    }
    let report = SyncedFrame {
        path: String::new(),
        exposure_delta: delta,
        white_balance_shifted: shifted,
        texture_scale: scale,
        skin_protection: a.style.skin_protection,
    };
    (a, report)
}

/// Where a frame's pre-sync sidecar is kept ("none" if it had none).
pub(crate) fn presync_path(raw: &Path) -> PathBuf {
    let mut p = sidecar_json_path(raw).into_os_string();
    p.push(".presync");
    PathBuf::from(p)
}

pub(crate) fn back_up(raw: &Path) -> Result<()> {
    let json = sidecar_json_path(raw);
    let previous = fs::read_to_string(&json).unwrap_or_else(|_| "none".into());
    fs::write(presync_path(raw), previous)?;
    Ok(())
}

/// Restore the pre-sync sidecar. Returns false when there is no backup.
pub(crate) fn restore(raw: &Path) -> Result<Option<String>> {
    let backup = presync_path(raw);
    let Ok(previous) = fs::read_to_string(&backup) else {
        return Ok(None);
    };
    fs::remove_file(&backup)?;
    Ok(Some(previous))
}

/// The XMP we wrote, which undo may remove (never another app's).
pub(crate) fn remove_own_xmp(raw: &Path) {
    let xmp = sidecar_xmp_path(raw);
    if fs::read_to_string(&xmp).is_ok_and(|s| s.contains("https://epikos.raw/ns/1.0/")) {
        let _ = fs::remove_file(xmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_time_and_scene_ev() {
        assert_eq!(duration_words(45 * 60), "45 min");
        assert_eq!(duration_words(5 * 3600), "5 h");
        assert_eq!(duration_words(245 * 86_400 + 10), "245 days");
        let a = exif_seconds("2026:09:20 12:04:56").unwrap();
        let b = exif_seconds("2026:09:20 12:40:56").unwrap();
        assert_eq!(b - a, 36 * 60);
        assert_eq!(
            exif_seconds("2026:03:01 00:00:00").unwrap()
                - exif_seconds("2026:02:28 00:00:00").unwrap(),
            86_400
        );
        let m = CaptureMetadata {
            f_number: Some((56, 10)),
            exposure_time: Some((1, 125)),
            iso: Some(500),
            ..Default::default()
        };
        // log2(5.6² × 125) − log2(5) ≈ 11.9 − 2.3 = 9.6: bright shade / overcast daylight.
        assert!((scene_ev(&m).unwrap() - 9.62).abs() < 0.05);
    }

    #[test]
    fn sync_carries_the_look_and_adapts_each_frame() {
        let mut hero = Adjustments::default();
        hero.style.id = "dark-melanin-glow".into();
        hero.style.skin_protection = 50.0;
        hero.texture.clarity = 30.0;
        hero.white_balance.mode = WbMode::Custom;
        hero.white_balance.temperature = 5000.0; // as-shot 5500: warmed by ~18 mired
        hero.white_balance.tint = 5.0;
        hero.atmosphere.lights.push(Default::default());
        let hs = FrameStats {
            key_ev: -1.0,
            detail: 0.01,
            skin: 8.0,
            as_shot: Some((5500.0, 0.0)),
        };
        let mut target = Adjustments::default();
        target.lens.distortion.enabled = true;
        target.exposure = 0.3;
        let ts = FrameStats {
            key_ev: -2.0,
            detail: 0.02,
            skin: 5.0,
            as_shot: Some((4000.0, 2.0)),
        };

        let (a, r) = synced_adjustments(&hero, &hs, &target, &ts);
        assert_eq!(a.style.id, "dark-melanin-glow");
        assert!(
            a.lens.distortion.enabled,
            "the frame keeps its own lens settings"
        );
        assert!(
            a.atmosphere.lights.is_empty(),
            "placed lights stay with the hero"
        );
        assert!(
            (a.exposure - 1.3).abs() < 1e-5,
            "one stop brighter to match the hero's skin"
        );
        // Same mired shift from its own as-shot 4000 K: warmer than 4000 K.
        assert!(a.white_balance.temperature < 4000.0 && a.white_balance.temperature > 3600.0);
        assert!((a.white_balance.tint - 7.0).abs() < 1e-5);
        assert!((r.texture_scale - 0.6).abs() < 1e-5 && (a.texture.clarity - 18.0).abs() < 1e-4);
        assert_eq!(
            a.style.skin_protection, 70.0,
            "a frame with skin gets firm protection"
        );
    }
}
