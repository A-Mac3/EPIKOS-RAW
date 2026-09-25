//! PRD Section 2.1: per-image context analysis — genre, lighting and skin.
//!
//! Everything here is measured from signals EPIKOS already has: the Steps 1–2
//! develop, the subject / sky masks and depth map (when their models are installed),
//! the skin detector and the capture EXIF. Genre scores are **rules over those
//! measurements**, not a trained classifier, and the report says so; each score comes
//! with the evidence that produced it.

use std::time::Instant;

use epikos_core::{resize_plane, CaptureMetadata, ImageRgbF32};
use rayon::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneAnalysis {
    /// Best-matching genres, strongest first (score 0–1); two strong ones = hybrid.
    pub genres: Vec<GenreScore>,
    pub lighting: Lighting,
    /// Present when enough skin is detected.
    pub skin: Option<SkinReport>,
    pub composition: Composition,
    /// Dominant colours, most prominent first.
    pub palette: Vec<Swatch>,
    /// What the analysis couldn't measure (e.g. a model isn't installed).
    pub limits: Vec<String>,
    pub analysis_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenreScore {
    pub id: &'static str,
    pub label: &'static str,
    pub score: f32,
    /// The measurements behind the score.
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lighting {
    /// The camera's white-balance estimate of the light, in kelvin.
    pub color_temperature: Option<f32>,
    /// Scene range from the 0.5th to the 99.5th luminance percentile, in stops.
    pub dynamic_range_ev: f32,
    /// % of the frame brighter than display white.
    pub highlights_clipped: f32,
    /// % of the frame more than ~7 stops below mid-grey.
    pub shadows_crushed: f32,
    /// "low-key", "mid-key" or "high-key".
    pub key: &'static str,
    /// 0 (diffuse) … 1 (hard, direct light).
    pub hardness: f32,
    pub hardness_label: &'static str,
    /// Where the light falls from on the subject, e.g. "from the upper left".
    pub direction: Option<String>,
    pub backlit: bool,
    /// 0 (clear) … 1 (dense fog), from the dark-channel prior.
    pub haze: f32,
    pub haze_label: &'static str,
    /// Share of the ground that is bright, colourless and flat-lit (snow-like).
    pub snow: f32,
    /// From the capture time, when known.
    pub time_of_day: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinReport {
    /// % of the frame.
    pub coverage: f32,
    /// Monk Skin Tone scale estimate (1 lightest … 10 deepest) under this exposure.
    pub tone_depth: f32,
    pub tone_label: &'static str,
    pub undertone: &'static str,
    /// 0 … 1: share of skin with specular shine.
    pub shine: f32,
    pub shine_label: &'static str,
    /// 0 … 1: fine structure on skin (pores, beard, lines).
    pub texture: f32,
    pub texture_label: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Composition {
    /// % of the frame, when the subject model is installed.
    pub subject_coverage: Option<f32>,
    pub sky_coverage: Option<f32>,
    /// 0 … 1: spread of distances in the frame, when depth is available.
    pub depth_range: Option<f32>,
    /// 0 … 1: how strongly straight horizontal / vertical lines dominate.
    pub line_strength: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Swatch {
    pub hex: String,
    /// Share of the sampled pixels, 0–1.
    pub weight: f32,
}

/// Model outputs, already at the analysis image's size (0–1).
pub(crate) struct Planes {
    pub subject: Option<Vec<f32>>,
    pub sky: Option<Vec<f32>>,
    pub depth: Option<Vec<f32>>,
}

/// Oklab lightness of the Monk Skin Tone swatches 1–10 (measured from the published
/// sRGB values), for placing detected skin on that scale.
const MONK_L: [f32; 10] = [
    0.951, 0.934, 0.941, 0.893, 0.811, 0.616, 0.509, 0.407, 0.321, 0.264,
];

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn percentile(v: &mut [f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let k = ((v.len() - 1) as f32 * p.clamp(0.0, 1.0)) as usize;
    *v.select_nth_unstable_by(k, f32::total_cmp).1
}

/// `rgb`: the Steps 1–2 develop (scene-linear Rec.2020, upright). `lab`: the same in
/// Oklab. `skin`: skin likelihood per pixel.
pub(crate) fn analyze(
    rgb: &ImageRgbF32,
    lab: &ImageRgbF32,
    skin: &[f32],
    planes: &Planes,
    meta: &CaptureMetadata,
    as_shot_kelvin: Option<f32>,
    started: Instant,
) -> SceneAnalysis {
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    let n = w * h;
    let y: Vec<f32> = (0..n)
        .into_par_iter()
        .map(|i| (0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(1e-6))
        .collect();
    let ev: Vec<f32> = y.par_iter().map(|v| (v / 0.18).log2()).collect();
    let mut limits = Vec::new();

    // ---- Exposure and key.
    let mut evs = ev.clone();
    let dynamic_range_ev = percentile(&mut evs, 0.995) - percentile(&mut evs, 0.005);
    let over = (0..n)
        .filter(|&i| rgb.r[i].max(rgb.g[i]).max(rgb.b[i]) > 1.0)
        .count();
    let crushed = y.iter().filter(|&&v| v < 0.18 / 128.0).count();
    let mut ls = lab.r.clone();
    let median_l = percentile(&mut ls, 0.5);
    let key = if median_l < 0.38 {
        "low-key"
    } else if median_l > 0.68 {
        "high-key"
    } else {
        "mid-key"
    };

    // ---- Regions.
    let subject = planes.subject.as_deref();
    let sky = planes.sky.as_deref();
    let cover = |p: Option<&[f32]>| p.map(|m| m.iter().sum::<f32>() / n as f32);
    let (subject_cov, sky_cov) = (cover(subject), cover(sky));
    if subject.is_none() {
        limits.push("Subject model not installed: lighting direction and portrait genres are estimated from skin only.".into());
    }
    let skin_cov = skin.iter().filter(|&&s| s > 0.5).count() as f32 / n as f32;
    // Where to read the subject's lighting: the subject mask, else detected skin.
    let on_subject: Vec<bool> = match subject {
        Some(m) => m.iter().map(|&v| v > 0.5).collect(),
        None => skin.iter().map(|&s| s > 0.5).collect(),
    };

    // ---- Hardness: brightness spread between the lit and shadow sides. Read on skin
    // when there's enough (its reflectance is fairly even), else on the subject, whose
    // clothing contrast would otherwise pass for hard light.
    let on_skin: Vec<bool> = skin.iter().map(|&s| s > 0.5).collect();
    let region = if skin_cov > 0.003 {
        &on_skin
    } else {
        &on_subject
    };
    let mut subj_ev: Vec<f32> = (0..n).filter(|&i| region[i]).map(|i| ev[i]).collect();
    let (hardness, spread) = if subj_ev.len() > n / 400 {
        let spread = percentile(&mut subj_ev, 0.9) - percentile(&mut subj_ev, 0.1);
        (smoothstep(1.2, 3.5, spread), spread)
    } else {
        let spread = percentile(&mut evs, 0.9) - percentile(&mut evs, 0.1);
        (smoothstep(2.0, 5.0, spread), spread)
    };
    let hardness_label = match hardness {
        h if h < 0.33 => "soft, diffuse",
        h if h < 0.66 => "medium",
        _ => "hard, direct",
    };
    let _ = spread;

    // ---- Direction and backlight from the subject's shading.
    let (direction, backlit) = light_direction(&ev, &on_subject, w, h);

    // ---- Haze: dark-channel prior on non-sky pixels.
    let haze = dark_channel_haze(rgb, sky, w, h);
    let haze_label = match haze {
        v if v < 0.15 => "clear",
        v if v < 0.3 => "slight haze",
        v if v < 0.5 => "hazy",
        _ => "foggy",
    };

    // ---- Snow: bright, colourless ground in the lower frame.
    let (mut ground, mut white) = (0usize, 0usize);
    for yy in (h * 2 / 5)..h {
        for xx in 0..w {
            let i = yy * w + xx;
            if sky.is_some_and(|s| s[i] > 0.5) {
                continue;
            }
            ground += 1;
            let c = lab.g[i].hypot(lab.b[i]);
            if lab.r[i] > 0.8 && c < 0.03 {
                white += 1;
            }
        }
    }
    let snow = if ground > 0 {
        white as f32 / ground as f32
    } else {
        0.0
    };

    // ---- Skin mechanics.
    let skin_report = (skin_cov > 0.005).then(|| skin_mechanics(lab, skin, w, h, skin_cov));

    // ---- Lines (architecture).
    let line_strength = line_strength(lab, w, h);

    // ---- Depth range.
    let depth_range = planes.depth.as_deref().map(|d| {
        let mut v = d.to_vec();
        (percentile(&mut v, 0.95) - percentile(&mut v, 0.05)).clamp(0.0, 1.0)
    });
    if planes.depth.is_none() {
        limits.push("Depth model not installed: no depth range or depth-based haze check.".into());
    }

    // ---- Time of day from the capture clock.
    let hour = meta
        .date_time_original
        .as_deref()
        .and_then(|d| d.get(11..13))
        .and_then(|h| h.parse::<u32>().ok());
    let time_of_day = hour.map(|h| match h {
        5..=7 => "sunrise / early morning",
        8..=15 => "daytime",
        16..=19 => "late afternoon / golden hour",
        _ => "night",
    });

    // ---- Colour cast (mean Oklab a/b of the midtones).
    let (mut ca, mut cb, mut cn) = (0.0f64, 0.0f64, 0usize);
    for i in (0..n).step_by(4) {
        if lab.r[i] > 0.25 && lab.r[i] < 0.9 {
            ca += lab.g[i] as f64;
            cb += lab.b[i] as f64;
            cn += 1;
        }
    }
    let (cast_a, cast_b) = if cn > 0 {
        ((ca / cn as f64) as f32, (cb / cn as f64) as f32)
    } else {
        (0.0, 0.0)
    };

    // ---- Nature: foliage and earth colours (hue 70–160° at some chroma).
    let natural = {
        let (mut k, mut total) = (0usize, 0usize);
        for i in (0..n).step_by(4) {
            total += 1;
            let c = lab.g[i].hypot(lab.b[i]);
            let hue = lab.b[i].atan2(lab.g[i]).to_degrees().rem_euclid(360.0);
            if c > 0.025 && (70.0..=160.0).contains(&hue) {
                k += 1;
            }
        }
        k as f32 / total.max(1) as f32
    };

    // ---- Stars: isolated bright points in dark sky.
    let stars = count_stars(&y, sky, w, h);

    let palette = dominant_colors(lab);

    let lighting = Lighting {
        color_temperature: as_shot_kelvin,
        dynamic_range_ev,
        highlights_clipped: 100.0 * over as f32 / n as f32,
        shadows_crushed: 100.0 * crushed as f32 / n as f32,
        key,
        hardness,
        hardness_label,
        direction,
        backlit,
        haze,
        haze_label,
        snow,
        time_of_day,
    };
    let composition = Composition {
        subject_coverage: subject_cov.map(|c| 100.0 * c),
        sky_coverage: sky_cov.map(|c| 100.0 * c),
        depth_range,
        line_strength,
    };
    let genres = score_genres(&GenreInputs {
        skin: skin_cov,
        subject: subject_cov,
        sky: sky_cov.unwrap_or(0.0),
        depth_range,
        line_strength,
        tone_depth: skin_report.as_ref().map(|s| s.tone_depth),
        kelvin: as_shot_kelvin,
        hour,
        focal_mm: meta.focal_length.map(|(n, d)| n as f32 / d.max(1) as f32),
        exposure_s: meta.exposure_time.map(|(n, d)| n as f32 / d.max(1) as f32),
        iso: meta.iso,
        median_l,
        cast: (cast_a, cast_b),
        white_share: white_share(lab),
        hardness,
        stars,
        backlit,
        natural,
    });
    SceneAnalysis {
        genres,
        lighting,
        skin: skin_report,
        composition,
        palette,
        limits,
        analysis_ms: started.elapsed().as_millis() as u64,
    }
}

fn light_direction(ev: &[f32], on: &[bool], w: usize, h: usize) -> (Option<String>, bool) {
    let idx: Vec<usize> = (0..w * h).filter(|&i| on[i]).collect();
    if idx.len() < (w * h) / 200 {
        return (None, false);
    }
    let (sx, sy) = idx.iter().fold((0.0f32, 0.0f32), |(a, b), &i| {
        (a + (i % w) as f32, b + (i / w) as f32)
    });
    let (cx, cy) = (sx / idx.len() as f32, sy / idx.len() as f32);
    let mean = |f: &dyn Fn(usize) -> bool| {
        let v: Vec<f32> = idx.iter().filter(|&&i| f(i)).map(|&i| ev[i]).collect();
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f32>() / v.len() as f32
        }
    };
    let lr = mean(&|i| ((i % w) as f32) < cx) - mean(&|i| ((i % w) as f32) >= cx);
    let tb = mean(&|i| ((i / w) as f32) < cy) - mean(&|i| ((i / w) as f32) >= cy);

    // Backlight: subject much darker than its surroundings, but its rim brighter than
    // its interior.
    let subject_ev = mean(&|_| true);
    let bg: Vec<f32> = (0..w * h).filter(|&i| !on[i]).map(|i| ev[i]).collect();
    let bg_ev = if bg.is_empty() {
        subject_ev
    } else {
        bg.iter().sum::<f32>() / bg.len() as f32
    };
    let rim = |i: usize| {
        let (x, y) = (i % w, i / w);
        let r = (w.max(h) / 150).max(1);
        [
            (x.saturating_sub(r), y),
            ((x + r).min(w - 1), y),
            (x, y.saturating_sub(r)),
            (x, (y + r).min(h - 1)),
        ]
        .iter()
        .any(|&(xx, yy)| !on[yy * w + xx])
    };
    let rim_ev = mean(&|i| rim(i));
    let interior_ev = mean(&|i| !rim(i));
    let backlit = bg_ev - subject_ev > 1.0 && rim_ev - interior_ev > 0.3;

    let mag = lr.hypot(tb);
    if mag < 0.25 {
        return (Some("frontal / even".into()), backlit);
    }
    let horizontal = if lr.abs() > 0.2 {
        if lr > 0.0 {
            "left"
        } else {
            "right"
        }
    } else {
        ""
    };
    let vertical = if tb.abs() > 0.2 {
        if tb > 0.0 {
            "upper"
        } else {
            "lower"
        }
    } else {
        ""
    };
    let text = match (vertical, horizontal) {
        ("", h) => format!("from the {h}"),
        (v, "") => format!("from {}", if v == "upper" { "above" } else { "below" }),
        (v, h) => format!("from the {v} {h}"),
    };
    (Some(text), backlit)
}

/// He et al. 2009: in haze-free outdoor images most patches have a very dark channel;
/// haze lifts it towards the airlight.
fn dark_channel_haze(rgb: &ImageRgbF32, sky: Option<&[f32]>, w: usize, h: usize) -> f32 {
    let dark: Vec<f32> = (0..w * h)
        .map(|i| rgb.r[i].min(rgb.g[i]).min(rgb.b[i]).max(0.0))
        .collect();
    let r = (w.max(h) / 60).max(2);
    let dark = min_filter(&dark, w, h, r);
    // Airlight: the brightest dark-channel values.
    let mut sorted = dark.clone();
    let air = percentile(&mut sorted, 0.999).max(1e-4);
    let mut t: Vec<f32> = (0..w * h)
        .filter(|&i| !sky.is_some_and(|s| s[i] > 0.5))
        .map(|i| (dark[i] / air).min(1.0))
        .collect();
    percentile(&mut t, 0.5)
}

fn min_filter(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut tmp = vec![0.0; src.len()];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            *o = src[y * w + a..=y * w + b]
                .iter()
                .copied()
                .fold(f32::MAX, f32::min);
        }
    });
    let mut out = vec![0.0; src.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
        for (x, o) in row.iter_mut().enumerate() {
            *o = (a..=b).map(|yy| tmp[yy * w + x]).fold(f32::MAX, f32::min);
        }
    });
    out
}

fn skin_mechanics(
    lab: &ImageRgbF32,
    skin: &[f32],
    w: usize,
    h: usize,
    coverage: f32,
) -> SkinReport {
    let idx: Vec<usize> = (0..w * h).filter(|&i| skin[i] > 0.5).collect();
    let mut ls: Vec<f32> = idx.iter().map(|&i| lab.r[i]).collect();
    let l = percentile(&mut ls, 0.5);
    // Place L between the Monk swatches (their L falls from 1 to 10).
    let tone_depth = {
        let mut depth = 10.0;
        for k in 0..9 {
            let (hi, lo) = (MONK_L[k].max(MONK_L[k + 1]), MONK_L[k].min(MONK_L[k + 1]));
            if l >= lo && l <= hi {
                let t = (MONK_L[k] - l) / (MONK_L[k] - MONK_L[k + 1]);
                depth = k as f32 + 1.0 + t.clamp(0.0, 1.0);
                break;
            }
        }
        if l > MONK_L[0] {
            1.0
        } else {
            depth
        }
    };
    let tone_label = match tone_depth {
        d if d < 3.5 => "light",
        d if d < 5.5 => "light-medium",
        d if d < 7.0 => "medium-deep",
        _ => "deep",
    };
    let (sa, sb) = idx
        .iter()
        .fold((0.0f32, 0.0f32), |(a, b), &i| (a + lab.g[i], b + lab.b[i]));
    let hue = (sb / idx.len() as f32)
        .atan2(sa / idx.len() as f32)
        .to_degrees();
    let undertone = match hue {
        h if h < 38.0 => "cool (pink-red)",
        h if h < 55.0 => "neutral",
        h if h < 70.0 => "warm (golden)",
        _ => "olive",
    };
    // Shine: skin pixels well above their neighbourhood's lightness.
    let r = (w.max(h) / 100).max(2);
    let base = box_mean(&lab.r, w, h, r);
    let shiny =
        idx.iter().filter(|&&i| lab.r[i] - base[i] > 0.06).count() as f32 / idx.len() as f32;
    let shine = smoothstep(0.005, 0.06, shiny);
    // Measured at the analysis size: on the test portraits it reads the same at twice
    // the resolution, so a sharper develop would add time without information.
    let tex = relative_detail(lab, skin, w, h);
    let texture = smoothstep(0.008, 0.035, tex);
    // Shine and texture need a face big enough to measure: on small skin areas the
    // numbers are mostly edges.
    let measurable = coverage >= 0.02;
    let too_small = "too small to judge";
    SkinReport {
        coverage: 100.0 * coverage,
        tone_depth,
        tone_label,
        undertone,
        shine,
        shine_label: if !measurable {
            too_small
        } else if shine < 0.33 {
            "matte"
        } else if shine < 0.66 {
            "some shine"
        } else {
            "glossy"
        },
        texture,
        texture_label: if !measurable {
            too_small
        } else if texture < 0.33 {
            "smooth"
        } else if texture < 0.66 {
            "moderate"
        } else {
            "pronounced (pores, beard, lines)"
        },
    }
}

/// Skin texture: fine-scale lightness variation on skin, relative to the local
/// lightness so deep skin (where the same pores move L less) reads like light skin.
fn relative_detail(lab: &ImageRgbF32, skin: &[f32], w: usize, h: usize) -> f32 {
    let fine = box_mean(&lab.r, w, h, 1);
    let (mut sum, mut k) = (0.0f32, 0usize);
    for i in (0..w * h).filter(|&i| skin[i] > 0.5) {
        sum += (lab.r[i] - fine[i]).abs() / fine[i].max(0.05);
        k += 1;
    }
    if k == 0 {
        0.0
    } else {
        sum / k as f32
    }
}

pub(crate) fn box_mean(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let pix: Vec<f32> = src.to_vec();
    // Small radii here: a direct separable mean is fast enough at analysis size.
    let mut tmp = vec![0.0; pix.len()];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            *o = pix[y * w + a..=y * w + b].iter().sum::<f32>() / (b - a + 1) as f32;
        }
    });
    let mut out = vec![0.0; pix.len()];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
        for (x, o) in row.iter_mut().enumerate() {
            *o = (a..=b).map(|yy| tmp[yy * w + x]).sum::<f32>() / (b - a + 1) as f32;
        }
    });
    out
}

