//! Scene-referred 32-bit develop pipeline.
//!
//! Order: demosaic → highlight recovery → white balance → noise reduction → lens
//! profile (distortion, lateral CA, vignetting) → manual chromatic aberration →
//! Brown–Conrady distortion → camera RGB → linear Rec.2020 → exposure → Step 2 tone →
//! orientation → straighten / vertical perspective → Step 3 local adjustments → Step 4
//! texture → Step 5 colour → Step 6 atmosphere (with the style layered into Steps 4–6)
//! → Step 7 curves and split toning → Step 8 finishing.
//! [`to_display_srgb`] is the separate view transform for the screen.

mod basic;
mod color_transform;
mod demosaic;
mod denoise;
mod develop;
mod display;
mod geometry;
mod highlights;
mod look;
mod matrix;
mod optics;
mod orient;
mod output;
mod preview;
mod white_balance;
mod xtrans;

pub use demosaic::{demosaic, DemosaicAlgorithm};
pub use denoise::reduce_noise;
pub use develop::{
    develop, develop_adjustments, develop_adjustments_with, develop_rgb, develop_rgb_with,
};
pub use display::{to_display_srgb, DisplayImage};
pub use highlights::recover_highlights;
pub use basic::{apply_tone, auto_tone};
pub use geometry::{apply_geometry, estimate_upright};
pub use look::{
    apply_look, bake_tone_curve, fit_to_image, look_is_active, look_masks, look_needs_depth, oklab_planes,
    skin_likelihood, styles, DepthPlane, LookInputs, MaskPlane, StyleInfo,
};
pub use optics::{apply_lens_profile, correct_chromatic_aberration, correct_distortion};
pub use orient::apply_orientation;
pub use output::{encode_rgb16, OutputSpace};
pub use preview::{base_block, bin_mosaic, block_for_size};
pub use white_balance::{gains_for_temperature, temperature_for_gains};
