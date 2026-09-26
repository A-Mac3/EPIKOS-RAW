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
    /// Library tab: "Film Simulations", "Portraits & Skin", "Landscape & Nature",
    /// "Cinematic".
    pub category: &'static str,
    /// Shown in the library. Earlier styles stay available to the sidecars that use
    /// them, but aren't offered for new edits.
    pub listed: bool,
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
    pub film: Film,
}

/// A film stock's tone and texture, 0…1 each: `fade` lifts the blacks (matte), `roll`
/// softens the highlights, `grain` and `grain_size` add film grain.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Film {
    pub fade: f32,
    pub roll: f32,
    pub grain: f32,
    pub grain_size: f32,
}

impl Film {
    pub(crate) fn scaled(self, k: f32) -> Self {
        Self { fade: self.fade * k, roll: self.roll * k, grain: self.grain * k, grain_size: self.grain_size }
    }

    /// Strengths add; grain size is weighted by grain.
    pub(crate) fn plus(self, o: Self) -> Self {
        let g = self.grain + o.grain;
        Self {
            fade: self.fade + o.fade,
            roll: self.roll + o.roll,
            grain: g,
            grain_size: if g > 0.0 { (self.grain_size * self.grain + o.grain_size * o.grain) / g } else { 0.0 },
        }
    }
}

fn film(fade: f32, roll: f32, grain: f32, grain_size: f32) -> Film {
    Film { fade, roll, grain, grain_size }
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
        lines: 0.0,
    }
}

/// Every style: the library in display order (by category), then the earlier ones
/// kept for sidecars that use them.
pub(crate) fn all() -> Vec<Style> {
    vec![
        // Film Simulations
        portra_400(),
        fuji_pro_400h(),
        cinestill_800t(),
        leica_monochrom(),
        ilford_hp5(),
        // Portraits & Skin
        dark_melanin_glow(),
        soft_editorial(),
        amber_warmth(),
        high_key_beauty(),
        // Landscape & Nature
        moody_pacific(),
        golden_hour_flare(),
        deep_emerald(),
        vivid_alpine(),
        // Cinematic
        teal_and_orange(),
        bleach_bypass(),
        cyberpunk(),
        vintage_pastel(),
        // Earlier styles (not listed)
        silver_and_charcoal(),
        volumetric_golden_hour(),
        moody_and_earthy(),
        high_key_editorial(),
    ]
}

#[allow(clippy::too_many_arguments)]
fn info(
    id: &'static str,
    name: &'static str,
    category: &'static str,
    world: &'static str,
    description: &'static str,
    skin_protection: f32,
    swatch: [&'static str; 2],
) -> StyleInfo {
    StyleInfo { id, name, world, category, listed: true, description, skin_protection, swatch }
}

const FILM: &str = "Film Simulations";
const PORTRAIT: &str = "Portraits & Skin";
const LANDSCAPE: &str = "Landscape & Nature";
const CINEMATIC: &str = "Cinematic";

// ---- Film Simulations ------------------------------------------------------------------
// Emulations of the stocks' character (palette, contrast, fade, grain), not measured
// reproductions of their spectral response.

/// Kodak Portra 400: warm, forgiving skin, soft contrast, pastel greens and blues.
fn portra_400() -> Style {
    Style {
        info: info(
            "portra-400",
            "Portra 400",
            FILM,
            "Cinematic & Film Emulation",
            "Kodak's portrait stock: warm, forgiving skin, soft contrast, gently faded blacks \
                and pastel greens and blues.",
            40.0,
            ["#b99a7e", "#f0d5b8"],
        ),
        texture: texture(-5.0, 0.0, 10.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (RED, [0.0, -5.0, 0.0]),
                (ORANGE, [2.0, 5.0, 6.0]),
                (YELLOW, [-6.0, -5.0, 0.0]),
                (GREEN, [-8.0, -20.0, 0.0]),
                (AQUA, [0.0, -15.0, 0.0]),
                (BLUE, [-4.0, -15.0, 4.0]),
            ]),
            wheels: wheels(&[(SHADOWS, [200.0, 6.0, 3.0]), (HIGHLIGHTS, [40.0, 12.0, 0.0])]),
            contrast: -0.08,
            ..Default::default()
        },
        skin: ColorParams { hsl: hsl(&[(ORANGE, [-2.0, 6.0, 6.0])]), ..Default::default() },
        atmosphere: AtmosphereParams::default(),
        film: film(0.18, 0.25, 0.18, 0.3),
    }
}

