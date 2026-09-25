//! Scene-referred 32-bit develop pipeline.
//!
//! Order: demosaic → highlight recovery → white balance → noise reduction →
//! chromatic aberration
//! → Brown–Conrady distortion → camera RGB → linear Rec.2020 → exposure → orientation.
//! [`to_display_srgb`] is the separate view transform for the screen.

mod color_transform;
mod demosaic;
mod denoise;
mod develop;
mod display;
mod highlights;
mod matrix;
mod optics;
mod orient;
mod output;
mod preview;
mod white_balance;
mod xtrans;

pub use demosaic::{demosaic, DemosaicAlgorithm};
pub use denoise::reduce_noise;
pub use develop::{develop, develop_adjustments, develop_rgb};
pub use display::{to_display_srgb, DisplayImage};
pub use highlights::recover_highlights;
pub use optics::{correct_chromatic_aberration, correct_distortion};
pub use orient::apply_orientation;
pub use output::{encode_rgb16, OutputSpace};
pub use preview::{base_block, bin_mosaic, block_for_size};
pub use white_balance::{gains_for_temperature, temperature_for_gains};
