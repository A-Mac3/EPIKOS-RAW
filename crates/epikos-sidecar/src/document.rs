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
}

impl Default for Adjustments {
    fn default() -> Self {
        Self {
            white_balance: WhiteBalance::default(),
            highlight_recovery: true,
            demosaic: DemosaicMode::Auto,
            lens: LensCorrections::default(),
            exposure: 0.0,
        }
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