/// Fujifilm Pro 400H: airy pastels, minty greens, cool-green shadows.
fn fuji_pro_400h() -> Style {
    Style {
        info: info(
            "fuji-pro-400h",
            "Fuji Pro 400H",
            FILM,
            "Cinematic & Film Emulation",
            "Fujifilm's wedding stock: airy pastels, minty greens, cool-green shadows and \
                soft, bright highlights.",
            45.0,
            ["#8fb3a3", "#eef2e6"],
        ),
        texture: texture(-5.0, 0.0, 10.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (RED, [0.0, -10.0, 0.0]),
                (ORANGE, [0.0, -4.0, 4.0]),
                (YELLOW, [10.0, -12.0, 4.0]),
                (GREEN, [10.0, -5.0, 6.0]),
                (AQUA, [0.0, 5.0, 4.0]),
                (BLUE, [-6.0, -10.0, 6.0]),
            ]),
            wheels: wheels(&[(SHADOWS, [170.0, 10.0, 4.0]), (HIGHLIGHTS, [60.0, 5.0, 3.0])]),
            saturation: -0.05,
            contrast: -0.1,
            ..Default::default()
        },
        skin: ColorParams::default(),
        atmosphere: AtmosphereParams::default(),
        film: film(0.2, 0.3, 0.15, 0.3),
    }
}

/// CineStill 800T: tungsten-balanced motion-picture stock; cool night colour, red
/// halation around lights, visible grain.
fn cinestill_800t() -> Style {
    Style {
        info: info(
            "cinestill-800t",
            "CineStill 800T",
            FILM,
            "Cinematic & Film Emulation",
            "Tungsten motion-picture stock: cool blue night colour, a red-orange halation glow \
                around lights and visible grain.",
            50.0,
            ["#1d2b4a", "#e0503a"],
        ),
        texture: texture(0.0, 5.0, 0.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (RED, [0.0, 10.0, 0.0]),
                (ORANGE, [4.0, 5.0, 0.0]),
                (AQUA, [-8.0, 10.0, 0.0]),
                (BLUE, [0.0, 10.0, -4.0]),
            ]),
            wheels: wheels(&[
                (SHADOWS, [205.0, 18.0, -2.0]),
                (MIDTONES, [200.0, 10.0, 0.0]),
                (HIGHLIGHTS, [25.0, 10.0, 0.0]),
            ]),
            contrast: 0.1,
            ..Default::default()
        },
        skin: ColorParams::default(),
        // Halation: a tight, warm bloom around bright lights.
        atmosphere: AtmosphereParams { glow: 0.3, glow_radius: 0.01, glow_warmth: 1.0, ..Default::default() },
        film: film(0.08, 0.1, 0.3, 0.45),
    }
}