/// Share of gradient energy along horizontal and vertical directions, relative to a
/// scene with no preferred direction.
fn line_strength(lab: &ImageRgbF32, w: usize, h: usize) -> f32 {
    let l = &lab.r;
    let (mut aligned, mut total) = (0.0f64, 0.0f64);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let gx = (l[i + 1] - l[i - 1]) as f64;
            let gy = (l[i + w] - l[i - w]) as f64;
            let m = gx.hypot(gy);
            if m < 0.02 {
                continue;
            }
            let angle = gy.atan2(gx).to_degrees().rem_euclid(90.0);
            if !(6.0..=84.0).contains(&angle) {
                aligned += m;
            }
            total += m;
        }
    }
    if total == 0.0 {
        return 0.0;
    }
    // Uniform directions put 12/90 ≈ 13 % in these bands.
    smoothstep(0.2, 0.5, (aligned / total) as f32)
}

fn count_stars(y: &[f32], sky: Option<&[f32]>, w: usize, h: usize) -> usize {
    let mut stars = 0;
    for yy in 2..h.saturating_sub(2) {
        for xx in 2..w.saturating_sub(2) {
            let i = yy * w + xx;
            if sky.is_some_and(|s| s[i] < 0.5) {
                continue;
            }
            let v = y[i];
            let around = [i - 2, i + 2, i - 2 * w, i + 2 * w];
            let bg = around.iter().map(|&j| y[j]).fold(0.0f32, f32::max);
            if v > 0.05
                && bg < 0.02
                && v > 4.0 * bg.max(1e-4)
                && v >= y[i - 1]
                && v >= y[i + 1]
                && v >= y[i - w]
                && v >= y[i + w]
            {
                stars += 1;
            }
        }
    }
    stars
}

