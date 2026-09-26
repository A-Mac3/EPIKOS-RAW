//! PRD Step 6: atmospheric and light sculpting, on scene-linear Rec.2020 (upright).
//!
//! - **Glow**: soft bloom around bright areas.
//! - **Fog**: Koschmieder aerial perspective, `I·t + A·(1 − t)` with `t = e^(−k·d)`,
//!   where `d` is distance from the depth model (uniform without one) and the airlight
//!   `A` follows the scene's own bright level, so it reads alike at any exposure.
//! - **Light shafts**: screen-space volumetric rays (Mitchell, GPU Gems 3 ch. 13): the
//!   bright, distant parts of the frame are smeared radially towards the light, so
//!   anything in front of them casts streaks. Computed at ≤ 640 px (rays are soft)
//!   and kept off near subjects when depth is known.
//! - **Haze**: a uniform warm veil (styles only).

use epikos_core::{resize_plane, ImageRgbF32};
use rayon::prelude::*;

use super::blur::smooth;
use super::{long_side, smoothstep};

/// Normalised Step 6 parameters: strengths 0…1, warmth −1 (cool) … 1 (gold).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AtmosphereParams {
    pub glow: f32,
    /// Bloom radius as a fraction of the long side.
    pub glow_radius: f32,
    pub glow_warmth: f32,
    pub fog: f32,
    /// Normalised distance (0 = camera, 1 = farthest) where fog begins.
    pub fog_start: f32,
    pub fog_warmth: f32,
    pub shafts: f32,
    /// Fraction of the way from each pixel to the light that rays reach back.
    pub shaft_length: f32,
    pub shaft_warmth: f32,
    /// Light position (0–1 across, down); `None` = find it.
    pub light: Option<(f32, f32)>,
    /// Uniform veil.
    pub haze: f32,
    pub haze_warmth: f32,
}

impl Default for AtmosphereParams {
    fn default() -> Self {
        Self {
            glow: 0.0,
            glow_radius: 0.02,
            glow_warmth: 0.0,
            fog: 0.0,
            fog_start: 0.3,
            fog_warmth: 0.0,
            shafts: 0.0,
            shaft_length: 0.6,
            shaft_warmth: 0.4,
            light: None,
            haze: 0.0,
            haze_warmth: 0.0,
        }
    }
}

impl AtmosphereParams {
    pub(crate) fn is_neutral(&self) -> bool {
        self.glow <= 0.0 && self.fog <= 0.0 && self.shafts <= 0.0 && self.haze <= 0.0
    }

    pub(crate) fn needs_depth(&self) -> bool {
        self.fog > 0.0 || self.shafts > 0.0
    }

    /// Strengths times `k`; shapes and colours stay.
    pub(crate) fn scaled(mut self, k: f32) -> Self {
        self.glow *= k;
        self.fog *= k;
        self.shafts *= k;
        self.haze *= k;
        self
    }

    /// Manual settings plus a style's: strengths add, shapes and colours are averaged
    /// by strength. A manually placed light always wins.
    pub(crate) fn plus(self, o: Self) -> Self {
        let mix = |a: f32, wa: f32, b: f32, wb: f32| {
            if wa + wb > 0.0 { (a * wa + b * wb) / (wa + wb) } else { a }
        };
        Self {
            glow: (self.glow + o.glow).min(1.0),
            glow_radius: mix(self.glow_radius, self.glow, o.glow_radius, o.glow),
            glow_warmth: mix(self.glow_warmth, self.glow, o.glow_warmth, o.glow),
            fog: (self.fog + o.fog).min(1.0),
            fog_start: mix(self.fog_start, self.fog, o.fog_start, o.fog),
            fog_warmth: mix(self.fog_warmth, self.fog, o.fog_warmth, o.fog),
            shafts: (self.shafts + o.shafts).min(1.0),
            shaft_length: mix(self.shaft_length, self.shafts, o.shaft_length, o.shafts),
            shaft_warmth: mix(self.shaft_warmth, self.shafts, o.shaft_warmth, o.shafts),
            light: self.light.or(o.light),
            haze: (self.haze + o.haze).min(1.0),
            haze_warmth: mix(self.haze_warmth, self.haze, o.haze_warmth, o.haze),
        }
    }
}