/// Leica Monochrom: a dedicated black-and-white sensor's smooth, rich tonality.
fn leica_monochrom() -> Style {
    Style {
        info: info(
            "leica-monochrom",
            "Leica Monochrom",
            FILM,
            "Character & High-Contrast Portraiture",
            "Black and white with the smooth, deep tonality of a monochrome sensor: rich \
                midtones, fine texture, skin lifted from dark clothing.",
            60.0,
            ["#1a1a1a", "#e8e8e8"],
        ),
        texture: texture(10.0, 18.0, 10.0, 15.0),
        scene: ColorParams {
            hsl: hsl(&[
                (RED, [0.0, 0.0, 10.0]),
                (ORANGE, [0.0, 0.0, 14.0]),
                (YELLOW, [0.0, 0.0, 8.0]),
                (GREEN, [0.0, 0.0, -6.0]),
                (AQUA, [0.0, 0.0, -8.0]),
                (BLUE, [0.0, 0.0, -18.0]),
            ]),
            contrast: 0.18,
            mono: 1.0,
            ..Default::default()
        },
        skin: ColorParams { hsl: hsl(&[(ORANGE, [0.0, 0.0, 5.0])]), ..Default::default() },
        atmosphere: AtmosphereParams::default(),
        film: film(0.04, 0.05, 0.06, 0.2),
    }
}

/// Ilford HP5 Plus: gritty, contrasty black-and-white press film.
fn ilford_hp5() -> Style {
    Style {
        info: info(
            "ilford-hp5",
            "Ilford HP5",
            FILM,
            "Character & High-Contrast Portraiture",
            "Classic black-and-white press film: punchy contrast, dark skies and a gritty, \
                visible grain.",
            55.0,
            ["#0e0e0e", "#cfcfcf"],
        ),
        texture: texture(20.0, 25.0, 0.0, 10.0),
        scene: ColorParams {
            hsl: hsl(&[
                (RED, [0.0, 0.0, 4.0]),
                (ORANGE, [0.0, 0.0, 6.0]),
                (GREEN, [0.0, 0.0, -10.0]),
                (BLUE, [0.0, 0.0, -25.0]),
            ]),
            contrast: 0.32,
            mono: 1.0,
            ..Default::default()
        },
        skin: ColorParams::default(),
        atmosphere: AtmosphereParams::default(),
        film: film(0.06, 0.0, 0.45, 0.55),
    }
}

// ---- Portraits & Skin ------------------------------------------------------------------

/// Soft Editorial: clean, bright and gentle; soft skin, quiet background colour.
fn soft_editorial() -> Style {
    Style {
        info: info(
            "soft-editorial",
            "Soft Editorial",
            PORTRAIT,
            "High-Fashion & Melanin Precision",
            "Magazine-clean and gentle: soft skin, lifted shadows, quiet background colour and \
                low contrast.",
            70.0,
            ["#c7b6ae", "#f6ede8"],
        ),
        texture: texture(-12.0, 0.0, 30.0, 20.0),
        scene: ColorParams {
            hsl: hsl(&[(GREEN, [0.0, -25.0, 0.0]), (BLUE, [0.0, -15.0, 5.0])]),
            wheels: wheels(&[(SHADOWS, [260.0, 4.0, 6.0]), (HIGHLIGHTS, [30.0, 4.0, 4.0])]),
            saturation: -0.1,
            contrast: -0.1,
            ..Default::default()
        },
        skin: ColorParams { hsl: hsl(&[(ORANGE, [0.0, 5.0, 8.0])]), ..Default::default() },
        atmosphere: AtmosphereParams::default(),
        film: film(0.1, 0.1, 0.0, 0.0),
    }
}

