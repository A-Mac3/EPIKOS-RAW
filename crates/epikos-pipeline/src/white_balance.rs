//! Absolute temperature/tint white balance with Adobe Camera Raw semantics.
//!
//! `temperature` is the correlated colour temperature (K) of the scene illuminant and
//! `tint` the offset from the Planckian locus: raising the temperature warms the image,
//! positive tint pushes it towards magenta. Camera gains are derived from the sensor's
//! colour matrix, so the same Kelvin value means the same thing on every camera.

use crate::matrix::{apply, effective_xyz_to_cam, Mat3};

const MIN_K: f32 = 2000.0;
const MAX_K: f32 = 25000.0;
const MAX_TINT: f32 = 150.0;
/// Δuv (CIE 1960) per tint unit, matching the DNG SDK's tint scale of 3000.
const TINT_SCALE: f32 = 1.0 / 3000.0;

/// Camera white-balance multipliers `[r, g, b, e]` (green-normalised) for an illuminant.
///
/// Returns `None` when the colour matrix cannot produce a positive camera response.
pub fn gains_for_temperature(xyz_to_cam: &Mat3, temperature: f32, tint: f32) -> Option<[f32; 4]> {
    let (x, y) = illuminant_xy(temperature, tint);
    let white = [x / y, 1.0, (1.0 - x - y) / y];
    let cam = apply(&effective_xyz_to_cam(xyz_to_cam), white);
    if cam.iter().any(|c| !c.is_finite() || *c <= 1e-6) {
        return None;
    }
    Some([cam[1] / cam[0], 1.0, cam[1] / cam[2], 1.0])
}

/// Inverse of [`gains_for_temperature`]: the (temperature, tint) that best explains `gains`.
/// Used to seed the UI sliders from the camera's as-shot multipliers.
pub fn temperature_for_gains(xyz_to_cam: &Mat3, gains: [f32; 4]) -> Option<(f32, f32)> {
    if gains[..3].iter().any(|g| !g.is_finite() || *g <= 1e-6) {
        return None;
    }
    let target = [(gains[0] / gains[1]).ln(), (gains[2] / gains[1]).ln()];
    let error = |mired: f32, tint: f32| -> f32 {
        match gains_for_temperature(xyz_to_cam, 1.0e6 / mired, tint) {
            Some(g) => {
                let dr = g[0].ln() - target[0];
                let db = g[2].ln() - target[1];
                dr * dr + db * db
            }
            None => f32::INFINITY,
        }
    };

    // Coarse grid in mired (perceptually uniform), then two refinement passes.
    let (mut best_m, mut best_t, mut best_e) = (1.0e6 / 5500.0, 0.0, f32::INFINITY);
    let (mut m_lo, mut m_hi) = (1.0e6 / MAX_K, 1.0e6 / MIN_K);
    let (mut t_lo, mut t_hi) = (-MAX_TINT, MAX_TINT);
    for steps in [48usize, 24, 24] {
        let dm = (m_hi - m_lo) / steps as f32;
        let dt = (t_hi - t_lo) / steps as f32;
        for i in 0..=steps {
            for j in 0..=steps {
                let m = m_lo + dm * i as f32;
                let t = t_lo + dt * j as f32;
                let e = error(m, t);
                if e < best_e {
                    (best_m, best_t, best_e) = (m, t, e);
                }
            }
        }
        m_lo = (best_m - 2.0 * dm).max(1.0e6 / MAX_K);
        m_hi = (best_m + 2.0 * dm).min(1.0e6 / MIN_K);
        t_lo = (best_t - 2.0 * dt).max(-MAX_TINT);
        t_hi = (best_t + 2.0 * dt).min(MAX_TINT);
    }
    best_e.is_finite().then_some((1.0e6 / best_m, best_t))
}

