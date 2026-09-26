//! Non-destructive sidecars. The original RAW/DNG is never rewritten.
//!
//! Canonical store is native JSON (`.epikos.json`). Adobe-compatible XMP (`.xmp`)
//! is a lossless projection of the same document.

mod document;
mod json;
mod xmp;

pub use document::{
    Adjustments, Atmosphere, BackgroundTint, ChromaticAberration, ColorGrade, ColorWheel, ColorWheels, Crop, Curves,
    DemosaicMode, DevelopDocument, DistortionCoeffs, Finishing, HslBands, HslChannel, LensCorrections,
    LocalAdjustment, LutRef, ManualAdjustment, ManualShape, BrushStroke, MaskTarget, NoiseReduction, SCurve, SourceRef, SplitToning, StyleRef, StyleWeight, Texture, Tone,
    ToneCurve, VirtualLight, WbMode, WhiteBalance,
};
pub use json::{load_json, save_json, sidecar_json_path};
pub use xmp::{load_xmp, save_xmp, sidecar_xmp_path};
