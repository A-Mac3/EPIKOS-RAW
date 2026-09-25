use serde::{Deserialize, Serialize};

use crate::format::CameraFormat;

/// Color-filter array geometry (Bayer 2×2 or X-Trans 6×6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfaPattern {
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// 0 = R, 1 = G, 2 = B, 3 = extra (emerald / white).
    pub pattern: Vec<u8>,
}

impl CfaPattern {
    pub fn rggb() -> Self {
        Self {
            name: "RGGB".to_string(),
            width: 2,
            height: 2,
            pattern: vec![0, 1, 1, 2],
        }
    }

    pub fn color_at(&self, row: usize, col: usize) -> u8 {
        if self.width == 0 || self.height == 0 || self.pattern.is_empty() {
            return 1;
        }
        let y = row % self.height;
        let x = col % self.width;
        self.pattern[y * self.width + x]
    }

    pub fn is_xtrans(&self) -> bool {
        self.width == 6 && self.height == 6
    }

    pub fn is_bayer(&self) -> bool {
        self.width == 2 && self.height == 2
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SensorLayout {
    Cfa { cfa: CfaPattern },
    LinearRgb,
    Monochrome,
}

/// Camera/sensor identity used for color science and demosaic routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorProfile {
    pub format: CameraFormat,
    pub make: String,
    pub model: String,
    pub clean_make: String,
    pub clean_model: String,
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: u32,
    pub samples_per_pixel: u32,
    pub layout: SensorLayout,
    pub as_shot_wb: [f32; 4],
    /// CIE XYZ (D65) → camera RGB: the DNG `ColorMatrix` convention, not normalised.
    /// All zeros means the file carries no matrix.
    pub xyz_to_cam: [[f32; 3]; 3],
}

impl SensorProfile {
    pub fn is_fuji_xtrans(&self) -> bool {
        matches!(&self.layout, SensorLayout::Cfa { cfa } if cfa.is_xtrans())
    }
}