/// Low-sun gold and cool daylight blue in linear Rec.2020, luminance ≈ 1.
const GOLD: [f32; 3] = [1.45, 0.86, 0.42];
const COOL: [f32; 3] = [0.85, 1.03, 1.35];
/// Scene-linear level of the haze veil at full strength (mid-grey is 0.18).
const HAZE_LEVEL: f32 = 0.03;
/// Fog optical depth at full strength and full distance (transmission e^−3 ≈ 5%).
const FOG_DENSITY: f32 = 3.0;
/// Long side of the working image for light shafts.
const SHAFT_SIDE: f32 = 640.0;
const SHAFT_SAMPLES: usize = 64;

pub(crate) fn tint(warmth: f32) -> [f32; 3] {
    let w = warmth.clamp(-1.0, 1.0);
    let target = if w >= 0.0 { GOLD } else { COOL };
    target.map(|t| 1.0 + (t - 1.0) * w.abs())
}

fn luminance(rgb: &ImageRgbF32) -> Vec<f32> {
    (0..rgb.len())
        .into_par_iter()
        .map(|i| (0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(0.0))
        .collect()
}

/// `p`-th percentile (0–1) from a strided sample.
pub(crate) fn percentile(v: &[f32], p: f32) -> f32 {
    let stride = (v.len() / 65_536).max(1);
    let mut s: Vec<f32> = v.iter().step_by(stride).copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return 0.0;
    }
    let k = ((s.len() - 1) as f32 * p) as usize;
    *s.select_nth_unstable_by(k, f32::total_cmp).1
}

/// Scene masks the atmosphere uses when the engine has them (all at the image's size).
#[derive(Default, Clone, Copy)]
pub(crate) struct Scene<'a> {
    /// 1 = near, 0 = far.
    pub depth: Option<&'a [f32]>,
    /// Only this region glows (Step 6 "subject only").
    pub glow_mask: Option<&'a [f32]>,
    /// Where the light may come from.
    pub sky: Option<&'a [f32]>,
    /// Never a light source, and a gentler bloom.
    pub subject: Option<&'a [f32]>,
}

/// With `scene.glow_mask` (the subject), only that region glows, and from a lower
/// brightness so a subject in ordinary light still gets a bloom. Otherwise the bloom
/// starts at the photo's own highlights (not a fixed level, which would bloom a whole
/// bright frame) and is kept off the subject and the near foreground, so a reflection
/// on a wheel or a lit face doesn't turn into a light orb. Glow and rays are added with
/// a soft ceiling, so bright areas brighten less and never blow out into white blobs.
pub(crate) fn apply_atmosphere(rgb: &mut ImageRgbF32, p: &AtmosphereParams, scene: Scene) {
    let (depth, glow_mask) = (scene.depth, scene.glow_mask);
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    if p.is_neutral() || w < 4 || h < 4 {
        return;
    }
    let y = luminance(rgb);

    let bloom = (p.glow > 0.0).then(|| {
        let bright: Vec<f32> = match glow_mask {
            Some(m) => y.par_iter().zip(m.par_iter()).map(|(&v, &m)| m * v * smoothstep(0.05, 0.6, v)).collect(),
            None => {
                let (lo, hi) = (percentile(&y, 0.9).max(0.05), percentile(&y, 0.995).max(0.1));
                let (lo, hi) = (lo.min(0.9 * hi), hi.max(lo * 1.1));
                (0..y.len())
                    .into_par_iter()
                    .map(|i| {
                        let keep_subject = scene.subject.map_or(1.0, |s| 1.0 - 0.85 * s[i]);
                        let keep_near = depth.map_or(1.0, |d| 1.0 - 0.7 * smoothstep(0.6, 0.9, d[i]));
                        y[i] * smoothstep(lo, hi, y[i]) * keep_subject * keep_near
                    })
                    .collect()
            }
        };
        let r = (p.glow_radius * long_side(w, h)).round().max(2.0) as usize;
        smooth(&bright, w, h, r, 3)
    });
    let rays = (p.shafts > 0.0).then(|| light_shafts(&y, w, h, p, scene));
    // Airlight: a little below the scene's bright level.
    let airlight = (0.85 * percentile(&y, 0.95)).max(0.02);

    let (glow_tint, fog_tint, shaft_tint, haze_tint) =
        (tint(p.glow_warmth), tint(p.fog_warmth), tint(p.shaft_warmth), tint(p.haze_warmth));
    let span = (1.0 - p.fog_start).max(1e-3);
    let keep = 1.0 - 0.15 * p.haze;
    let veil = p.haze * HAZE_LEVEL;
    for (c, plane) in [&mut rgb.r, &mut rgb.g, &mut rgb.b].into_iter().enumerate() {
        plane.par_iter_mut().enumerate().for_each(|(i, v)| {
            let mut out = *v;
            if p.fog > 0.0 {
                // Without depth, fog is a uniform haze at mid distance.
                let d = depth.map_or(0.5, |d| ((1.0 - d[i] - p.fog_start) / span).clamp(0.0, 1.0));
                let t = (-FOG_DENSITY * p.fog * d).exp();
                out = out * t + airlight * fog_tint[c] * (1.0 - t);
            }
            // Light added with a soft ceiling: the brighter a pixel already is, the less it
            // gains, so glow and rays can't pile up into clipped blobs.
            let mut add = 0.0;
            if let Some(b) = &bloom {
                add += 0.8 * p.glow * b[i] * glow_tint[c];
            }
            if let Some(r) = &rays {
                add += p.shafts * r[i] * shaft_tint[c];
            }
            if add > 0.0 {
                out += add / (1.0 + out.max(0.0).powi(2));
            }
            *v = out * keep + veil * haze_tint[c];
        });
    }
}