fn white_share(lab: &ImageRgbF32) -> f32 {
    let n = lab.r.len();
    (0..n)
        .filter(|&i| lab.r[i] > 0.85 && lab.g[i].hypot(lab.b[i]) < 0.025)
        .count() as f32
        / n as f32
}

/// Five dominant colours by k-means in Oklab over a pixel sample.
pub(crate) fn dominant_colors(lab: &ImageRgbF32) -> Vec<Swatch> {
    let n = lab.r.len();
    let step = (n / 4000).max(1);
    let samples: Vec<[f32; 3]> = (0..n)
        .step_by(step)
        .map(|i| [lab.r[i], lab.g[i], lab.b[i]])
        .collect();
    if samples.is_empty() {
        return Vec::new();
    }
    let k = 5.min(samples.len());
    // Deterministic seeds spread through the sample sorted by lightness.
    let mut by_l = samples.clone();
    by_l.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut centers: Vec<[f32; 3]> = (0..k)
        .map(|j| by_l[(j * 2 + 1) * by_l.len() / (2 * k)])
        .collect();
    let mut assign = vec![0usize; samples.len()];
    for _ in 0..12 {
        for (s, a) in samples.iter().zip(assign.iter_mut()) {
            *a = (0..k)
                .min_by(|&x, &y| dist(s, &centers[x]).total_cmp(&dist(s, &centers[y])))
                .unwrap_or(0);
        }
        for (j, c) in centers.iter_mut().enumerate() {
            let members: Vec<&[f32; 3]> = samples
                .iter()
                .zip(&assign)
                .filter(|(_, &a)| a == j)
                .map(|(s, _)| s)
                .collect();
            if !members.is_empty() {
                let m = members.len() as f32;
                *c = [0, 1, 2].map(|d| members.iter().map(|s| s[d]).sum::<f32>() / m);
            }
        }
    }
    let mut out: Vec<Swatch> = centers
        .iter()
        .enumerate()
        .map(|(j, c)| Swatch {
            hex: oklab_to_hex(*c),
            weight: assign.iter().filter(|&&a| a == j).count() as f32 / samples.len() as f32,
        })
        .filter(|s| s.weight > 0.0)
        .collect();
    out.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    out
}