/// Amber Warmth: golden-amber light on skin and highlights.
fn amber_warmth() -> Style {
    Style {
        info: info(
            "amber-warmth",
            "Amber Warmth",
            PORTRAIT,
            "High-Fashion & Melanin Precision",
            "Golden-amber light: warm, glowing skin and honeyed highlights against cooled-down \
                greens and blues.",
            55.0,
            ["#6b3a18", "#f2a54a"],
        ),
        texture: texture(0.0, 5.0, 15.0, 10.0),
        scene: ColorParams {
            hsl: hsl(&[
                (ORANGE, [0.0, 10.0, 4.0]),
                (YELLOW, [-6.0, 10.0, 0.0]),
                (GREEN, [-10.0, -15.0, 0.0]),
                (BLUE, [0.0, -20.0, -4.0]),
            ]),
            wheels: wheels(&[
                (SHADOWS, [30.0, 8.0, 0.0]),
                (MIDTONES, [38.0, 14.0, 0.0]),
                (HIGHLIGHTS, [45.0, 18.0, 2.0]),
            ]),
            vibrance: 0.08,
            ..Default::default()
        },
        skin: ColorParams {
            hsl: hsl(&[(RED, [-2.0, 8.0, 4.0]), (ORANGE, [-3.0, 12.0, 8.0])]),
            ..Default::default()
        },
        atmosphere: AtmosphereParams { glow: 0.15, glow_radius: 0.02, glow_warmth: 0.8, ..Default::default() },
        film: Film::default(),
    }
}

/// High-Key Beauty: bright, luminous, flawless-looking skin.
fn high_key_beauty() -> Style {
    let mut s = high_key_editorial();
    s.info = info(
        "high-key-beauty",
        "High-Key Beauty",
        PORTRAIT,
        "High-Fashion & Melanin Precision",
        "Bright beauty light: luminous, evenly smoothed skin, pastel shadows and gentle contrast.",
        60.0,
        ["#e3d6dc", "#fbf5f1"],
    );
    s.texture = texture(-12.0, 0.0, 45.0, 35.0);
    s
}

// ---- Landscape & Nature ----------------------------------------------------------------

/// Moody Pacific: cold sea and sky, deep slate shadows, a cool mist.
fn moody_pacific() -> Style {
    Style {
        info: info(
            "moody-pacific",
            "Moody Pacific",
            LANDSCAPE,
            "Atmospheric & Environmental Landscapes",
            "Cold coast light: deep slate blues and teals, muted greens, dark shadows and a \
                cool sea mist.",
            70.0,
            ["#1f2f3a", "#8aa1ab"],
        ),
        texture: texture(15.0, 10.0, 0.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (ORANGE, [0.0, -10.0, 0.0]),
                (YELLOW, [0.0, -25.0, 0.0]),
                (GREEN, [15.0, -30.0, -10.0]),
                (AQUA, [-10.0, -5.0, -8.0]),
                (BLUE, [-6.0, -10.0, -12.0]),
            ]),
            wheels: wheels(&[(SHADOWS, [210.0, 16.0, -6.0]), (HIGHLIGHTS, [200.0, 5.0, 0.0])]),
            saturation: -0.1,
            contrast: 0.15,
            ..Default::default()
        },
        skin: ColorParams::default(),
        atmosphere: AtmosphereParams { haze: 0.1, haze_warmth: -0.6, ..Default::default() },
        film: film(0.06, 0.0, 0.0, 0.0),
    }
}

/// Golden Hour Flare: low sun, strong warm bloom and rays.
fn golden_hour_flare() -> Style {
    let mut s = volumetric_golden_hour();
    s.info = info(
        "golden-hour-flare",
        "Golden Hour Flare",
        LANDSCAPE,
        "Atmospheric & Environmental Landscapes",
        "Low sun straight into the lens: warm light, a strong golden bloom and rays through \
            the bright sky.",
        75.0,
        ["#5a3a1c", "#ffc15e"],
    );
    s.atmosphere.glow = 0.55;
    s.atmosphere.shafts = 0.3;
    s.film = film(0.05, 0.15, 0.0, 0.0);
    s
}

