//! Parametric Style Engine (PRD Section 3).
//!
//! A style is not a LUT: it is a point in the same parameter space the Step 4–6
//! controls use (texture, HSL, wheels, glow, …), split into
//!
//! - a **scene** grade, which skin is shielded from by the style's skin protection,
//! - a **skin** grade, applied only where skin is detected (e.g. melanin richness),
//! - texture and atmosphere.
//!
//! Applying a style at amount `k` scales every strength by `k`, so styles also blend
//! linearly (the groundwork for the PRD's Style Fusion Matrix). Styles layer on top of
//! the photographer's own Step 4–5 settings, which stay editable.

use serde::Serialize;

use super::atmosphere::AtmosphereParams;
use super::grade::ColorParams;
use super::texture::TextureParams;

/// What the UI shows for a style.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// Style world from the PRD taxonomy.
    pub world: &'static str,
    pub description: &'static str,
    /// Default skin protection, 0–100.
    pub skin_protection: f32,
    /// Two CSS colours for the style's swatch.
    pub swatch: [&'static str; 2],
}

pub(crate) struct Style {
    pub info: StyleInfo,
    pub texture: TextureParams,
    pub scene: ColorParams,
    pub skin: ColorParams,
    pub atmosphere: AtmosphereParams,
}

/// HSL band indices.
const RED: usize = 0;
const ORANGE: usize = 1;
const YELLOW: usize = 2;
const GREEN: usize = 3;
const AQUA: usize = 4;
const BLUE: usize = 5;
const PURPLE: usize = 6;
const MAGENTA: usize = 7;

/// Wheel indices.
const SHADOWS: usize = 0;
const MIDTONES: usize = 1;
const HIGHLIGHTS: usize = 2;

/// Slider units (−100…100) → normalised.
fn hsl(bands: &[(usize, [f32; 3])]) -> [[f32; 3]; 8] {
    let mut out = [[0.0; 3]; 8];
    for (i, v) in bands {
        out[*i] = v.map(|x| x / 100.0);
    }
    out
}

/// `(hue°, amount, luminance)` in slider units → normalised.
fn wheels(w: &[(usize, [f32; 3])]) -> [[f32; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, [h, a, l]) in w {
        out[*i] = [*h, a / 100.0, l / 100.0];
    }
    out
}

fn texture(clarity: f32, micro: f32, blemish: f32, specular: f32) -> TextureParams {
    TextureParams {
        clarity: clarity / 100.0,
        micro: micro / 100.0,
        blemish: blemish / 100.0,
        specular: specular / 100.0,
    }
}

pub(crate) fn all() -> Vec<Style> {
    vec![dark_melanin_glow(), silver_and_charcoal(), volumetric_golden_hour()]
}

pub(crate) fn find(id: &str) -> Option<Style> {
    all().into_iter().find(|s| s.info.id == id)
}

/// PRD "Dark Melanin Deep Tone": rich chocolate and bronze undertones, clean specular
/// highlights, neutral whites.
fn dark_melanin_glow() -> Style {
    Style {
        info: StyleInfo {
            id: "dark-melanin-glow",
            name: "Dark Melanin Glow",
            world: "High-Fashion & Melanin Precision",
            description: "Rich chocolate and bronze undertones with a clean, luminous glow. \
                Shine is balanced, not flattened; whites stay neutral.",
            skin_protection: 70.0,
            swatch: ["#3b2116", "#c98a4b"],
        },
        // Specular kept light: the PRD asks for clean skin highlights, not matte skin.
        texture: texture(10.0, 12.0, 25.0, 20.0),
        // Scene: deep, slightly warm shadows; neutral whites; muted competing colours.
        scene: ColorParams {
            hsl: hsl(&[
                (YELLOW, [0.0, -12.0, 0.0]),
                (GREEN, [0.0, -15.0, -5.0]),
                (AQUA, [0.0, -10.0, 0.0]),
                (BLUE, [0.0, -8.0, -5.0]),
            ]),
            wheels: wheels(&[
                (SHADOWS, [25.0, 14.0, -8.0]),
                (MIDTONES, [35.0, 6.0, 0.0]),
                // Cancels the warm midtone wheel where it would reach the whites.
                (HIGHLIGHTS, [215.0, 4.0, 2.0]),
            ]),
            vibrance: 0.1,
            contrast: 0.12,
            ..Default::default()
        },
        // Skin: bronze depth and a gentle lift so deep skin glows instead of greying.
        skin: ColorParams {
            hsl: hsl(&[
                (RED, [-3.0, 30.0, 10.0]),
                (ORANGE, [-4.0, 35.0, 14.0]),
                (YELLOW, [-8.0, 15.0, 8.0]),
            ]),
            wheels: wheels(&[(MIDTONES, [30.0, 14.0, 0.0])]),
            ..Default::default()
        },
        atmosphere: AtmosphereParams { glow: 0.12, warmth: 0.4, haze: 0.0 },
    }
}

