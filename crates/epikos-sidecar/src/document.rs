use serde::{Deserialize, Serialize};

/// Canonical develop document. Round-trips 1:1 through JSON and XMP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopDocument {
    pub version: u32,
    pub source: SourceRef,
    pub adjustments: Adjustments,
}

impl DevelopDocument {
    pub const VERSION: u32 = 1;

    pub fn new(source: SourceRef) -> Self {
        Self {
            version: Self::VERSION,
            source,
            adjustments: Adjustments::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub path: String,
    pub sha256: String,
    pub format: String,
    pub make: String,
    pub model: String,
}

/// Missing fields take their default, so sidecars written by older versions still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Adjustments {
    pub white_balance: WhiteBalance,
    /// Reconstruct clipped CFA channels from unclipped neighbours.
    pub highlight_recovery: bool,
    pub demosaic: DemosaicMode,
    pub lens: LensCorrections,
    /// Global exposure in stops (EV), applied in scene-referred linear light.
    pub exposure: f32,
    pub noise_reduction: NoiseReduction,
    /// Step 4: micro-texture and skin retouching.
    pub texture: Texture,
    /// Step 5: HSL and three-way colour grading.
    pub color: ColorGrade,
    /// Step 6: glow, depth-based fog and light shafts.
    pub atmosphere: Atmosphere,
    /// Parametric style layered on top of the manual Step 4–6 settings.
    pub style: StyleRef,
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            white_balance: WhiteBalance::default(),
            highlight_recovery: true,
            demosaic: DemosaicMode::Auto,
            lens: LensCorrections::default(),
            exposure: 0.0,
            noise_reduction: NoiseReduction::default(),
            texture: Texture::default(),
            color: ColorGrade::default(),
            atmosphere: Atmosphere::default(),
            style: StyleRef::default(),
        }
    }
}

/// Sensor noise reduction strengths, 0–100. Like Camera Raw, colour noise reduction
/// is on by default for RAW files and luminance smoothing is off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NoiseReduction {
    pub luminance: f32,
    pub color: f32,
}

impl Default for NoiseReduction {
    fn default() -> Self {
        Self {
            luminance: 0.0,
            color: 25.0,
        }
    }
}

/// PRD Step 4. Clarity and micro-texture run −100…100 (negative softens); the skin
/// retouching controls run 0…100 and only act where skin is detected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Texture {
    /// Mid-scale local contrast in the midtones.
    pub clarity: f32,
    /// Fine structure (hair, fabric, bark), cored against noise and eased off on skin.
    pub micro_texture: f32,
    /// Evens out spot-sized blemishes on skin while keeping pores and facial lines.
    pub blemish_smoothing: f32,
    /// Tames shiny hot spots on skin and restores the skin colour under them.
    pub specular_balance: f32,
}

impl Texture {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }
}

/// One HSL band: each value −100…100.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct HslChannel {
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
}

/// PRD Step 5 HSL: eight hue bands, as in Lightroom's HSL panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct HslBands {
    pub red: HslChannel,
    pub orange: HslChannel,
    pub yellow: HslChannel,
    pub green: HslChannel,
    pub aqua: HslChannel,
    pub blue: HslChannel,
    pub purple: HslChannel,
    pub magenta: HslChannel,
}

impl HslBands {
    /// Band names in hue order, matching [`HslBands::bands`].
    pub const NAMES: [&'static str; 8] = [
        "red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta",
    ];

    pub fn bands(&self) -> [&HslChannel; 8] {
        [
            &self.red,
            &self.orange,
            &self.yellow,
            &self.green,
            &self.aqua,
            &self.blue,
            &self.purple,
            &self.magenta,
        ]
    }

    pub fn bands_mut(&mut self) -> [&mut HslChannel; 8] {
        [
            &mut self.red,
            &mut self.orange,
            &mut self.yellow,
            &mut self.green,
            &mut self.aqua,
            &mut self.blue,
            &mut self.purple,
            &mut self.magenta,
        ]
    }
}