/// Deep Emerald: rich, dark forest greens.
fn deep_emerald() -> Style {
    Style {
        info: info(
            "deep-emerald",
            "Deep Emerald",
            LANDSCAPE,
            "Atmospheric & Environmental Landscapes",
            "Rich, dark forest greens and teals with deep shadows and crisp leaf texture.",
            70.0,
            ["#0d2a1c", "#3f8a5a"],
        ),
        texture: texture(15.0, 15.0, 0.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (ORANGE, [0.0, -5.0, 0.0]),
                (YELLOW, [12.0, 5.0, -8.0]),
                (GREEN, [8.0, 25.0, -18.0]),
                (AQUA, [-10.0, 15.0, -10.0]),
                (BLUE, [0.0, -10.0, -8.0]),
            ]),
            wheels: wheels(&[(SHADOWS, [160.0, 10.0, -6.0])]),
            vibrance: 0.1,
            contrast: 0.15,
            ..Default::default()
        },
        skin: ColorParams::default(),
        atmosphere: AtmosphereParams::default(),
        film: Film::default(),
    }
}

/// Vivid Alpine: crisp mountain air; deep blue sky, clear greens.
fn vivid_alpine() -> Style {
    Style {
        info: info(
            "vivid-alpine",
            "Vivid Alpine",
            LANDSCAPE,
            "Atmospheric & Environmental Landscapes",
            "Crisp mountain air: deep blue skies, clear greens, bright snow and strong detail.",
            75.0,
            ["#1a5fa8", "#e8f4ff"],
        ),
        texture: texture(25.0, 20.0, 0.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (YELLOW, [0.0, 8.0, 4.0]),
                (GREEN, [-6.0, 15.0, 4.0]),
                (AQUA, [0.0, 20.0, 0.0]),
                (BLUE, [-4.0, 25.0, -8.0]),
            ]),
            vibrance: 0.25,
            contrast: 0.12,
            ..Default::default()
        },
        skin: ColorParams::default(),
        atmosphere: AtmosphereParams::default(),
        film: Film::default(),
    }
}

// ---- Cinematic -------------------------------------------------------------------------

/// Bleach Bypass: skipping the bleach bath leaves silver in the print; desaturated,
/// contrasty, metallic.
fn bleach_bypass() -> Style {
    Style {
        info: info(
            "bleach-bypass",
            "Bleach Bypass",
            CINEMATIC,
            "Cinematic & Film Emulation",
            "The skipped-bleach film process: muted, silvery colour with hard contrast and gritty \
                detail.",
            30.0,
            ["#2b2e30", "#b8b4a8"],
        ),
        texture: texture(20.0, 15.0, 0.0, 0.0),
        scene: ColorParams {
            wheels: wheels(&[(SHADOWS, [210.0, 5.0, -6.0]), (HIGHLIGHTS, [40.0, 4.0, 2.0])]),
            saturation: -0.5,
            contrast: 0.35,
            ..Default::default()
        },
        skin: ColorParams::default(),
        atmosphere: AtmosphereParams::default(),
        film: film(0.0, 0.0, 0.12, 0.35),
    }
}

/// Cyberpunk: neon magenta and cyan, purple shadows.
fn cyberpunk() -> Style {
    Style {
        info: info(
            "cyberpunk",
            "Cyberpunk",
            CINEMATIC,
            "Cinematic & Film Emulation",
            "Neon night city: purple shadows, magenta and cyan light, glowing highlights.",
            45.0,
            ["#2a0f4a", "#ff3cac"],
        ),
        texture: texture(10.0, 10.0, 0.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (YELLOW, [-10.0, -20.0, 0.0]),
                (GREEN, [30.0, -30.0, 0.0]),
                (AQUA, [0.0, 25.0, 5.0]),
                (BLUE, [15.0, 20.0, 0.0]),
                (PURPLE, [0.0, 25.0, 0.0]),
                (MAGENTA, [0.0, 25.0, 5.0]),
            ]),
            wheels: wheels(&[
                (SHADOWS, [260.0, 25.0, -4.0]),
                (MIDTONES, [300.0, 10.0, 0.0]),
                (HIGHLIGHTS, [185.0, 18.0, 2.0]),
            ]),
            vibrance: 0.2,
            contrast: 0.2,
            ..Default::default()
        },
        skin: ColorParams::default(),
        atmosphere: AtmosphereParams { glow: 0.25, glow_radius: 0.012, glow_warmth: -0.5, ..Default::default() },
        film: Film::default(),
    }
}

