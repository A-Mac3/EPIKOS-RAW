//! High-bit-depth (32-bit float per channel) primitives for the EPIKOS RAW engine.

mod buffer;
mod color;
mod error;
mod format;
mod metadata;
mod profile;

pub use buffer::{ImageRgbF32, MosaicF32, Pixel};
pub use color::ColorSpace;
pub use error::{Error, Result};
pub use format::CameraFormat;
pub use metadata::{CaptureMetadata, GpsInfo, Ratio, SRatio};
pub use profile::{CfaPattern, Orientation, SensorLayout, SensorProfile};