fn dist(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// Oklab → display sRGB hex (clipped), for showing palette swatches.
pub(crate) fn oklab_to_hex([l, a, b]: [f32; 3]) -> String {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    let lin = [
        4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3,
        -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3,
    ];
    let enc = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        let e = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (e * 255.0).round() as u8
    };
    format!("#{:02x}{:02x}{:02x}", enc(lin[0]), enc(lin[1]), enc(lin[2]))
}

struct GenreInputs {
    skin: f32,
    subject: Option<f32>,
    sky: f32,
    depth_range: Option<f32>,
    line_strength: f32,
    tone_depth: Option<f32>,
    kelvin: Option<f32>,
    hour: Option<u32>,
    focal_mm: Option<f32>,
    exposure_s: Option<f32>,
    iso: Option<u32>,
    median_l: f32,
    cast: (f32, f32),
    white_share: f32,
    hardness: f32,
    stars: usize,
    backlit: bool,
    /// Share of foliage / earth colours.
    natural: f32,
}

/// Rules over the measurements; each genre keeps its evidence for the UI.
fn score_genres(g: &GenreInputs) -> Vec<GenreScore> {
    let s = smoothstep;
    let pct = |v: f32| format!("{:.1}%", 100.0 * v);
    let subject = g.subject.unwrap_or((g.skin * 5.0).min(1.0));
    let focal = g.focal_mm.unwrap_or(50.0);
    // People can be small in the frame: 0.2 % skin is already a person.
    let people = s(0.002, 0.008, g.skin);
    let warm_light = g.kelvin.map_or(0.5, |k| 1.0 - s(3800.0, 5600.0, k));
    let golden_clock = g.hour.map_or(0.5, |h| {
        if (16..=19).contains(&h) || (5..=7).contains(&h) {
            1.0
        } else {
            0.2
        }
    });
    let blue_cast = s(0.01, 0.04, -g.cast.1) * s(0.0, 0.02, -g.cast.0 + 0.01);
    let long_exposure = g.exposure_s.map_or(0.0, |t| s(1.0, 8.0, t));
    let dark_high_iso =
        s(0.35, 0.2, g.median_l) * g.iso.map_or(0.0, |i| s(1600.0, 6400.0, i as f32));

    let close_up = s(0.03, 0.08, g.skin) * s(0.12, 0.3, subject);
    let environmental = people
        * (1.0 - s(0.06, 0.15, g.skin))
        * s(0.01, 0.06, subject)
        * g.depth_range.map_or(0.8, |d| s(0.3, 0.6, d));
    let portrait = close_up.max(environmental);
    let mut out = vec![
        GenreScore {
            id: "close-up-portrait",
            label: "Close-up Portrait",
            score: close_up,
            evidence: format!("skin {}, subject {}", pct(g.skin), pct(subject)),
        },
        GenreScore {
            id: "environmental-portrait",
            label: "Environmental Portrait",
            score: environmental,
            evidence: format!(
                "skin {} in a wider scene, depth range {}",
                pct(g.skin),
                g.depth_range.map_or("n/a".into(), |d| format!("{d:.2}"))
            ),
        },
        GenreScore {
            id: "dark-melanin-fashion",
            label: "Dark Melanin Fashion",
            score: portrait * s(0.01, 0.03, g.skin) * g.tone_depth.map_or(0.0, |d| s(6.5, 7.5, d)),
            evidence: format!(
                "portrait with skin tone depth {}",
                g.tone_depth.map_or("n/a".into(), |d| format!("~{d:.0}/10"))
            ),
        },
        GenreScore {
            id: "golden-hour-landscape",
            label: "Golden Hour Landscape",
            score: s(0.08, 0.25, g.sky) * warm_light * golden_clock * (1.0 - s(0.01, 0.05, g.skin)),
            evidence: format!(
                "sky {}, light {}, {}",
                pct(g.sky),
                g.kelvin.map_or("n/a".into(), |k| format!("{k:.0} K")),
                g.hour
                    .map_or("no capture time".into(), |h| format!("shot at {h}:00"))
            ),
        },
        GenreScore {
            id: "wildlife",
            label: "Wildlife",
            score: (1.0 - s(0.001, 0.006, g.skin))
                * s(60.0, 150.0, focal)
                * s(0.003, 0.02, subject).max(s(0.2, 0.5, g.natural)),
            evidence: format!(
                "no people, {focal:.0} mm, isolated subject {}, {} foliage / earth colours",
                pct(subject),
                pct(g.natural)
            ),
        },
        GenreScore {
            id: "architectural",
            label: "Architectural",
            score: g.line_strength * (1.0 - s(0.005, 0.03, g.skin)),
            evidence: format!("straight-line strength {:.2}", g.line_strength),
        },
        GenreScore {
            id: "street",
            label: "Street",
            score: s(0.0005, 0.003, g.skin)
                * (1.0 - s(0.02, 0.06, g.skin))
                * s(0.1, 0.35, g.line_strength)
                * (1.0 - s(70.0, 135.0, focal)),
            evidence: format!(
                "small people ({} skin) in a lined, urban-looking frame",
                pct(g.skin)
            ),
        },
        GenreScore {
            id: "fine-art-wedding",
            label: "Fine Art Wedding",
            score: people * s(0.08, 0.2, g.white_share) * (1.0 - 0.5 * g.hardness),
            evidence: format!("people with {} bright white areas", pct(g.white_share)),
        },
        GenreScore {
            id: "underwater",
            label: "Underwater",
            score: blue_cast * (1.0 - s(0.02, 0.1, g.sky)),
            evidence: format!(
                "blue-green cast (a {:+.3}, b {:+.3}), no sky",
                g.cast.0, g.cast.1
            ),
        },
        GenreScore {
            id: "astrophotography",
            label: "Astrophotography",
            score: long_exposure.max(dark_high_iso) * s(5.0, 40.0, g.stars as f32),
            evidence: format!(
                "{} star-like points, {}",
                g.stars,
                g.exposure_s
                    .map_or("no exposure data".into(), |t| format!("{t:.1} s exposure"))
            ),
        },
    ];
    if g.backlit {
        for x in out.iter_mut().filter(|x| x.id == "golden-hour-landscape") {
            x.score = (x.score * 1.2).min(1.0);
        }
    }
    out.retain(|x| x.score >= 0.2);
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(3);
    out
}