/// PRD "Silver & Charcoal Monochromatic": deep blacks, crisp midtone micro-texture,
/// no muddy greys.
fn silver_and_charcoal() -> Style {
    Style {
        info: StyleInfo {
            id: "silver-charcoal",
            name: "Silver & Charcoal",
            world: "Character & High-Contrast Portraiture",
            description: "Black and white with deep blacks and crisp midtone texture. \
                Skin keeps its tonal separation instead of sinking into mud.",
            skin_protection: 60.0,
            swatch: ["#111214", "#d9dadc"],
        },
        texture: texture(25.0, 35.0, 15.0, 20.0),
        scene: ColorParams {
            // The black-and-white mixer: HSL luminance before desaturation.
            hsl: hsl(&[
                (RED, [0.0, 0.0, 8.0]),
                (ORANGE, [0.0, 0.0, 12.0]),
                (YELLOW, [0.0, 0.0, 6.0]),
                (GREEN, [0.0, 0.0, -12.0]),
                (AQUA, [0.0, 0.0, -15.0]),
                (BLUE, [0.0, 0.0, -30.0]),
                (PURPLE, [0.0, 0.0, -10.0]),
            ]),
            wheels: wheels(&[
                (SHADOWS, [220.0, 3.0, -14.0]),
                (HIGHLIGHTS, [40.0, 1.5, 4.0]),
            ]),
            contrast: 0.35,
            mono: 1.0,
            ..Default::default()
        },
        // Skin: a little extra light so faces separate from dark clothing.
        skin: ColorParams {
            hsl: hsl(&[(ORANGE, [0.0, 0.0, 6.0]), (RED, [0.0, 0.0, 4.0])]),
            ..Default::default()
        },
        atmosphere: AtmosphereParams::default(),
    }
}

/// PRD "Volumetric Golden Hour": warm low-angle light, soft bloom, hazy air.
fn volumetric_golden_hour() -> Style {
    Style {
        info: StyleInfo {
            id: "volumetric-golden-hour",
            name: "Volumetric Golden Hour",
            world: "Atmospheric & Environmental Landscapes",
            description: "Warm low-sun light with soft bloom around the brights and a hazy, \
                lifted atmosphere. Skin stays natural rather than orange.",
            skin_protection: 75.0,
            swatch: ["#5a3a1c", "#f2b45a"],
        },
        texture: texture(-10.0, 0.0, 0.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (ORANGE, [0.0, 10.0, 4.0]),
                (YELLOW, [-8.0, 15.0, 5.0]),
                (GREEN, [-15.0, -12.0, 0.0]),
                (AQUA, [0.0, -20.0, 0.0]),
                (BLUE, [0.0, -22.0, -6.0]),
                (MAGENTA, [0.0, -10.0, 0.0]),
            ]),
            wheels: wheels(&[
                (SHADOWS, [24.0, 14.0, 2.0]),
                (MIDTONES, [36.0, 28.0, 0.0]),
                (HIGHLIGHTS, [42.0, 50.0, 3.0]),
            ]),
            vibrance: 0.12,
            contrast: -0.04,
            ..Default::default()
        },
        // Skin: warm light falls on faces too, just gently.
        skin: ColorParams {
            wheels: wheels(&[(HIGHLIGHTS, [38.0, 6.0, 2.0])]),
            ..Default::default()
        },
        atmosphere: AtmosphereParams { glow: 0.45, warmth: 0.85, haze: 0.3 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_ids_are_unique_and_findable() {
        let styles = all();
        for s in &styles {
            assert!(find(s.info.id).is_some());
            assert_eq!(styles.iter().filter(|o| o.info.id == s.info.id).count(), 1);
            assert!((0.0..=100.0).contains(&s.info.skin_protection));
        }
        assert!(find("no-such-style").is_none());
    }
}
