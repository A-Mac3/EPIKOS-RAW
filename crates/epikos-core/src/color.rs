use serde::{Deserialize, Serialize};

/// Scene-referred linear RGB working spaces used after sensor conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ColorSpace {
    /// Camera native RGB (after demosaic, before XYZ).
    CameraRgb,
    /// CIE XYZ D50-ish from the camera color matrix.
    Xyz,
    /// Linear Rec.2020, the default develop output.
    #[default]
    LinearRec2020,
}

impl ColorSpace {
    pub fn channels(&self) -> u8 {
        3
    }
}
