//! Lens profiles: distortion, lateral chromatic aberration and vignetting corrections
//! for one capture, from the camera (DNG `OpcodeList3`) or the Lensfun database.
//!
//! Every model maps a position in the *corrected* image to the position to sample in
//! the captured one, so a correction is a single resampling pass.

use serde::Serialize;

/// Corrections for one lens at the capture's focal length and aperture.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LensProfile {
    /// Where the profile came from, for the UI ("Camera (DNG)", "Lensfun: …").
    pub source: String,
    pub geometry: Option<LensGeometry>,
    pub vignetting: Option<LensVignetting>,
}

impl LensProfile {
    pub fn is_empty(&self) -> bool {
        self.geometry.is_none() && self.vignetting.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LensGeometry {
    /// DNG `WarpRectilinear`: per plane (R, G, B; one plane applies to all)
    /// `[kr0, kr1, kr2, kr3, kt0, kt1]`, optical centre as a fraction of the frame, and
    /// radius 1 at the corner farthest from the centre.
    Dng {
        center: [f64; 2],
        planes: Vec<[f64; 6]>,
    },
    /// Lensfun (Hugin coordinates: radius 1 is half the short side of the calibration
    /// frame). `scale` converts "radius in half short sides of this image" to Hugin
    /// radius (differing crop factors); `calib_aspect` is the calibration frame's long /
    /// short ratio.
    Hugin {
        scale: f64,
        calib_aspect: f64,
        distortion: Option<Radial>,
        /// Red and blue relative to green.
        tca: Option<[Radial; 2]>,
    },
}

/// Radial models from Lensfun: each gives the captured radius for a corrected one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "model")]
pub enum Radial {
    /// `r_d = r_u (1 − k1 + k1 r_u²)`
    Poly3 { k1: f64 },
    /// `r_d = r_u (1 + k1 r_u² + k2 r_u⁴)`
    Poly5 { k1: f64, k2: f64 },
    /// `r_d = r_u (a r_u³ + b r_u² + c r_u + 1 − a − b − c)`
    PtLens { a: f64, b: f64, c: f64 },
    /// TCA: `r_d = k r_u`
    Linear { k: f64 },
    /// TCA: `r_d = r_u (b r_u² + c r_u + v)`
    TcaPoly3 { b: f64, c: f64, v: f64 },
}

impl Radial {
    /// Captured radius ÷ corrected radius at corrected radius `r`.
    pub fn factor(self, r: f64) -> f64 {
        match self {
            Radial::Poly3 { k1 } => 1.0 - k1 + k1 * r * r,
            Radial::Poly5 { k1, k2 } => {
                let r2 = r * r;
                1.0 + k1 * r2 + k2 * r2 * r2
            }
            Radial::PtLens { a, b, c } => a * r * r * r + b * r * r + c * r + 1.0 - a - b - c,
            Radial::Linear { k } => k,
            Radial::TcaPoly3 { b, c, v } => b * r * r + c * r + v,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LensVignetting {
    /// DNG `FixVignetteRadial`: gain `1 + k0 r² + k1 r⁴ + … + k4 r¹⁰`, radius 1 at the
    /// farthest corner.
    Dng { center: [f64; 2], k: [f64; 5] },
    /// Lensfun "pa": the lens darkens by `1 + k1 r² + k2 r⁴ + k3 r⁶`, radius 1 at the
    /// corner.
    Pa { k1: f64, k2: f64, k3: f64 },
}

impl LensVignetting {
    /// Multiplier that undoes the fall-off at normalised radius `r` (1 = corner).
    pub fn gain(&self, r: f64) -> f64 {
        let r2 = r * r;
        match self {
            LensVignetting::Dng { k, .. } => {
                let mut g = 1.0;
                let mut p = r2;
                for c in k {
                    g += c * p;
                    p *= r2;
                }
                g
            }
            LensVignetting::Pa { k1, k2, k3 } => {
                let fall = 1.0 + k1 * r2 + k2 * r2 * r2 + k3 * r2 * r2 * r2;
                // A fitted polynomial can dip towards zero past its data; cap the boost.
                1.0 / fall.max(0.25)
            }
        }
    }

    pub fn center(&self) -> [f64; 2] {
        match self {
            LensVignetting::Dng { center, .. } => *center,
            LensVignetting::Pa { .. } => [0.5, 0.5],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radial_models_are_identity_at_zero_coefficients() {
        for m in [
            Radial::Poly3 { k1: 0.0 },
            Radial::Poly5 { k1: 0.0, k2: 0.0 },
            Radial::PtLens { a: 0.0, b: 0.0, c: 0.0 },
            Radial::Linear { k: 1.0 },
            Radial::TcaPoly3 { b: 0.0, c: 0.0, v: 1.0 },
        ] {
            assert!((m.factor(0.7) - 1.0).abs() < 1e-12, "{m:?}");
        }
        // PTLens keeps r = 1 fixed whatever the coefficients.
        assert!((Radial::PtLens { a: 0.02, b: -0.08, c: 0.03 }.factor(1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn vignetting_gain_brightens_the_corners() {
        let pa = LensVignetting::Pa { k1: -0.3, k2: 0.0, k3: 0.0 };
        assert!((pa.gain(0.0) - 1.0).abs() < 1e-12);
        assert!((pa.gain(1.0) - 1.0 / 0.7).abs() < 1e-9);
    }
}