/// Ray intensity at full resolution.
fn light_shafts(y: &[f32], w: usize, h: usize, p: &AtmosphereParams, scene: Scene) -> Vec<f32> {
    let depth = scene.depth;
    let scale = (SHAFT_SIDE / long_side(w, h)).min(1.0);
    let (sw, sh) = (((w as f32 * scale).round() as usize).max(2), ((h as f32 * scale).round() as usize).max(2));
    let small = resize_plane(y, w as u32, h as u32, sw as u32, sh as u32);
    let far = depth.map(|d| resize_plane(d, w as u32, h as u32, sw as u32, sh as u32));
    let shrink = |m: &[f32]| resize_plane(m, w as u32, h as u32, sw as u32, sh as u32);
    let sky = scene.sky.map(shrink);
    let subject = scene.subject.map(shrink);

    // Rays come from what is bright, and — when depth is known — far away (sky, backlight).
    // Relative to the brightest part of the frame, so a large bright sky still counts.
    let hi = percentile(&small, 0.995).max(1e-4);
    let lo = 0.5 * hi;
    let source: Vec<f32> = small
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let behind = far.as_ref().map_or(1.0, |d| (1.0 - d[i]).powi(2));
            v * smoothstep(lo, hi, v) * behind
        })
        .collect();

    let (lx, ly) = match p.light {
        Some((x, y)) => (x * sw as f32, y * sh as f32),
        None => find_light(&source, sw, sh, sky.as_deref(), subject.as_deref()),
    };
    let len = p.shaft_length.clamp(0.05, 1.0);
    let decay: f32 = 0.965;
    let norm: f32 = (0..SHAFT_SAMPLES).map(|i| decay.powi(i as i32)).sum();
    let sample = |x: f32, y: f32| {
        let x = x.clamp(0.0, (sw - 1) as f32);
        let y = y.clamp(0.0, (sh - 1) as f32);
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
        let (tx, ty) = (x - x0 as f32, y - y0 as f32);
        let row = |yy: usize| source[yy * sw + x0] * (1.0 - tx) + source[yy * sw + x1] * tx;
        row(y0) * (1.0 - ty) + row(y1) * ty
    };
    let rays: Vec<f32> = (0..sw * sh)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % sw) as f32, (i / sw) as f32);
            let (dx, dy) = ((lx - x) * len, (ly - y) * len);
            let mut sum = 0.0;
            let mut wgt = 1.0;
            for k in 0..SHAFT_SAMPLES {
                let t = k as f32 / SHAFT_SAMPLES as f32;
                sum += wgt * sample(x + dx * t, y + dy * t);
                wgt *= decay;
            }
            // 1.5: rays should read, not merely lift the shadows.
            1.5 * sum / norm
        })
        .collect();
    let mut full = resize_plane(&rays, sw as u32, sh as u32, w as u32, h as u32);
    if let Some(d) = depth {
        // Keep near subjects clear: the rays are in the air behind them.
        full.par_iter_mut().zip(d.par_iter()).for_each(|(r, &d)| *r *= 1.0 - smoothstep(0.4, 0.8, d));
    }
    full
}

