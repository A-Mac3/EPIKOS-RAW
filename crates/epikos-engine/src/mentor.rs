//! The AI Photography Mentor: a rule-based reading of one photo that explains how and
//! why to edit it, a recommended starting point, and live feedback on the current edit.
//!
//! Everything here is measured, not generated: the scene analysis (genre, light, skin,
//! composition), the scene's tonal statistics and colour cast, the auto-tone and
//! auto-upright suggestions, the skin's colour in the current edit, subject /
//! background separation and composition (see [`crate::guidance`]), and which lens
//! profile and masks are available. Each insight says what was seen, why it matters and
//! what to do. The reading is cheap to repeat (the scene analysis and masks are
//! cached), so the app re-reads it as the edit changes.

use std::path::Path;
use std::time::Instant;

use epikos_core::{ImageRgbF32, Result};
use epikos_pipeline::{develop_rgb_with, oklab_planes, skin_likelihood, to_display_srgb};
use epikos_sidecar::{Adjustments, Crop, LocalAdjustment, MaskTarget, WbMode};
use serde::Serialize;

use crate::analysis::{self, SceneAnalysis};
use crate::composition::{person_crop, rank_subjects, RankedSubject};
use crate::guidance::{self, CropAdvice, SkinFix, SkinProblem};
use crate::learn::{editorial, pro_profiles, Target};
use crate::{scene_only, Engine};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MentorReport {
    /// One line: what kind of photo this is and its main challenge.
    pub summary: String,
    pub insights: Vec<Insight>,
    /// The recommended starting point (the current settings with the suggestions in).
    pub recommended: Adjustments,
    /// What the starting point changes, in words.
    pub changes: Vec<String>,
    /// A composition crop and straighten, offered separately from the starting point.
    pub crop: Option<CropAdvice>,
    /// The look the starting point aims for: "Editorial" or a learned style's name.
    pub target: String,
    /// Subjects in the frame, primary first.
    pub subjects: Vec<RankedSubject>,
    pub analysis_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Insight {
    /// "Exposure", "Dynamic range", "Colour", "Light", "Subject", "Skin", "Framing", "Style".
    pub topic: &'static str,
    /// What was measured.
    pub observation: String,
    /// Why it matters for this photo.
    pub why: String,
    /// What to do, naming the control.
    pub how: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Feedback {
    /// "praise", "warning" or "tip".
    pub level: &'static str,
    pub text: String,
    /// The correction, for warnings and tips: slider targets in words and the
    /// settings with them applied (one click in the app).
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fix {
    /// e.g. "Highlights −35, Exposure −0.20 EV".
    pub label: String,
    pub adjustments: Adjustments,
}

/// Builds a [`Fix`]: each step changes a copy of the settings and names the new value.
struct FixBuilder {
    adj: Adjustments,
    parts: Vec<String>,
}

impl FixBuilder {
    fn new(adj: &Adjustments) -> Self {
        Self { adj: adj.clone(), parts: Vec::new() }
    }

    fn set(mut self, label: impl Into<String>, f: impl FnOnce(&mut Adjustments)) -> Self {
        f(&mut self.adj);
        self.parts.push(label.into());
        self
    }

    fn skin(self, label: impl Into<String>, f: impl FnOnce(&mut LocalAdjustment)) -> Self {
        self.set(label, |a| {
            let l = match a.local.iter_mut().position(|l| l.mask == MaskTarget::Skin) {
                Some(i) => &mut a.local[i],
                None => {
                    a.local.push(LocalAdjustment { mask: MaskTarget::Skin, ..Default::default() });
                    a.local.last_mut().expect("just pushed")
                }
            };
            f(l);
        })
    }

    fn done(self) -> Option<Fix> {
        (!self.parts.is_empty()).then(|| Fix { label: self.parts.join(", "), adjustments: self.adj })
    }
}

fn signed(v: f32) -> String {
    format!("{}{:.0}", if v >= 0.0 { "+" } else { "−" }, v.abs())
}

/// Tonal statistics of a developed image.
struct Stats {
    /// Luminance percentiles in stops relative to mid-grey (0.18).
    p02: f32,
    p50: f32,
    p98: f32,
    /// Share of pixels with a channel at display white / black (0–1).
    clipped: f32,
    crushed: f32,
    /// Mean Oklab a/b of the mid-tone, low-chroma pixels (the neutral-ish ones).
    cast: (f32, f32),
    /// Share of strongly saturated pixels (0–1).
    loud: f32,
    /// Median Oklab lightness and hue (degrees) of skin, if there is enough.
    skin: Option<(f32, f32)>,
}

fn stats(rgb: &ImageRgbF32) -> Stats {
    let n = rgb.len().max(1);
    let display = to_display_srgb(rgb);
    let px = display.rgba.as_chunks::<4>().0;
    let clipped = px.iter().filter(|p| p[..3].iter().any(|&v| v >= 254)).count() as f32 / n as f32;
    let crushed = px.iter().filter(|p| p[..3].iter().all(|&v| v <= 2)).count() as f32 / n as f32;
    let lab = oklab_planes(rgb);
    let mut ev: Vec<f32> = (0..n)
        .map(|i| ((0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(1e-6) / 0.18).log2())
        .collect();
    ev.sort_unstable_by(f32::total_cmp);
    let pct = |p: f32| ev[((n - 1) as f32 * p) as usize];
    let (mut sa, mut sb, mut k) = (0.0f32, 0.0f32, 0usize);
    let mut loud = 0usize;
    for i in 0..n {
        let c = lab.g[i].hypot(lab.b[i]);
        if (0.35..0.85).contains(&lab.r[i]) && c < 0.05 {
            sa += lab.g[i];
            sb += lab.b[i];
            k += 1;
        }
        if c > 0.22 {
            loud += 1;
        }
    }
    let skin_map = skin_likelihood(rgb);
    let on: Vec<usize> = (0..n).filter(|&i| skin_map[i] > 0.5).collect();
    let skin = (on.len() > n / 100).then(|| {
        let mut l: Vec<f32> = on.iter().map(|&i| lab.r[i]).collect();
        let mut h: Vec<f32> = on.iter().map(|&i| lab.b[i].atan2(lab.g[i]).to_degrees()).collect();
        let mid = l.len() / 2;
        (*l.select_nth_unstable_by(mid, f32::total_cmp).1, *h.select_nth_unstable_by(mid, f32::total_cmp).1)
    });
    Stats {
        p02: pct(0.02),
        p50: pct(0.5),
        p98: pct(0.98),
        clipped,
        crushed,
        cast: if k > n / 50 { (sa / k as f32, sb / k as f32) } else { (0.0, 0.0) },
        loud: loud as f32 / n as f32,
        skin,
    }
}

fn cast_words((a, b): (f32, f32)) -> Option<&'static str> {
    if a.hypot(b) < 0.008 {
        return None;
    }
    let hue = b.atan2(a).to_degrees().rem_euclid(360.0);
    Some(match hue {
        h if h < 45.0 => "magenta-red",
        h if h < 100.0 => "warm yellow-orange",
        h if h < 170.0 => "green",
        h if h < 230.0 => "cyan",
        h if h < 300.0 => "blue",
        _ => "magenta",
    })
}

impl Engine {
    /// Read the photo and explain how and why to edit it, with a recommended start.
    ///
    /// The starting point aims at `target`: a learned style's id, or `None` for the
    /// built-in Editorial profile (rich blacks, dimensional skin, calm olive foliage,
    /// a midtone S-curve).
    pub fn mentor(&self, path: &Path, adjustments: &Adjustments, target: Option<&str>) -> Result<MentorReport> {
        use std::sync::atomic::Ordering;
        let started = Instant::now();
        let ticket = self.mentor_ticket.fetch_add(1, Ordering::SeqCst) + 1;
        let current = || if self.mentor_ticket.load(Ordering::SeqCst) == ticket { Ok(()) } else { Err(epikos_core::Error::Superseded) };
        let loaded = self.load(path)?;
        let scene = self.analyze(path, adjustments)?;
        current()?;
        let neutral = Adjustments { exposure: 0.0, tone: Default::default(), ..scene_only(adjustments) };
        let mut s = stats(&loaded.develop_scene(512, &neutral)?);
        // Colour is judged on what's on screen (the current look), tone on the scene. The
        // whole frame, uncropped: the masks and the composition advice are made for it.
        let whole = Adjustments { crop: Crop::default(), ..adjustments.clone() };
        let base = loaded.base(512, 512);
        let prepared = self.prepare(&loaded, &whole)?;
        let shown = prepared
            .with_inputs(|inputs| develop_rgb_with((*base).clone(), &loaded.raw.profile, &whole, inputs))?;
        s.cast = stats(&shown).cast;
        current()?;
        let (auto_ev, auto_tone) = self.auto_tone(path, adjustments)?;
        current()?;
        let (rotation, vertical) = self.auto_upright(path, adjustments)?;
        current()?;
        let targets = self.mask_targets();
        let available = |t: MaskTarget| targets.iter().any(|x| x.target == t && x.available);
        let has_profile = loaded.lens_profile().is_some();

        // Regions of the current edit, at its size.
        let (w, h) = (shown.width, shown.height);
        let mask = |target: MaskTarget| {
            self.mask_plane(&loaded, adjustments, target)
                .ok()
                .flatten()
                .map(|m| analysis::fit(&m.data, m.width, m.height, w, h))
        };
        let (skin, subject, sky) = (mask(MaskTarget::Skin), mask(MaskTarget::Subject), mask(MaskTarget::Sky));
        let current_skin = adjustments.local.iter().find(|l| l.mask == MaskTarget::Skin);
        let skin_fix = skin.as_deref().and_then(|m| guidance::skin_balance(&shown, m, current_skin));
        let separation = subject.as_deref().and_then(|m| guidance::inside_outside_ev(&shown, m));
        let background_warmth = subject.as_deref().map(|m| guidance::outside_warmth(&shown, m));
        let straighten = if rotation.abs() >= 0.3 { rotation } else { adjustments.lens.rotation };
        // Subjects ranked by area and nearness; a person as the primary subject is
        // framed by portrait rules (no cut joints, feet clearance, headroom, lead room).
        let depth = self
            .masker
            .depth_available()
            .then(|| self.depth_for(&loaded, &whole).ok())
            .flatten()
            .map(|d| analysis::fit(&d.depth, d.width, d.height, w, h));
        let subjects = subject
            .as_deref()
            .map(|m| rank_subjects(m, skin.as_deref(), depth.as_deref(), w as usize, h as usize))
            .unwrap_or_default();
        let crop = match subjects.first() {
            Some(p) if p.person => person_crop(p, skin.as_deref(), w as usize, h as usize, straighten),
            _ => guidance::suggest_crop(subject.as_deref(), sky.as_deref(), w as usize, h as usize, straighten),
        };

        let mut report = advise(&scene, &s, adjustments, AdviceInputs {
            subjects: &subjects,
            auto_ev,
            auto_tone,
            rotation,
            vertical,
            has_profile,
            bitmap: loaded.raw.profile.format.is_bitmap(),
            eyes: available(MaskTarget::Eyes),
            subject: available(MaskTarget::Subject),
            skin_fix,
            separation,
            background_warmth,
            crop,
            started,
        });

        // Aim the starting point at the target look, measured on the real render.
        // A learned style when chosen; otherwise a pro profile (the one chosen, or the
        // one for the detected genre); otherwise the Editorial profile.
        let learned = match target {
            Some(id) if id != "editorial" && !id.starts_with("pro-") => {
                self.learned_styles()?.into_iter().find(|s| s.id == id)
            }
            _ => None,
        };
        let profiles = pro_profiles();
        let genre = scene.genres.first().map(|g| g.id);
        let pro = match target {
            Some(id) if id.starts_with("pro-") => profiles.iter().find(|p| p.id == id),
            None => genre.and_then(|g| profiles.iter().find(|p| p.genres.contains(&g))),
            _ => None,
        };
        let built_in = editorial();
        let target = match (&learned, pro) {
            (Some(l), _) => Target { name: l.name.clone(), signature: &l.signature, match_exposure: true, match_colours: true },
            (None, Some(p)) => Target { name: p.name.into(), signature: &p.signature, match_exposure: false, match_colours: false },
            (None, None) => {
                Target { name: "Editorial".into(), signature: &built_in, match_exposure: false, match_colours: false }
            }
        };
        let pro = if learned.is_none() { pro } else { None };
        current()?;
        let before = report.recommended.clone();
        let fitted = self.fit_to_target(&loaded, &before, &target)?;
        current()?;
        let moves = describe_fit(&before, &fitted);
        if !moves.is_empty() {
            insights_for_target(&mut report, &target, learned.is_some(), pro.map(|p| (p.summary, p.why)), &moves);
            report.changes.extend(moves);
        }
        report.recommended = fitted;
        report.target = target.name;
        report.analysis_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }

    /// Live feedback on `adjustments`: what the edit does well and what to watch.
    pub fn critique(&self, path: &Path, adjustments: &Adjustments) -> Result<Vec<Feedback>> {
        use std::sync::atomic::Ordering;
        let ticket = self.critique_ticket.fetch_add(1, Ordering::SeqCst) + 1;
        let loaded = self.load(path)?;
        let base = loaded.base(512, 512);
        // Compared with the original over the same (whole) frame.
        let whole = Adjustments { crop: Crop::default(), ..adjustments.clone() };
        let prepared = self.prepare(&loaded, &whole)?;
        let edited = prepared
            .with_inputs(|inputs| develop_rgb_with((*base).clone(), &loaded.raw.profile, &whole, inputs))?;
        if self.critique_ticket.load(Ordering::SeqCst) != ticket {
            return Err(epikos_core::Error::Superseded);
        }
        let now = stats(&edited);
        let before = stats(&loaded.develop_scene(512, &Adjustments::default())?);
        Ok(judge(&before, &now, adjustments))
    }
}

struct AdviceInputs<'a> {
    auto_ev: f32,
    auto_tone: epikos_sidecar::Tone,
    rotation: f32,
    vertical: f32,
    has_profile: bool,
    bitmap: bool,
    eyes: bool,
    subject: bool,
    /// Skin outside the natural range for its depth, and the correction.
    skin_fix: Option<SkinFix>,
    /// Mean stops on the subject and off it, in the current edit.
    separation: Option<(f32, f32)>,
    /// Oklab b of the background (warm +, cool −).
    background_warmth: Option<f32>,
    crop: Option<CropAdvice>,
    subjects: &'a [RankedSubject],
    started: Instant,
}

/// Set (or add) the local adjustment on `mask` in `rec` with `f`.
fn upsert_local(rec: &mut Adjustments, mask: MaskTarget, f: impl FnOnce(&mut LocalAdjustment)) {
    match rec.local.iter_mut().find(|l| l.mask == mask) {
        Some(l) => f(l),
        None => {
            let mut l = LocalAdjustment { mask, ..Default::default() };
            f(&mut l);
            rec.local.push(l);
        }
    }
}

fn advise(scene: &SceneAnalysis, s: &Stats, current: &Adjustments, a: AdviceInputs<'_>) -> MentorReport {
    let mut insights = Vec::new();
    let mut rec = current.clone();
    let mut changes = Vec::new();
    let genre = scene.genres.first();
    let portrait = scene.genres.iter().any(|g| g.id.contains("portrait") || g.id == "dark-melanin-fashion");
    let l = &scene.lighting;

    // Framing first: it changes what every later step sees.
    if !a.bitmap && a.has_profile && !current.lens.profile {
        insights.push(Insight {
            topic: "Framing",
            observation: "A lens profile exists for this lens but is switched off.".into(),
            why: "Uncorrected distortion bends straight lines and vignetting darkens the corners before you've made a single creative choice.".into(),
            how: "Step 1: turn Lens profile on.".into(),
        });
        rec.lens.profile = true;
        changes.push("Lens profile on".into());
    }
    if a.rotation.abs() >= 0.5 || a.vertical.abs() >= 10.0 {
        insights.push(Insight {
            topic: "Framing",
            observation: format!(
                "The lines in the frame lean{} ({:+.1}° tilt{}).",
                if a.vertical.abs() >= 10.0 { " and converge" } else { "" },
                a.rotation,
                if a.vertical.abs() >= 10.0 { format!(", {:+.0} vertical perspective", a.vertical) } else { String::new() }
            ),
            why: "A tilted horizon or falling buildings read as a mistake and pull the eye to the edges.".into(),
            how: "Step 1: Auto upright, then fine-tune Straighten if the subject itself leans on purpose.".into(),
        });
        (rec.lens.rotation, rec.lens.vertical) = (a.rotation, a.vertical);
        changes.push(format!("Straighten {:+.2}°, vertical {:+.0}", a.rotation, a.vertical));
    }

    // Exposure and range.
    if (a.auto_ev - current.exposure).abs() >= 0.3 {
        let dir = if a.auto_ev > current.exposure { "below" } else { "above" };
        insights.push(Insight {
            topic: "Exposure",
            observation: format!("The mid-tones sit about {:.1} stops {dir} mid-grey.", (a.auto_ev - current.exposure).abs()),
            why: if portrait {
                "Exposure sets where every other decision starts. This reading uses the whole frame, not the skin, so deep skin keeps its natural depth instead of being pushed lighter.".into()
            } else {
                "Exposure sets where every other decision starts; getting the mid-tones right first means colour and contrast work on the tones you'll actually see.".into()
            },
            how: format!("Step 2: Exposure to {:+.2} EV (or Auto exposure & tone).", a.auto_ev),
        });
        rec.exposure = a.auto_ev;
        changes.push(format!("Exposure {:+.2} EV", a.auto_ev));
    }
    if l.highlights_clipped >= 0.5 || s.p98 + a.auto_ev > 2.2 {
        insights.push(Insight {
            topic: "Dynamic range",
            observation: format!("{:.1}% of the frame is at or near clipping.", l.highlights_clipped.max(100.0 * s.clipped)),
            why: "Clipped highlights have no detail or colour left; skies turn flat white and bright skin loses its shape.".into(),
            how: "Step 2: pull Highlights down (keep Highlight recovery on) before adding any contrast.".into(),
        });
        rec.highlight_recovery = true;
        if a.auto_tone.highlights < rec.tone.highlights {
            rec.tone.highlights = a.auto_tone.highlights;
            changes.push(format!("Highlights {:+.0}", a.auto_tone.highlights));
        }
    }
    if l.dynamic_range_ev >= 9.0 {
        insights.push(Insight {
            topic: "Dynamic range",
            observation: format!("The scene spans about {:.1} stops from deep shadow to bright highlight.", l.dynamic_range_ev),
            why: "A wide range can't all fit on screen at once; a global contrast boost would crush one end or the other.".into(),
            how: "Step 2: recover Highlights first; keep the shadows anchored for depth and lift only the subject (Step 3) if its detail matters. Lifting every shadow flattens the picture into an HDR look.".into(),
        });
    }
    // Shadows stay anchored (no flat, washed-out lift): only a large share of the frame
    // crushed to black gets a gentle opening. The black point is set by the target.
    if s.crushed > 0.08 && a.auto_tone.shadows > rec.tone.shadows + 5.0 {
        let lift = a.auto_tone.shadows.min(15.0);
        rec.tone.shadows = lift;
        changes.push(format!("Shadows {lift:+.0} (only the crushed shadows)"));
    }

    // Colour cast.
    if let Some(words) = cast_words(s.cast) {
        let golden = l.time_of_day.is_some_and(|t| t.contains("golden")) || scene.genres.iter().any(|g| g.id == "golden-hour-landscape");
        insights.push(Insight {
            topic: "Colour",
            observation: format!(
                "Neutral greys in the current edit lean {words}{}.",
                l.color_temperature.map_or(String::new(), |k| format!(" (as shot {:.0} K)", k))
            ),
            why: if golden && words.starts_with("warm") {
                "Here the warmth is the light itself: golden-hour colour is usually worth keeping, just not on the whites.".into()
            } else {
                "A cast tints everything, including skin and whites; neutralising it first makes any later grade look intentional rather than accidental.".into()
            },
            how: if current.white_balance.mode == WbMode::AsShot {
                "Step 2: White balance → Custom, then move Temperature/Tint away from the cast until whites and greys look neutral.".into()
            } else {
                "Step 2: nudge Temperature/Tint against the cast; check a white or grey area.".into()
            },
        });
    }

    // Light.
    if l.backlit {
        insights.push(Insight {
            topic: "Light",
            observation: format!("The light comes from behind the subject{}.", l.direction.as_deref().map_or(String::new(), |d| format!(" ({d})"))),
            why: "Backlight gives a lovely rim but leaves the side facing the camera in shadow.".into(),
            how: if a.subject {
                "Step 3: add a local adjustment on Subject with about +0.3 EV, instead of brightening the whole frame.".into()
            } else {
                "Step 2: lift Shadows gently; install the subject model for a subject-only lift.".into()
            },
        });
        if a.subject && !rec.local.iter().any(|x| x.mask == MaskTarget::Subject) {
            rec.local.push(LocalAdjustment { mask: MaskTarget::Subject, exposure: 0.3, ..Default::default() });
            changes.push("Subject +0.30 EV".into());
        }
    }
    if l.haze >= 0.25 {
        insights.push(Insight {
            topic: "Light",
            observation: format!("The air is {} ({:.0}% haze).", l.haze_label, 100.0 * l.haze),
            why: "Haze lowers contrast and colour in the distance; sometimes that's the mood, sometimes it's just flatness.".into(),
            how: "Step 4: a little Clarity restores separation; Step 6 Depth fog does the opposite if you want to lean into it.".into(),
        });
    }

    // Subject and skin.
    if let Some(skin) = scene.skin.as_ref().filter(|s| s.coverage >= 2.0) {
        insights.push(Insight {
            topic: "Skin",
            observation: format!(
                "Skin covers {:.1}% of the frame: {} tone (Monk ~{:.0}/10), {} undertone, shine: {}.",
                skin.coverage, skin.tone_label, skin.tone_depth, skin.undertone, skin.shine_label
            ),
            why: if skin.tone_depth >= 7.0 {
                "Deep skin loses its richness quickly: a warm grade or heavy contrast can turn it grey or muddy, and brightening it towards a light-skin exposure flattens its depth.".into()
            } else {
                "Skin is where viewers notice colour mistakes first; grades should shape the scene around it, not recolour it.".into()
            },
            how: "Keep Skin tone protection at 40–70% when grading (Step 5), and judge exposure on the whole frame, not the face.".into(),
        });
        rec.color.skin_protection = rec.color.skin_protection.max(50.0);
        // Specular highlights on cheekbones and forehead give skin its dimension: only
        // an oily, spread-out shine is tamed, and gently.
        if skin.shine >= 0.85 && current.texture.specular_balance < 12.0 {
            rec.texture.specular_balance = 12.0;
            changes.push("Specular balance 12 (oily shine only)".into());
        }
        if portrait && a.eyes && !rec.local.iter().any(|x| x.mask == MaskTarget::Eyes) {
            insights.push(Insight {
                topic: "Subject",
                observation: "Eyes are detected in the frame.".into(),
                why: "In a portrait the eyes are where the viewer looks first; a small lift and a touch of clarity there brings the whole face forward.".into(),
                how: "Step 3: local adjustment on Eyes, around +0.3 EV and +15 clarity. More than about +0.6 EV starts to look lit.".into(),
            });
            rec.local.push(LocalAdjustment { mask: MaskTarget::Eyes, exposure: 0.3, clarity: 15.0, ..Default::default() });
            changes.push("Eyes +0.30 EV, +15 clarity".into());
        }
    }

    // Skin balance for the skin's own depth: every tone is judged against its own
    // natural range, never pushed towards another.
    if let Some(fix) = a.skin_fix {
        let deep = scene.skin.as_ref().is_some_and(|s| s.tone_depth >= 7.0);
        let (observation, why) = match fix.problem {
            SkinProblem::Exposure if fix.exposure > 0.0 => (
                "Much of the skin is close to black, so its shape and texture are lost.".to_string(),
                "Deep skin carries its detail in the shadows; lifting only the skin brings back that detail without greying it or lightening the rest of the frame.".to_string(),
            ),
            SkinProblem::Exposure => (
                "Much of the skin is at or near clipping.".to_string(),
                "Fair skin is the first thing to lose detail in bright light; pulling only the skin back keeps its gradation without darkening the scene.".to_string(),
            ),
            p => (
                format!("Skin in the current edit reads {}.", p.words()),
                if deep && p == SkinProblem::Ashy {
                    "Deep skin turns grey under cool light or a cool grade; its richness is warmth and colour, not brightness, so the fix is colour on the skin alone.".into()
                } else if p == SkinProblem::Orange || p == SkinProblem::Magenta {
                    "Too much colour makes skin look sunburnt or made-up; calming it on the skin alone keeps the rest of the grade.".into()
                } else {
                    "A cast on skin reads as ill health or bad light; correcting it on the skin alone leaves the scene's mood intact.".into()
                },
            ),
        };
        let mut parts = Vec::new();
        if fix.warmth != 0.0 || fix.tint != 0.0 {
            parts.push(format!("warmth {:+.0}, tint {:+.0}", fix.warmth, fix.tint));
        }
        if fix.saturation != 0.0 {
            parts.push(format!("saturation {:+.0}", fix.saturation));
        }
        if fix.exposure != 0.0 {
            parts.push(format!("{:+.2} EV", fix.exposure));
        }
        insights.push(Insight {
            topic: "Skin",
            observation,
            why,
            how: format!("Step 3: local adjustment on Skin, {} (measured on this photo's skin).", parts.join(", ")),
        });
        upsert_local(&mut rec, MaskTarget::Skin, |l| {
            if fix.warmth != 0.0 || fix.tint != 0.0 || fix.saturation != 0.0 {
                (l.warmth, l.tint, l.saturation) = (fix.warmth, fix.tint, fix.saturation);
            }
            if fix.exposure != 0.0 {
                l.exposure = fix.exposure;
            }
        });
        changes.push(format!("Skin {}", parts.join(", ")));
    }

    // Subject and background separation.
    let has_subject_local = rec.local.iter().any(|x| x.mask == MaskTarget::Subject);
    if let (true, Some((on, off))) = (a.subject && !has_subject_local, a.separation) {
        if on < off - 0.5 {
            let lift = ((off - on) * 0.3).clamp(0.2, 0.45);
            insights.push(Insight {
                topic: "Subject",
                observation: format!("The subject is {:.1} stops darker than its surroundings.", off - on),
                why: "The eye goes to the brightest part of a picture first; a subject darker than the background competes with it.".into(),
                how: format!("Step 3: local adjustment on Subject, about {lift:+.2} EV (rather than brightening everything)."),
            });
            upsert_local(&mut rec, MaskTarget::Subject, |l| l.exposure = (lift * 20.0).round() / 20.0);
            changes.push(format!("Subject {lift:+.2} EV"));
        }
    }
    if let (true, true, Some(warm)) = (a.subject, portrait, a.background_warmth) {
        if warm > 0.02 && !rec.local.iter().any(|x| x.mask == MaskTarget::Background) {
            insights.push(Insight {
                topic: "Subject",
                observation: "The background is warm, close to the skin's own colour.".into(),
                why: "Skin separates best against cooler, quieter colour; a warm background blends the person into it.".into(),
                how: "Step 3: local adjustment on Background, warmth about −20 and saturation −10.".into(),
            });
            upsert_local(&mut rec, MaskTarget::Background, |l| {
                l.warmth = -20.0;
                l.saturation = -10.0;
            });
            changes.push("Background cooler (warmth −20, saturation −10)".into());
        }
    }

    // Composition: who the picture is about, then how to frame them.
    if let Some(p) = a.subjects.first().filter(|_| a.subjects.len() > 1) {
        let secondary = a.subjects.len() - 1;
        insights.push(Insight {
            topic: "Subject",
            observation: format!(
                "Primary subject: {} ({:.0}% of the frame{}); {secondary} secondary subject{}.",
                if p.person { "a person" } else { "the largest, nearest subject" },
                100.0 * p.area,
                p.nearness.map_or(String::new(), |n| format!(", {}", if n > 0.6 { "near" } else if n > 0.35 { "mid-distance" } else { "far" })),
                if secondary == 1 { "" } else { "s" }
            ),
            why: "Framing, local light and colour decisions follow the primary subject; secondary subjects support it and shouldn't compete.".into(),
            how: "Keep local lifts (Step 3 Subject) and the crop centred on the primary subject.".into(),
        });
    }
    if let Some(c) = &a.crop {
        let person = a.subjects.first().is_some_and(|p| p.person);
        insights.push(Insight {
            topic: "Framing",
            observation: format!("A crop that {}.", c.reason.trim_start_matches("A tighter crop ")),
            why: if person {
                "Cutting at the neck, waist, knees or ankles reads as an amputation; a full-body frame with ground below the feet, or a clean three-quarter at mid-thigh, keeps the person whole. Anatomy and impact come before exact alignment.".into()
            } else {
                "Placing the subject and the horizon on the thirds gives the picture direction and room; a centred subject and a horizon through the middle split it in two.".into()
            },
            how: format!(
                "Apply suggested crop below, or Step 1: Crop & straighten{}.",
                if c.rotation.abs() >= 0.3 { format!(" at {:+.1}°", c.rotation) } else { String::new() }
            ),
        });
    }

    // A style to start from, by genre.
    let style = match genre.map(|g| g.id) {
        Some("dark-melanin-fashion") => Some(("dark-melanin-glow", 70.0)),
        Some("close-up-portrait" | "environmental-portrait") => Some(("soft-editorial", 70.0)),
        Some("golden-hour-landscape") => Some(("golden-hour-flare", 75.0)),
        Some("wildlife") => Some(("deep-emerald", 70.0)),
        Some("street") => Some(("ilford-hp5", 55.0)),
        Some("architectural") => Some(("vivid-alpine", 75.0)),
        Some("fine-art-wedding") => Some(("fuji-pro-400h", 45.0)),
        Some("astrophotography") => Some(("moody-pacific", 70.0)),
        _ => None,
    };
    if let (Some(g), Some((id, protection))) = (genre, style) {
        if current.style.is_none() {
            insights.push(Insight {
                topic: "Style",
                observation: format!("This reads as {} ({:.0}% match: {}).", g.label, 100.0 * g.score, g.evidence),
                why: "A style that suits the genre gives a coherent starting look you can then adjust, rather than building it from sliders.".into(),
                how: format!("Presets & Styles: try {} at about 70% amount.", style_name(id)),
            });
            rec.style.id = id.into();
            rec.style.amount = 70.0;
            rec.style.skin_protection = protection;
            rec.style.blend.clear();
            changes.push(format!("{} style at 70%", style_name(id)));
        }
    }

    let summary = match (genre, insights.first()) {
        (Some(g), Some(first)) => format!("{} · first priority: {}", g.label, first.topic.to_lowercase()),
        (Some(g), None) => format!("{}: well exposed and level, a good base to grade from", g.label),
        (None, Some(first)) => format!("First priority: {}", first.topic.to_lowercase()),
        (None, None) => "Well exposed and level: a good base to grade from".into(),
    };
    MentorReport {
        summary,
        insights,
        recommended: rec,
        changes,
        crop: a.crop.clone(),
        target: String::new(),
        subjects: a.subjects.to_vec(),
        analysis_ms: a.started.elapsed().as_millis() as u64,
    }
}

/// What fitting to the target changed, in words.
fn describe_fit(before: &Adjustments, after: &Adjustments) -> Vec<String> {
    let mut out = Vec::new();
    let changed = |a: f32, b: f32| (a - b).abs() >= 1.0;
    if (after.exposure - before.exposure).abs() >= 0.05 {
        out.push(format!("Exposure {:+.2} EV (matched to the look's brightness)", after.exposure));
    }
    if changed(after.tone.blacks, before.tone.blacks) {
        out.push(format!("Blacks {:+.0} (black point)", after.tone.blacks));
    }
    if changed(after.tone.whites, before.tone.whites) {
        out.push(format!("Whites {:+.0} (white point)", after.tone.whites));
    }
    if changed(after.tone.contrast, before.tone.contrast) {
        out.push(format!("Contrast {:+.0}", after.tone.contrast));
    }
    let (s0, s1) = (&before.curves.s_curve, &after.curves.s_curve);
    if s1.enabled && (!s0.enabled || changed(s1.amount, s0.amount)) {
        out.push(format!("Midtone S-curve {:.0}", s1.amount));
    } else if s0.enabled && !s1.enabled {
        out.push("S-curve off".into());
    }
    let (f0, f1) = (&before.color.foliage, &after.color.foliage);
    if changed(f1.hue, f0.hue) || changed(f1.saturation, f0.saturation) {
        out.push(format!("Foliage hue {:+.0} (towards olive), saturation {:+.0}", f1.hue, f1.saturation));
    }
    let skin = |a: &Adjustments| a.local.iter().find(|l| l.mask == MaskTarget::Skin).cloned().unwrap_or_default();
    let (k0, k1) = (skin(before), skin(after));
    if changed(k1.warmth, k0.warmth) || changed(k1.tint, k0.tint) || changed(k1.saturation, k0.saturation) || (k1.exposure - k0.exposure).abs() >= 0.05 {
        out.push(format!(
            "Skin warmth {:+.0}, tint {:+.0}, saturation {:+.0}{}",
            k1.warmth,
            k1.tint,
            k1.saturation,
            if k1.exposure != 0.0 { format!(", {:+.2} EV", k1.exposure) } else { String::new() }
        ));
    }
    if after.texture.specular_balance < before.texture.specular_balance {
        out.push("Specular highlights kept (balance 0)".into());
    }
    let hsl = before.color.hsl.bands().iter().zip(after.color.hsl.bands()).filter(|(a, b)| **a != *b).count();
    if hsl > 0 {
        out.push(format!("HSL: {hsl} colour band{} matched", if hsl == 1 { "" } else { "s" }));
    }
    if changed(after.color.background.saturation, before.color.background.saturation) {
        out.push(format!("Background saturation {:+.0}", after.color.background.saturation));
    }
    out
}

/// One insight saying what the target look is and what it set.
fn insights_for_target(
    report: &mut MentorReport,
    target: &Target,
    learned: bool,
    pro: Option<(&str, &str)>,
    moves: &[String],
) {
    let (observation, why) = if let (false, Some((summary, why))) = (learned, pro) {
        (format!("Starting point aimed at the {} profile: {summary}.", target.name), why.to_string())
    } else if learned {
        (
            format!("Starting point aimed at your learned style \"{}\": its black and white points, midtone curve, skin richness and colour.", target.name),
            "The look is measured from the reference and matched on this photo's own render, so each slider lands where the reference's look is, for this light.".to_string(),
        )
    } else {
        (
            "Starting point aimed at the Editorial profile: rich, anchored blacks, a clean white point, a midtone S-curve, skin rich and warm with its highlights kept, and foliage calmed towards olive.".to_string(),
            "Contrast and depth read as professional; lifting every shadow reads as flat HDR. Skin is judged within its own depth, so every skin tone keeps its own richness.".to_string(),
        )
    };
    report.insights.push(Insight {
        topic: "Style",
        observation,
        why,
        how: format!("Apply Recommended Starting Point: {}.", moves.join(", ")),
    });
    if report.summary.is_empty() || report.insights.len() == 1 {
        report.summary = format!("First priority: {}", report.insights[0].topic.to_lowercase());
    }
}

fn style_name(id: &str) -> &'static str {
    epikos_pipeline::styles().into_iter().find(|s| s.id == id).map_or("a style", |s| s.name)
}

fn judge(before: &Stats, now: &Stats, adj: &Adjustments) -> Vec<Feedback> {
    let mut out = Vec::new();
    let mut add = |level: &'static str, text: String, fix: Option<Fix>| {
        // Warnings and tips spell the correction out.
        let text = match &fix {
            Some(f) => format!("{text} Fix: {}.", f.label),
            None => text,
        };
        out.push(Feedback { level, text, fix });
    };
    let fix = || FixBuilder::new(adj);

    // Highlights and shadows.
    if now.clipped > 0.02 {
        let h = (adj.tone.highlights - 25.0).max(-100.0);
        let mut f = fix().set(format!("Highlights {}", signed(h)), |a| a.tone.highlights = h);
        if adj.tone.highlights <= -60.0 || now.clipped > 0.06 {
            let ev = ((adj.exposure - 0.25) * 100.0).round() / 100.0;
            f = f.set(format!("Exposure {ev:+.2} EV"), |a| a.exposure = ev);
        }
        add("warning", format!("{:.1}% of the image is clipping to white.", 100.0 * now.clipped), f.done());
    } else if before.clipped > 0.01 && now.clipped < 0.5 * before.clipped {
        add("praise", "Highlight detail recovered: the brights clip far less than in the original.".into(), None);
    }
    if now.crushed > 0.05 {
        let (sh, bl) = ((adj.tone.shadows + 15.0).min(100.0), (adj.tone.blacks + 10.0).min(100.0));
        let f = fix()
            .set(format!("Shadows {}", signed(sh)), |a| a.tone.shadows = sh)
            .set(format!("Blacks {}", signed(bl)), |a| a.tone.blacks = bl);
        add("warning", format!("{:.1}% of the image is crushed to black (fine if that's the look).", 100.0 * now.crushed), f.done());
    }
    // Mid-tones.
    if now.p50.abs() <= 0.8 && before.p50.abs() > 1.2 {
        add("praise", "Mid-tones now sit close to mid-grey: a well-judged exposure.".into(), None);
    } else if now.p50 < -2.5 {
        let ev = ((adj.exposure + (-1.2 - now.p50).clamp(0.3, 1.5)) * 100.0).round() / 100.0;
        let f = fix().set(format!("Exposure {ev:+.2} EV"), |a| a.exposure = ev);
        add("tip", "The image is overall very dark (fine for a low-key mood).".into(), f.done());
    } else if now.p50 > 1.5 {
        let ev = ((adj.exposure - (now.p50 - 0.7).clamp(0.3, 1.5)) * 100.0).round() / 100.0;
        let f = fix().set(format!("Exposure {ev:+.2} EV"), |a| a.exposure = ev);
        add("tip", "The image is overall very bright (fine for high-key).".into(), f.done());
    }
    // Contrast range.
    let (range_before, range_now) = (before.p98 - before.p02, now.p98 - now.p02);
    if range_now < 0.6 * range_before && range_before > 4.0 {
        let amount = adj.curves.s_curve.amount.max(30.0);
        let f = fix().set(format!("S-curve on, {amount:.0}"), |a| {
            a.curves.s_curve.enabled = true;
            a.curves.s_curve.amount = amount;
        });
        add("tip", "The tonal range has been compressed a lot; the image may look flat.".into(), f.done());
    }
    // Colour.
    if now.loud > 0.08 && now.loud > 1.5 * before.loud {
        let (sat, vib) = ((adj.tone.saturation - 15.0).max(-100.0), (adj.tone.vibrance - 10.0).max(-100.0));
        let f = fix()
            .set(format!("Saturation {}", signed(sat)), |a| a.tone.saturation = sat)
            .set(format!("Vibrance {}", signed(vib)), |a| a.tone.vibrance = vib);
        add("warning", format!("{:.0}% of the image is very saturated; colours may look artificial or posterise in print.", 100.0 * now.loud), f.done());
    }
    if let (Some(w), None) = (cast_words(before.cast), cast_words(now.cast)) {
        add("praise", format!("The {w} cast is neutralised: whites and greys look clean."), None);
    }
    // Skin.
    let protection = |a: &Adjustments| if a.style.is_none() { a.color.skin_protection } else { a.style.skin_protection };
    let set_protection = |a: &mut Adjustments, v: f32| {
        if a.style.is_none() {
            a.color.skin_protection = v;
        } else {
            a.style.skin_protection = v;
        }
    };
    let skin_now = adj.local.iter().find(|l| l.mask == MaskTarget::Skin).cloned().unwrap_or_default();
    match (before.skin, now.skin) {
        (Some((l0, h0)), Some((l1, h1))) => {
            let dh = (h1 - h0 + 540.0).rem_euclid(360.0) - 180.0;
            if dh.abs() > 8.0 {
                let towards = if dh > 0.0 { "yellow/green" } else { "red/magenta" };
                // Back towards the original hue: magenta against yellow-green, warmth
                // and green against red-magenta.
                let (dw, dt) = if dh > 0.0 { (-3.0, (dh * 0.4).clamp(3.0, 12.0)) } else { (5.0, -(dh.abs() * 0.4).clamp(3.0, 12.0)) };
                let (w, t) = ((skin_now.warmth + dw).clamp(-100.0, 100.0), (skin_now.tint + dt).clamp(-100.0, 100.0));
                let p = (protection(adj) + 25.0).clamp(25.0, 100.0);
                let f = fix()
                    .skin(format!("Skin warmth {}", signed(w)), |l| l.warmth = w)
                    .set(format!("Skin protection {p:.0}%"), |a| set_protection(a, p))
                    .skin(format!("Skin tint {}", signed(t)), |l| l.tint = t);
                add("warning", format!("Skin hue has shifted {:.0}° towards {towards}.", dh.abs()), f.done());
            } else if (l1 - l0).abs() < 0.06 {
                add("praise", "Skin tones are balanced: natural hue and depth, kept through the grade.".into(), None);
            }
            if l1 < l0 - 0.08 {
                let ev = ((skin_now.exposure + 0.2).min(1.0) * 100.0).round() / 100.0;
                let f = fix().skin(format!("Skin exposure {ev:+.2} EV"), |l| l.exposure = ev);
                add("tip", "Skin has become noticeably darker than in the original and may look muddy.".into(), f.done());
            }
        }
        (Some(_), None) => {
            let p = protection(adj).max(70.0);
            let mut f = fix().set(format!("Skin protection {p:.0}%"), |a| set_protection(a, p));
            if !adj.style.is_none() {
                let amt = (adj.style.amount - 30.0).max(20.0);
                f = f.set(format!("Style amount {amt:.0}%"), |a| a.style.amount = amt);
            }
            add("warning", "Skin is barely recognisable as skin after the grade.".into(), f.done());
        }
        _ => {}
    }
    // Specific settings.
    if let Some(i) = adj.local.iter().position(|l| l.mask == MaskTarget::Eyes && l.exposure > 0.6) {
        let f = fix().set("Eyes exposure +0.45 EV", |a| a.local[i].exposure = 0.45);
        add("warning", "Eyes are lifted more than +0.6 EV and may look lit rather than bright.".into(), f.done());
    }
    if adj.texture.clarity > 60.0 || adj.texture.micro_texture > 70.0 {
        let (c, m) = (adj.texture.clarity.min(40.0), adj.texture.micro_texture.min(50.0));
        let f = fix()
            .set(format!("Clarity {}", signed(c)), |a| a.texture.clarity = c)
            .set(format!("Micro-texture {}", signed(m)), |a| a.texture.micro_texture = m);
        add("tip", "Strong clarity or micro-texture can add halos and age faces.".into(), f.done());
    }
    if adj.lens.rotation != 0.0 || adj.lens.vertical != 0.0 {
        add("praise", "Framing straightened: lines now read as intentional.".into(), None);
    }
    if out.is_empty() {
        out.push(Feedback { level: "praise", text: "Nothing to flag: highlights, shadows and colour are all in a healthy range.".into(), fix: None });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats_with(clipped: f32, p50: f32, cast: (f32, f32), skin: Option<(f32, f32)>) -> Stats {
        Stats { p02: -5.0, p50, p98: 2.0, clipped, crushed: 0.0, cast, loud: 0.0, skin }
    }

    #[test]
    fn casts_are_named_by_direction() {
        assert_eq!(cast_words((0.0, 0.0)), None);
        assert_eq!(cast_words((0.0, -0.03)), Some("blue"));
        assert_eq!(cast_words((0.005, 0.03)), Some("warm yellow-orange"));
        assert_eq!(cast_words((-0.03, 0.01)), Some("green"));
    }

    #[test]
    fn feedback_warns_on_clipping_and_skin_shift_and_praises_recovery() {
        let adj = Adjustments::default();
        let before = stats_with(0.05, 0.0, (0.0, 0.0), Some((0.6, 60.0)));
        let clipping = judge(&before, &stats_with(0.06, 0.0, (0.0, 0.0), Some((0.6, 60.0))), &adj);
        let clip = clipping.iter().find(|f| f.level == "warning" && f.text.contains("clipping")).expect("clipping warning");
        let fix = clip.fix.as_ref().expect("a fix");
        assert!(fix.adjustments.tone.highlights < adj.tone.highlights, "{}", fix.label);
        assert!(clip.text.contains("Fix: Highlights"), "{}", clip.text);
        let recovered = judge(&before, &stats_with(0.005, 0.0, (0.0, 0.0), Some((0.6, 61.0))), &adj);
        assert!(recovered.iter().any(|f| f.level == "praise" && f.text.contains("Highlight detail")));
        assert!(recovered.iter().any(|f| f.level == "praise" && f.text.contains("Skin tones are balanced")));
        let shifted = judge(&before, &stats_with(0.0, 0.0, (0.0, 0.0), Some((0.6, 75.0))), &adj);
        let shift = shifted.iter().find(|f| f.level == "warning" && f.text.contains("yellow/green")).expect("skin shift");
        let fix = shift.fix.as_ref().expect("a fix");
        let skin = fix.adjustments.local.iter().find(|l| l.mask == MaskTarget::Skin).expect("skin fix");
        assert!(skin.tint > 0.0, "magenta against yellow-green: {}", fix.label);
        assert!(fix.adjustments.color.skin_protection >= 25.0);
        assert!(recovered.iter().all(|f| f.level != "praise" || f.fix.is_none()));
    }

    #[test]
    fn a_healthy_edit_still_gets_a_word() {
        let s = stats_with(0.0, 0.0, (0.0, 0.0), None);
        let f = judge(&s, &s, &Adjustments::default());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].level, "praise");
    }
}