/// One colour wheel: a tint (hue in degrees on the usual HSV wheel, amount 0…100)
/// and a luminance offset (−100…100) for one tonal range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorWheel {
    pub hue: f32,
    pub amount: f32,
    pub luminance: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorWheels {
    pub shadows: ColorWheel,
    pub midtones: ColorWheel,
    pub highlights: ColorWheel,
}

/// PRD Step 5: base colour grading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorGrade {
    pub hsl: HslBands,
    pub wheels: ColorWheels,
    /// 0…100: how much of the HSL and wheel changes skin is shielded from.
    pub skin_protection: f32,
}

/// PRD Step 6: atmospheric light. Strengths 0…100, warmth −100 (cool) … 100 (gold).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Atmosphere {
    /// Bloom around bright areas.
    pub glow: f32,
    /// Bloom radius, 0…100 (≈0.5–5% of the frame).
    pub glow_size: f32,
    pub glow_warmth: f32,
    /// Aerial haze that thickens with distance (from the depth model; uniform without it).
    pub fog: f32,
    /// Depth at which fog begins, 0 (at the camera) … 100 (only the far background).
    pub fog_start: f32,
    pub fog_warmth: f32,
    /// Volumetric rays streaming from a light source through the bright areas.
    pub shafts: f32,
    /// Ray length, 0…100 (fraction of the way to the light).
    pub shaft_length: f32,
    pub shaft_warmth: f32,
    /// Find the light source automatically (the brightest part of the sky).
    pub shaft_auto: bool,
    /// Light position when not automatic, 0–1 across and down the upright frame.
    pub shaft_x: f32,
    pub shaft_y: f32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            glow: 0.0,
            glow_size: 50.0,
            glow_warmth: 0.0,
            fog: 0.0,
            fog_start: 30.0,
            fog_warmth: 0.0,
            shafts: 0.0,
            shaft_length: 60.0,
            shaft_warmth: 40.0,
            shaft_auto: true,
            shaft_x: 0.5,
            shaft_y: 0.15,
        }
    }
}

/// A parametric style from the style engine, applied on top of the manual settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StyleRef {
    /// Style id (e.g. `"dark-melanin-glow"`); empty for none.
    pub id: String,
    /// 0…100: scales every parameter of the style.
    pub amount: f32,
    /// 0…100: how much of the style's scene grade skin is shielded from. Seeded from
    /// the style's own default when it is applied.
    pub skin_protection: f32,
}

impl Default for StyleRef {
    fn default() -> Self {
        Self {
            id: String::new(),
            amount: 100.0,
            skin_protection: 0.0,
        }
    }
}

impl StyleRef {
    pub fn is_none(&self) -> bool {
        self.id.is_empty() || self.amount <= 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DemosaicMode {
    #[default]
    Auto,
    Malvar,
    Bilinear,
    Xtrans,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhiteBalance {
    pub mode: WbMode,
    pub temperature: f32,
    pub tint: f32,
}

impl Default for WhiteBalance {
    fn default() -> Self {
        Self {
            mode: WbMode::AsShot,
            temperature: 6500.0,
            tint: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WbMode {
    #[default]
    AsShot,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LensCorrections {
    pub distortion: DistortionCoeffs,
    pub chromatic_aberration: ChromaticAberration,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DistortionCoeffs {
    pub enabled: bool,
    pub k1: f32,
    pub k2: f32,
    pub k3: f32,
    pub p1: f32,
    pub p2: f32,
    pub cx: f32,
    pub cy: f32,
}

impl Default for DistortionCoeffs {
    fn default() -> Self {
        Self {
            enabled: false,
            k1: 0.0,
            k2: 0.0,
            k3: 0.0,
            p1: 0.0,
            p2: 0.0,
            cx: 0.0,
            cy: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChromaticAberration {
    pub enabled: bool,
    /// Radial scale offset for red vs green (typical ±0.02).
    pub red: f32,
    /// Radial scale offset for blue vs green.
    pub blue: f32,
}

impl Default for ChromaticAberration {
    fn default() -> Self {
        Self {
            enabled: false,
            red: 0.0,
            blue: 0.0,
        }
    }
}
