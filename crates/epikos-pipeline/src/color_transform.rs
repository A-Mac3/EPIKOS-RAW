use epikos_core::{ColorSpace, ImageRgbF32, SensorProfile};

use crate::matrix::{effective_xyz_to_cam, invert, mul, Mat3, REC2020_TO_XYZ};

/// Apply as-shot (or sidecar) white-balance multipliers in camera RGB.
pub fn apply_white_balance(image: &mut ImageRgbF32, wb: [f32; 4]) {
    let valid = |v: f32| v.is_finite() && v > 1e-6;
    let wr = if valid(wb[0]) { wb[0] } else { 1.0 };
    let wg = if valid(wb[1]) { wb[1] } else { 1.0 };
    let wb_ = if valid(wb[2]) { wb[2] } else { 1.0 };
    let scale = 1.0 / wg;
    for i in 0..image.len() {
        image.r[i] *= wr * scale;
        image.b[i] *= wb_ * scale;
    }
}

/// White-balanced camera RGB → linear Rec.2020.
///
/// Uses the camera's XYZ → camera matrix, row-normalised (as dcraw/RawTherapee do) so a
/// white-balanced neutral (R = G = B) lands exactly on Rec.2020 neutral.
pub fn camera_to_linear_rec2020(image: &mut ImageRgbF32, profile: &SensorProfile) {
    let m = rec2020_from_camera(&profile.xyz_to_cam);
    for i in 0..image.len() {
        let (r, g, b) = (image.r[i], image.g[i], image.b[i]);
        image.r[i] = m[0][0] * r + m[0][1] * g + m[0][2] * b;
        image.g[i] = m[1][0] * r + m[1][1] * g + m[1][2] * b;
        image.b[i] = m[2][0] * r + m[2][1] * g + m[2][2] * b;
    }
    image.space = ColorSpace::LinearRec2020;
}

fn rec2020_from_camera(xyz_to_cam: &Mat3) -> Mat3 {
    const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut cam_from_rec = mul(&effective_xyz_to_cam(xyz_to_cam), &REC2020_TO_XYZ);
    for row in cam_from_rec.iter_mut() {
        let sum: f32 = row.iter().sum();
        if sum.abs() < 1e-9 {
            return IDENTITY;
        }
        row.iter_mut().for_each(|v| *v /= sum);
    }
    invert(&cam_from_rec).unwrap_or(IDENTITY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::apply;

    /// Adobe DNG ColorMatrix2 (D65) for the Canon EOS 5D Mark IV.
    const CANON_5D4: Mat3 = [
        [0.6446, -0.0366, -0.0864],
        [-0.4436, 1.2204, 0.2513],
        [-0.0952, 0.2496, 0.6348],
    ];

    #[test]
    fn white_balanced_neutral_maps_to_rec2020_neutral() {
        let m = rec2020_from_camera(&CANON_5D4);
        let out = apply(&m, [0.5, 0.5, 0.5]);
        for c in out {
            assert!((c - 0.5).abs() < 1e-4, "{out:?}");
        }
    }

    #[test]
    fn missing_matrix_is_identity_not_nan() {
        let nan = [[f32::NAN; 3]; 3];
        for m in [[[0.0; 3]; 3], nan] {
            let out = apply(&rec2020_from_camera(&m), [0.2, 0.4, 0.6]);
            assert!(
                out.iter()
                    .zip([0.2, 0.4, 0.6])
                    .all(|(a, b)| (a - b).abs() < 1e-4),
                "{out:?}"
            );
        }
    }
}