/// The light: the strongest blob of the bright source in the upper three quarters of
/// the frame (low sun and backlight sit there), in working-image pixels.
///
/// With a sky mask covering a real part of the frame, the light is looked for only in
/// the sky; with the sky model but hardly any sky visible, the light is out of frame, so
/// only along the top and side edges; without the model, in the upper three quarters.
/// The subject is always excluded, so the source never lands on a person or a bright
/// foreground object.
fn find_light(source: &[f32], w: usize, h: usize, sky: Option<&[f32]>, subject: Option<&[f32]>) -> (f32, f32) {
    let r = (w.max(h) / 40).max(1);
    let n = w * h;
    let sky_share = sky.map_or(0.0, |s| s.iter().filter(|&&v| v > 0.5).count() as f32 / n as f32);
    let edge = |i: usize| {
        let (x, y) = (i % w, i / w);
        let band = (w.min(h) as f32 * 0.2).max(1.0);
        let d = (x.min(w - 1 - x)).min(y) as f32; // top and sides, not the bottom
        1.0 - smoothstep(0.0, band, d)
    };
    let allowed: Vec<f32> = (0..n)
        .map(|i| {
            let region = match sky {
                Some(s) if sky_share >= 0.02 => s[i],
                Some(_) => edge(i),
                None => (i / w < h * 3 / 4) as u8 as f32,
            };
            region * subject.map_or(1.0, |s| 1.0 - s[i])
        })
        .collect();
    let weighted: Vec<f32> = source.iter().zip(&allowed).map(|(v, a)| v * a).collect();
    let blurred = smooth(&weighted, w, h, r, 2);
    let (best, value) = blurred.iter().enumerate().fold((usize::MAX, f32::MIN), |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc });
    if best == usize::MAX || value <= 0.0 {
        // Nothing bright where a light may be: just above the top centre.
        return (w as f32 / 2.0, -0.05 * h as f32);
    }
    ((best % w) as f32, (best / w) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    fn flat(w: u32, h: u32, v: f32) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
        for p in [&mut img.r, &mut img.g, &mut img.b] {
            p.fill(v);
        }
        img
    }

    #[test]
    fn glow_spreads_light_from_a_bright_spot_and_haze_lifts_black() {
        let mut img = flat(200, 200, 0.0);
        for y in 95..105 {
            for x in 95..105 {
                let i = img.index(x, y);
                (img.r[i], img.g[i], img.b[i]) = (4.0, 4.0, 4.0);
            }
        }
        let near = img.index(110, 100);
        let p = AtmosphereParams { glow: 1.0, glow_warmth: 1.0, haze: 1.0, ..Default::default() };
        apply_atmosphere(&mut img, &p, Scene::default());
        assert!(img.r[near] > 0.03, "no bloom: {}", img.r[near]);
        assert!(img.r[near] > img.b[near], "bloom not warm");
        assert!(img.g[img.index(0, 0)] > 0.02, "black not lifted");
    }

    #[test]
    fn fog_thickens_with_distance() {
        // Dark scene; left half near, right half far.
        let mut img = flat(64, 32, 0.02);
        for i in 0..64 * 8 {
            // Bright sky across the top quarter sets the airlight level.
            (img.r[i], img.g[i], img.b[i]) = (0.6, 0.6, 0.6);
        }
        let depth: Vec<f32> = (0..img.len()).map(|i| if i % 64 < 32 { 1.0 } else { 0.0 }).collect();
        let p = AtmosphereParams { fog: 1.0, fog_start: 0.0, ..Default::default() };
        let (near, far) = (img.index(10, 20), img.index(50, 20));
        apply_atmosphere(&mut img, &p, Scene { depth: Some(&depth), ..Default::default() });
        assert!((img.g[near] - 0.02).abs() < 1e-4, "near fogged: {}", img.g[near]);
        assert!(img.g[far] > 0.2, "far not fogged: {}", img.g[far]);
    }

    #[test]
    fn shafts_stream_away_from_the_light_past_an_occluder() {
        // Bright sky at the top with a dark post in it; light behind the post.
        let (w, h) = (200u32, 200u32);
        let mut img = flat(w, h, 0.03);
        for y in 0..60 {
            for x in 0..w {
                if !(95..105).contains(&x) {
                    let i = img.index(x, y);
                    (img.r[i], img.g[i], img.b[i]) = (1.0, 1.0, 1.0);
                }
            }
        }
        let p = AtmosphereParams {
            shafts: 1.0,
            shaft_length: 1.0,
            shaft_warmth: 0.0,
            light: Some((0.5, 0.05)),
            ..Default::default()
        };
        let before = img.clone();
        apply_atmosphere(&mut img, &p, Scene::default());
        let gain = |x: u32, y: u32| img.g[img.index(x, y)] - before.g[before.index(x, y)];
        // Below the sky, rays light the ground either side of the post's shadow.
        assert!(gain(70, 120) > 0.02, "no rays: {}", gain(70, 120));
        assert!(gain(100, 120) < 0.7 * gain(70, 120), "post casts no shadow ray");
    }

    #[test]
    fn auto_light_finds_the_bright_patch() {
        let (w, h) = (80, 60);
        let mut src = vec![0.0; w * h];
        for y in 8..14 {
            for x in 58..66 {
                src[y * w + x] = 1.0;
            }
        }
        let (x, y) = find_light(&src, w, h, None, None);
        assert!((58.0..66.0).contains(&x) && (8.0..14.0).contains(&y), "{x} {y}");
    }

    #[test]
    fn the_light_stays_in_the_sky_and_off_the_subject() {
        let (w, h) = (80, 60);
        // A brighter reflection low in the frame (a wheel), a dimmer glow in the sky.
        let mut src = vec![0.0; w * h];
        for y in 40..46 {
            for x in 10..16 {
                src[y * w + x] = 2.0;
            }
        }
        for y in 4..10 {
            for x in 50..58 {
                src[y * w + x] = 0.6;
            }
        }
        let sky: Vec<f32> = (0..w * h).map(|i| if i / w < 20 { 1.0 } else { 0.0 }).collect();
        let (x, y) = find_light(&src, w, h, Some(&sky), None);
        assert!((50.0..58.0).contains(&x) && (4.0..10.0).contains(&y), "sky light at {x} {y}");

        // No sky in view: never on the subject, and at an edge.
        let no_sky = vec![0.0; w * h];
        let subject: Vec<f32> = (0..w * h).map(|i| if (30..50).contains(&(i % w)) { 1.0 } else { 0.0 }).collect();
        let mut bright_on_subject = vec![0.0; w * h];
        for y in 2..8 {
            for x in 36..44 {
                bright_on_subject[y * w + x] = 3.0;
            }
            bright_on_subject[y * w + 3] = 0.5;
        }
        let (x, _) = find_light(&bright_on_subject, w, h, Some(&no_sky), Some(&subject));
        assert!(!(30.0..50.0).contains(&x), "light on the subject at x = {x}");
    }

    #[test]
    fn glow_never_blows_bright_areas_out() {
        let mut img = flat(100, 100, 0.9);
        for y in 40..60 {
            for x in 40..60 {
                let i = img.index(x, y);
                (img.r[i], img.g[i], img.b[i]) = (3.0, 3.0, 3.0);
            }
        }
        let before = img.clone();
        let p = AtmosphereParams { glow: 1.0, glow_radius: 0.05, ..Default::default() };
        apply_atmosphere(&mut img, &p, Scene::default());
        let c = img.index(50, 50);
        // The brightest area gains a fraction of what a dark one would.
        assert!(img.g[c] - before.g[c] < 0.3, "centre gained {}", img.g[c] - before.g[c]);
    }
}