/// Vintage Pastel: faded seventies print; pink highlights, green-cyan shadows.
fn vintage_pastel() -> Style {
    Style {
        info: info(
            "vintage-pastel",
            "Vintage Pastel",
            CINEMATIC,
            "Cinematic & Film Emulation",
            "A faded 1970s print: soft pastels, pink highlights, green-cyan shadows and gentle \
                grain.",
            45.0,
            ["#a8c9c2", "#f5d6d9"],
        ),
        texture: texture(-5.0, 0.0, 0.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (RED, [5.0, -15.0, 6.0]),
                (GREEN, [10.0, -20.0, 6.0]),
                (BLUE, [-8.0, -20.0, 8.0]),
            ]),
            wheels: wheels(&[(SHADOWS, [170.0, 10.0, 6.0]), (HIGHLIGHTS, [340.0, 8.0, 3.0])]),
            saturation: -0.2,
            contrast: -0.18,
            ..Default::default()
        },
        skin: ColorParams::default(),
        atmosphere: AtmosphereParams::default(),
        film: film(0.25, 0.3, 0.12, 0.3),
    }
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
            category: "Portraits & Skin",
            listed: true,
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
        atmosphere: AtmosphereParams {
            glow: 0.12,
            glow_radius: 0.015,
            glow_warmth: 0.4,
            ..Default::default()
        },
        film: Film::default(),
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
            category: "Legacy",
            listed: false,
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
        film: Film::default(),
    }
}

/// PRD "Volumetric Golden Hour": warm low-angle light, soft bloom, hazy air.
fn volumetric_golden_hour() -> Style {
    Style {
        info: StyleInfo {
            id: "volumetric-golden-hour",
            name: "Volumetric Golden Hour",
            world: "Atmospheric & Environmental Landscapes",
            category: "Legacy",
            listed: false,
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
        // Bloom, a warm veil and soft rays from the brightest part of the sky. No depth
        // fog: gold airlight over a blue sky reads mauve, not golden.
        atmosphere: AtmosphereParams {
            glow: 0.45,
            glow_radius: 0.02,
            glow_warmth: 0.85,
            shafts: 0.2,
            shaft_length: 0.6,
            shaft_warmth: 0.85,
            light: None,
            haze: 0.2,
            haze_warmth: 0.85,
            ..Default::default()
        },
        film: Film::default(),
    }
}

/// PRD "Hollywood Blockbuster: Teal & Orange": teal shadows, warm highlights, skin
/// kept warm against the cool background.
fn teal_and_orange() -> Style {
    Style {
        info: StyleInfo {
            id: "teal-orange",
            name: "Teal & Orange",
            world: "Cinematic & Film Emulation",
            category: "Cinematic",
            listed: true,
            description: "Blockbuster grade: teal shadows and backgrounds against warm highlights. \
                Skin stays warm and natural.",
            skin_protection: 80.0,
            swatch: ["#0f4c55", "#e08a3c"],
        },
        texture: texture(10.0, 5.0, 0.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (ORANGE, [0.0, 10.0, 0.0]),
                (YELLOW, [-10.0, -5.0, 0.0]),
                (GREEN, [20.0, -25.0, -5.0]),
                (AQUA, [0.0, 10.0, 0.0]),
                (BLUE, [-10.0, 5.0, -5.0]),
            ]),
            wheels: wheels(&[
                (SHADOWS, [188.0, 30.0, -5.0]),
                (MIDTONES, [190.0, 8.0, 0.0]),
                (HIGHLIGHTS, [35.0, 25.0, 2.0]),
            ]),
            contrast: 0.15,
            ..Default::default()
        },
        skin: ColorParams {
            hsl: hsl(&[(ORANGE, [0.0, 8.0, 3.0])]),
            ..Default::default()
        },
        atmosphere: AtmosphereParams::default(),
        film: Film::default(),
    }
}

