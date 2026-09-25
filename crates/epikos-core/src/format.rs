use serde::{Deserialize, Serialize};

/// First-class camera container formats the decoder advertises support for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CameraFormat {
    SonyArw,
    CanonCr3,
    CanonCr2,
    NikonNef,
    FujifilmRaf,
    LeicaDng,
    AppleProRaw,
    AdobeDng,
    /// Already-rendered images: developed like a linear-RGB DNG from their sRGB values.
    Jpeg,
    Png,
    Unknown,
}

impl CameraFormat {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "arw" | "srf" | "sr2" => Self::SonyArw,
            "cr3" => Self::CanonCr3,
            "cr2" | "crw" => Self::CanonCr2,
            "nef" | "nrw" => Self::NikonNef,
            "raf" => Self::FujifilmRaf,
            "dng" => Self::AdobeDng,
            "jpg" | "jpeg" => Self::Jpeg,
            "png" => Self::Png,
            _ => Self::Unknown,
        }
    }

    /// Refine a DNG guess using maker strings (Leica DNG, Apple ProRAW).
    pub fn refine_dng(self, make: &str, model: &str) -> Self {
        if self != Self::AdobeDng && self != Self::Unknown {
            return self;
        }
        let make_l = make.to_ascii_lowercase();
        let model_l = model.to_ascii_lowercase();
        if make_l.contains("apple") || model_l.contains("iphone") || model_l.contains("proraw") {
            return Self::AppleProRaw;
        }
        if make_l.contains("leica") {
            return Self::LeicaDng;
        }
        Self::AdobeDng
    }

    /// JPEG or PNG: 8-bit display-referred pixels, not sensor data.
    pub fn is_bitmap(self) -> bool {
        matches!(self, Self::Jpeg | Self::Png)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SonyArw => "Sony ARW",
            Self::CanonCr3 => "Canon CR3",
            Self::CanonCr2 => "Canon CR2",
            Self::NikonNef => "Nikon NEF",
            Self::FujifilmRaf => "Fujifilm RAF",
            Self::LeicaDng => "Leica DNG",
            Self::AppleProRaw => "Apple ProRAW",
            Self::AdobeDng => "Adobe DNG",
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Unknown => "Unknown",
        }
    }
}
