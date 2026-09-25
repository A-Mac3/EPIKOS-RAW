//! 3×3 colour-matrix helpers shared by white balance and the camera → Rec.2020 transform.

pub type Mat3 = [[f32; 3]; 3];

/// Linear Rec.2020 → CIE XYZ (D65).
pub const REC2020_TO_XYZ: Mat3 = [
    [0.636_958, 0.144_617, 0.168_881],
    [0.262_700, 0.677_998, 0.059_302],
    [0.0, 0.028_073, 1.060_985],
];

/// CIE XYZ (D65) → linear Rec.2020.
pub const XYZ_TO_REC2020: Mat3 = [
    [1.716_651, -0.355_671, -0.253_366],
    [-0.666_684, 1.616_481, 0.015_769],
    [0.017_640, -0.042_771, 0.942_103],
];

/// The camera's XYZ → camera matrix, or — when the file carries none (all zeros or
/// non-finite) — one that treats camera RGB as linear Rec.2020.
pub fn effective_xyz_to_cam(xyz_to_cam: &Mat3) -> Mat3 {
    let flat = xyz_to_cam.iter().flatten();
    let missing = flat.clone().all(|v| *v == 0.0) || flat.clone().any(|v| !v.is_finite());
    if missing {
        XYZ_TO_REC2020
    } else {
        *xyz_to_cam
    }
}

pub fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

pub fn apply(m: &Mat3, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

pub fn invert(m: &Mat3) -> Option<Mat3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let s = 1.0 / det;
    Some([
        [
            (e * i - f * h) * s,
            (c * h - b * i) * s,
            (b * f - c * e) * s,
        ],
        [
            (f * g - d * i) * s,
            (a * i - c * g) * s,
            (c * d - a * f) * s,
        ],
        [
            (d * h - e * g) * s,
            (b * g - a * h) * s,
            (a * e - b * d) * s,
        ],
    ])
}