/// Resize a model output to the analysis image.
pub(crate) fn fit(plane: &[f32], pw: u32, ph: u32, w: u32, h: u32) -> Vec<f32> {
    resize_plane(plane, pw, ph, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monk_scale_placement_and_palette_hex() {
        // White and black round-trip through the hex conversion.
        assert_eq!(oklab_to_hex([1.0, 0.0, 0.0]), "#ffffff");
        assert_eq!(oklab_to_hex([0.0, 0.0, 0.0]), "#000000");
        let base = GenreInputs {
            skin: 0.09,
            subject: Some(0.35),
            sky: 0.0,
            depth_range: Some(0.7),
            line_strength: 0.1,
            tone_depth: Some(8.2),
            kelvin: Some(5200.0),
            hour: Some(12),
            focal_mm: Some(39.0),
            exposure_s: Some(0.008),
            iso: Some(500),
            median_l: 0.5,
            cast: (0.0, 0.0),
            white_share: 0.02,
            hardness: 0.4,
            stars: 0,
            backlit: false,
            natural: 0.1,
        };
        let g = score_genres(&base);
        let ids: Vec<&str> = g.iter().map(|x| x.id).collect();
        assert!(
            ids.contains(&"close-up-portrait") && ids.contains(&"dark-melanin-fashion"),
            "{ids:?}"
        );
        // A long-lens subject without skin reads as wildlife.
        let bird = GenreInputs {
            skin: 0.0,
            subject: Some(0.012),
            focal_mm: Some(140.0),
            tone_depth: None,
            natural: 0.55,
            ..base
        };
        assert_eq!(score_genres(&bird).first().map(|x| x.id), Some("wildlife"));
    }
}