/// PRD "Moody & Earthy": muted foliage greens, deep slate blues, warm skin accents,
/// rich earth-tone shadows.
fn moody_and_earthy() -> Style {
    Style {
        info: StyleInfo {
            id: "moody-earthy",
            name: "Moody & Earthy",
            world: "Atmospheric & Environmental Landscapes",
            category: "Legacy",
            listed: false,
            description: "Muted foliage, deep slate blues and rich earth-tone shadows, with warm \
                accents on skin.",
            skin_protection: 70.0,
            swatch: ["#2d2a22", "#6b7a5e"],
        },
        texture: texture(15.0, 10.0, 0.0, 0.0),
        scene: ColorParams {
            hsl: hsl(&[
                (ORANGE, [0.0, 5.0, 0.0]),
                (YELLOW, [-5.0, -20.0, -5.0]),
                (GREEN, [-20.0, -35.0, -15.0]),
                (AQUA, [0.0, -30.0, -10.0]),
                (BLUE, [-8.0, -30.0, -20.0]),
            ]),
            wheels: wheels(&[
                (SHADOWS, [30.0, 12.0, -10.0]),
                (HIGHLIGHTS, [45.0, 6.0, -2.0]),
            ]),
            saturation: -0.1,
            contrast: 0.1,
            ..Default::default()
        },
        skin: ColorParams {
            hsl: hsl(&[(ORANGE, [0.0, 10.0, 4.0]), (RED, [0.0, 5.0, 2.0])]),
            ..Default::default()
        },
        atmosphere: AtmosphereParams::default(),
        film: Film::default(),
    }
}

/// PRD "High-Key Editorial": luminous skin, pastel shadows, clean isolation.
fn high_key_editorial() -> Style {
    Style {
        info: StyleInfo {
            id: "high-key-editorial",
            name: "High-Key Editorial",
            world: "High-Fashion & Melanin Precision",
            category: "Legacy",
            listed: false,
            description: "Bright and clean: luminous skin, soft pastel shadows, gentle contrast.",
            skin_protection: 60.0,
            swatch: ["#d9d2e6", "#f7efe9"],
        },
        texture: texture(-10.0, 0.0, 35.0, 30.0),
        scene: ColorParams {
            wheels: wheels(&[
                (SHADOWS, [260.0, 8.0, 20.0]),
                (MIDTONES, [30.0, 2.0, 8.0]),
                (HIGHLIGHTS, [30.0, 3.0, 6.0]),
            ]),
            saturation: -0.08,
            vibrance: 0.05,
            contrast: -0.15,
            ..Default::default()
        },
        skin: ColorParams {
            hsl: hsl(&[(ORANGE, [0.0, -5.0, 10.0]), (RED, [0.0, -3.0, 6.0])]),
            ..Default::default()
        },
        atmosphere: AtmosphereParams {
            glow: 0.15,
            glow_radius: 0.02,
            glow_warmth: 0.1,
            ..Default::default()
        },
        film: Film::default(),
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

    #[test]
    fn the_library_has_four_categories_of_listed_styles() {
        let listed: Vec<_> = all().into_iter().filter(|s| s.info.listed).collect();
        assert_eq!(listed.len(), 17);
        for (cat, n) in [(FILM, 5), (PORTRAIT, 4), (LANDSCAPE, 4), (CINEMATIC, 4)] {
            assert_eq!(listed.iter().filter(|s| s.info.category == cat).count(), n, "{cat}");
        }
        // Earlier styles still resolve for the sidecars that name them.
        for id in ["silver-charcoal", "volumetric-golden-hour", "moody-earthy", "high-key-editorial"] {
            assert!(find(id).is_some_and(|s| !s.info.listed));
        }
    }
}