/// CIE xy chromaticity of the illuminant: Planckian locus (Kim et al. 2002) shifted
/// perpendicular to the locus in CIE 1960 uv by `tint`.
fn illuminant_xy(temperature: f32, tint: f32) -> (f32, f32) {
    let t = temperature.clamp(MIN_K, MAX_K);
    let tint = tint.clamp(-MAX_TINT, MAX_TINT);
    let (u, v) = xy_to_uv(planckian_xy(t));
    let (u2, v2) = xy_to_uv(planckian_xy((t * 1.01).min(MAX_K + 250.0)));
    let (du, dv) = (u2 - u, v2 - v);
    let len = (du * du + dv * dv).sqrt().max(1e-9);
    // Unit normal pointing towards green (+v). An illuminant assumed greener than the
    // locus is compensated with less green gain, so the image turns magenta (+tint).
    let (mut nu, mut nv) = (-dv / len, du / len);
    if nv < 0.0 {
        nu = -nu;
        nv = -nv;
    }
    let off = tint * TINT_SCALE;
    uv_to_xy(u + nu * off, v + nv * off)
}

fn planckian_xy(t: f32) -> (f32, f32) {
    let t = t as f64;
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.266_123_9e9 / t3 - 0.234_358_9e6 / t2 + 0.877_695_6e3 / t + 0.179_910
    } else {
        -3.025_846_9e9 / t3 + 2.107_037_9e6 / t2 + 0.222_634_7e3 / t + 0.240_390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.106_381_4 * x3 - 1.348_110_20 * x2 + 2.185_558_32 * x - 0.202_196_83
    } else if t <= 4000.0 {
        -0.954_947_6 * x3 - 1.374_185_93 * x2 + 2.091_370_15 * x - 0.167_488_67
    } else {
        3.081_758_0 * x3 - 5.873_386_70 * x2 + 3.751_129_97 * x - 0.370_014_83
    };
    (x as f32, y as f32)
}

fn xy_to_uv((x, y): (f32, f32)) -> (f32, f32) {
    let d = -2.0 * x + 12.0 * y + 3.0;
    (4.0 * x / d, 6.0 * y / d)
}

fn uv_to_xy(u: f32, v: f32) -> (f32, f32) {
    let d = 2.0 * u - 8.0 * v + 4.0;
    (3.0 * u / d, 2.0 * v / d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    #[test]
    fn d65_on_locus_is_close_to_standard_white() {
        // The Planckian locus passes ~0.003 uv from D65 (0.3127, 0.3290).
        let (x, y) = illuminant_xy(6504.0, 0.0);
        assert!(
            (x - 0.3135).abs() < 0.003 && (y - 0.3237).abs() < 0.003,
            "({x}, {y})"
        );
    }

    #[test]
    fn higher_temperature_warms_the_image() {
        let cool = gains_for_temperature(&IDENTITY, 3000.0, 0.0).unwrap();
        let warm = gains_for_temperature(&IDENTITY, 10000.0, 0.0).unwrap();
        assert!(
            warm[0] > cool[0],
            "red gain should rise: {cool:?} → {warm:?}"
        );
        assert!(
            warm[2] < cool[2],
            "blue gain should fall: {cool:?} → {warm:?}"
        );
    }

    #[test]
    fn positive_tint_turns_the_image_magenta() {
        let neutral = gains_for_temperature(&IDENTITY, 5500.0, 0.0).unwrap();
        let magenta = gains_for_temperature(&IDENTITY, 5500.0, 50.0).unwrap();
        // Green-normalised: less green gain shows up as more red and blue gain.
        assert!(magenta[0] > neutral[0] && magenta[2] > neutral[2]);
    }

    #[test]
    fn inverse_recovers_temperature_and_tint() {
        let xyz_to_cam = [[0.65, 0.25, 0.10], [0.28, 0.85, -0.13], [0.02, -0.12, 1.15]];
        for (t, tint) in [(3200.0, 0.0), (5600.0, 12.0), (7500.0, -20.0)] {
            let gains = gains_for_temperature(&xyz_to_cam, t, tint).unwrap();
            let (t2, tint2) = temperature_for_gains(&xyz_to_cam, gains).unwrap();
            assert!((t2 - t).abs() / t < 0.01, "temp {t} → {t2}");
            assert!((tint2 - tint).abs() < 1.5, "tint {tint} → {tint2}");
        }
    }
}
