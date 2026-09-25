//! PRD Step 8 finishing: edge vignette and procedural analog film grain, the very
//! last look stage (on upright scene-linear Rec.2020).
//!
//! Grain is value noise on a lattice whose cell is a fraction of the frame's long
//! side, so the preview and the full-size export show the same grain *structure*;
//! when a cell is smaller than a preview pixel, the grain is attenuated as averaging
//! would (σ ∝ cell size) instead of aliasing into sparkle. It is multiplicative and
//! strongest in the midtones, like silver grain. The pattern is fixed (seeded), so it
//! doesn't crawl between renders.

use epikos_core::ImageRgbF32;
use epikos_sidecar::Finishing;
use rayon::prelude::*;

use super::smoothstep;

const SEED: u32 = 0x5eed_9a1e;

/// Deterministic lattice value in about [−1, 1] (triangular distribution).
fn lattice(ix: i32, iy: i32, seed: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x8da6_b343) ^ (iy as u32).wrapping_mul(0xd816_3841) ^ seed;
    h ^= h >> 13;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 16;
    let a = (h & 0xffff) as f32 / 65535.0;
    let b = (h >> 16) as f32 / 65535.0;
    a + b - 1.0
}

/// Smoothly interpolated value noise at `(x, y)` in lattice units.
fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let top = lattice(ix, iy, seed) * (1.0 - sx) + lattice(ix + 1, iy, seed) * sx;
    let bottom = lattice(ix, iy + 1, seed) * (1.0 - sx) + lattice(ix + 1, iy + 1, seed) * sx;
    top * (1.0 - sy) + bottom * sy
}

pub(crate) fn apply_finishing(rgb: &mut ImageRgbF32, f: &Finishing) {
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    if f.is_neutral() || w < 2 || h < 2 {
        return;
    }
    let pct = |v: f32| (v / 100.0).clamp(0.0, 1.0);
    let spct = |v: f32| (v / 100.0).clamp(-1.0, 1.0);
    let long = w.max(h) as f32;

    // Grain: cell 0.04 %–0.25 % of the long side (≈ 3–20 px on a 8 000 px export).
    let grain = pct(f.grain);
    let cell = (0.0004 + 0.0021 * pct(f.grain_size)) * long;
    let (cell_px, grain_gain) = (cell.max(1.0), 0.35 * grain * cell.min(1.0));
    let rough = pct(f.grain_roughness);
    let fine_gain = 0.7 * rough * (0.5 * cell).min(1.0);

    // Vignette geometry: ellipse inscribed in the frame, blended to a circle for
    // positive roundness and towards a rounded rectangle for negative.
    let amount = spct(f.vignette);
    let round = spct(f.vignette_roundness);
    let circle = round.max(0.0);
    let p = 2.0 + 6.0 * (-round).max(0.0);
    let (ax, ay) = (1.0 + (w as f32 / long - 1.0) * circle, 1.0 + (h as f32 / long - 1.0) * circle);
    let dist = |u: f32, v: f32| ((u * ax).abs().powf(p) + (v * ay).abs().powf(p)).powf(1.0 / p);
    let corner = dist(1.0, 1.0);
    let mid = 0.25 + 0.65 * pct(f.vignette_midpoint);
    let feather = 0.05 + 0.9 * pct(f.vignette_feather);
    let (lo, hi) = (mid - 0.5 * feather, mid + 0.5 * feather);

    let factors: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            let mut k = 1.0;
            if amount != 0.0 {
                let (u, v) = ((x + 0.5) / w as f32 * 2.0 - 1.0, (y + 0.5) / h as f32 * 2.0 - 1.0);
                let fall = smoothstep(lo, hi, dist(u, v) / corner);
                k *= 2f32.powf(1.5 * amount * fall);
            }
            if grain > 0.0 {
                let (gx, gy) = (x / cell_px, y / cell_px);
                let n = value_noise(gx, gy, SEED) * grain_gain
                    + value_noise(gx * 2.0 + 17.3, gy * 2.0 + 5.1, SEED ^ 0x9e37_79b9) * grain_gain * fine_gain;
                let lum = (0.2627 * rgb.r[i] + 0.678 * rgb.g[i] + 0.0593 * rgb.b[i]).max(0.0);
                let t = lum.powf(1.0 / 2.2).min(1.0);
                let mids = 0.35 + 0.65 * 4.0 * t * (1.0 - t);
                k *= (1.0 + n * mids).max(0.0);
            }
            k
        })
        .collect();
    for plane in [&mut rgb.r, &mut rgb.g, &mut rgb.b] {
        plane.par_iter_mut().zip(factors.par_iter()).for_each(|(v, k)| *v *= k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    fn grey(w: u32, h: u32, v: f32) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
        for p in [&mut img.r, &mut img.g, &mut img.b] {
            p.fill(v);
        }
        img
    }

    fn std_dev(v: &[f32]) -> f32 {
        let m = v.iter().sum::<f32>() / v.len() as f32;
        (v.iter().map(|x| (x - m).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
    }

    #[test]
    fn neutral_finishing_is_identity() {
        let mut img = grey(32, 24, 0.18);
        apply_finishing(&mut img, &Finishing::default());
        assert!(img.g.iter().all(|&v| v == 0.18));
    }

    #[test]
    fn vignette_darkens_corners_not_the_centre() {
        let mut img = grey(300, 200, 0.18);
        apply_finishing(&mut img, &Finishing { vignette: -100.0, ..Default::default() });
        let (centre, corner) = (img.g[img.index(150, 100)], img.g[img.index(0, 0)]);
        assert!((centre - 0.18).abs() < 1e-3, "centre {centre}");
        assert!(corner < 0.18 * 0.45, "corner {corner}");
    }

    #[test]
    fn grain_is_deterministic_neutral_and_scaled_to_the_frame() {
        let f = Finishing { grain: 100.0, ..Default::default() };
        let mut a = grey(400, 300, 0.18);
        let mut b = grey(400, 300, 0.18);
        apply_finishing(&mut a, &f);
        apply_finishing(&mut b, &f);
        assert_eq!(a.g, b.g, "grain must not change between renders");
        assert_eq!(a.r, a.g, "grain is monochrome");
        let sd = std_dev(&a.g) / 0.18;
        assert!(sd > 0.03 && sd < 0.3, "relative grain σ {sd}");
        let mean = a.g.iter().sum::<f32>() / a.g.len() as f32;
        assert!((mean - 0.18).abs() < 0.01, "grain shifted the level: {mean}");

        // A small preview of the same frame: finer relative cells, weaker grain.
        let mut small = grey(100, 75, 0.18);
        apply_finishing(&mut small, &f);
        assert!(std_dev(&small.g) < std_dev(&a.g));
    }
}
