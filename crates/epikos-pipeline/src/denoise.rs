//! Sensor noise reduction (PRD Step 1): wavelet shrinkage in a luma / chroma space.
//!
//! Runs on white-balanced camera RGB, before any resampling (CA, distortion) so noise
//! is still uncorrelated between pixels. The image is split into luma
//! `Y = (R + 2G + B) / 4` and colour differences `R − G`, `B − G`, each plane is
//! variance-stabilised (shot noise grows with √signal), decomposed with an à trous
//! B3-spline wavelet, and every detail band is soft-thresholded against its own noise
//! level (median absolute deviation). Measuring noise on the image itself makes one
//! slider setting behave alike across ISOs and across preview / full resolution.

use epikos_core::ImageRgbF32;
use epikos_sidecar::NoiseReduction;
use rayon::prelude::*;

/// Luma keeps fine texture: fewer bands, weaker on coarse ones.
const LUMA_BAND_WEIGHTS: [f32; 4] = [1.0, 0.8, 0.5, 0.25];
/// Colour noise is blotchy and spans many scales.
const CHROMA_BAND_WEIGHTS: [f32; 5] = [1.0, 1.0, 1.0, 0.8, 0.6];
/// Threshold, in noise σ, at slider 100.
const LUMA_MAX_K: f32 = 3.0;
const CHROMA_MAX_K: f32 = 4.0;
/// Keeps the √ stabilisation finite in the deep shadows.
const FLOOR: f32 = 1e-4;
/// Noise statistics are taken from at most this many samples per band.
const MAD_SAMPLES: usize = 1 << 16;

/// Whether `nr` changes the image at all (lets callers skip the work).
pub fn is_active(nr: &NoiseReduction, monochrome: bool) -> bool {
    nr.luminance > 0.0 || (!monochrome && nr.color > 0.0)
}

/// Denoise `rgb` in place. Monochrome images only get luminance smoothing.
pub fn reduce_noise(rgb: &mut ImageRgbF32, nr: &NoiseReduction, monochrome: bool) {
    if !is_active(nr, monochrome) || rgb.width < 8 || rgb.height < 8 {
        return;
    }
    let luma_k = LUMA_MAX_K * (nr.luminance.clamp(0.0, 100.0) / 100.0);
    let chroma_k = if monochrome {
        0.0
    } else {
        CHROMA_MAX_K * (nr.color.clamp(0.0, 100.0) / 100.0)
    };
    let (w, h) = (rgb.width as usize, rgb.height as usize);

    // RGB → (√Y, (R−G)/√Y, (B−G)/√Y), reusing the planes: r ← U, g ← Y, b ← V.
    rgb.r
        .par_iter_mut()
        .zip(rgb.g.par_iter_mut())
        .zip(rgb.b.par_iter_mut())
        .for_each(|((r, g), b)| {
            let y = ((*r + 2.0 * *g + *b) * 0.25).max(0.0);
            let s = (y + FLOOR).sqrt();
            let (u, v) = ((*r - *g) / s, (*b - *g) / s);
            *r = u;
            *g = s;
            *b = v;
        });

    if luma_k > 0.0 {
        shrink(&mut rgb.g, w, h, luma_k, &LUMA_BAND_WEIGHTS);
    }
    if chroma_k > 0.0 {
        shrink(&mut rgb.r, w, h, chroma_k, &CHROMA_BAND_WEIGHTS);
        shrink(&mut rgb.b, w, h, chroma_k, &CHROMA_BAND_WEIGHTS);
    }

    rgb.r
        .par_iter_mut()
        .zip(rgb.g.par_iter_mut())
        .zip(rgb.b.par_iter_mut())
        .for_each(|((r, g), b)| {
            let s = g.max(0.0);
            let y = s * s - FLOOR;
            let (u, v) = (*r * s, *b * s);
            let green = y - (u + v) * 0.25;
            *r = u + green;
            *g = green;
            *b = v + green;
        });
}

/// À trous wavelet shrinkage of one plane, in place. `weights[i]` scales the threshold
/// of detail band `i`; the residual (coarsest) band passes through untouched.
fn shrink(plane: &mut [f32], w: usize, h: usize, k: f32, weights: &[f32]) {
    let mut coarse = plane.to_vec();
    let mut next = vec![0.0f32; plane.len()];
    let mut tmp = vec![0.0f32; plane.len()];
    plane.fill(0.0);

    for (level, &weight) in weights.iter().enumerate() {
        let step = 1usize << level;
        b3_blur(&coarse, &mut next, &mut tmp, w, h, step);
        let sigma = noise_sigma(&coarse, &next);
        let t = k * weight * sigma;
        plane
            .par_iter_mut()
            .zip(coarse.par_iter())
            .zip(next.par_iter())
            .for_each(|((out, &c), &n)| *out += soft_threshold(c - n, t));
        std::mem::swap(&mut coarse, &mut next);
    }
    plane
        .par_iter_mut()
        .zip(coarse.par_iter())
        .for_each(|(out, &c)| *out += c);
}

fn soft_threshold(d: f32, t: f32) -> f32 {
    if d > t {
        d - t
    } else if d < -t {
        d + t
    } else {
        0.0
    }
}

/// Robust noise σ of the detail band `coarse − next`: MAD / 0.6745 over a strided sample.
fn noise_sigma(coarse: &[f32], next: &[f32]) -> f32 {
    let stride = (coarse.len() / MAD_SAMPLES).max(1);
    let mut d: Vec<f32> = (0..coarse.len())
        .step_by(stride)
        .map(|i| (coarse[i] - next[i]).abs())
        .filter(|v| v.is_finite())
        .collect();
    if d.is_empty() {
        return 0.0;
    }
    let mid = d.len() / 2;
    let (_, median, _) = d.select_nth_unstable_by(mid, f32::total_cmp);
    *median / 0.6745
}

/// Separable 5-tap B3-spline blur with holes of `step` pixels, mirrored at the edges.
fn b3_blur(src: &[f32], dst: &mut [f32], tmp: &mut [f32], w: usize, h: usize, step: usize) {
    const K: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
    let offsets = [-2 * step as isize, -(step as isize), 0, step as isize, 2 * step as isize];

    tmp.par_chunks_mut(w)
        .zip(src.par_chunks(w))
        .for_each(|(out, row)| {
            for (x, o) in out.iter_mut().enumerate() {
                *o = offsets
                    .iter()
                    .zip(K)
                    .map(|(&dx, k)| k * row[mirror(x as isize + dx, w)])
                    .sum();
            }
        });
    dst.par_chunks_mut(w).enumerate().for_each(|(y, out)| {
        let rows = offsets.map(|dy| mirror(y as isize + dy, h) * w);
        for (x, o) in out.iter_mut().enumerate() {
            *o = rows.iter().zip(K).map(|(&r, k)| k * tmp[r + x]).sum();
        }
    });
}

/// Reflect an out-of-range index back into `0..n` (edge pixel not repeated).
fn mirror(i: isize, n: usize) -> usize {
    let n = n as isize;
    if n == 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let m = i.rem_euclid(period);
    (if m < n { m } else { period - m }) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    /// Deterministic uniform noise in [-1, 1].
    fn lcg(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    fn noisy_grey(w: u32, h: u32, level: f32, amp: f32) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(w, h, ColorSpace::CameraRgb);
        let mut seed = 7;
        for i in 0..img.len() {
            img.r[i] = level + amp * lcg(&mut seed);
            img.g[i] = level + amp * lcg(&mut seed);
            img.b[i] = level + amp * lcg(&mut seed);
        }
        img
    }

    fn std_dev(v: &[f32]) -> f32 {
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
    }

    /// Luma only: with independent R, G, B noise half of a channel's variance is chroma.
    fn luma(img: &ImageRgbF32, x: u32, y: u32) -> f32 {
        let i = img.index(x, y);
        (img.r[i] + 2.0 * img.g[i] + img.b[i]) * 0.25
    }

    fn chroma_noise(img: &ImageRgbF32) -> f32 {
        let d: Vec<f32> = img.r.iter().zip(&img.g).map(|(r, g)| r - g).collect();
        std_dev(&d)
    }

    #[test]
    fn zero_strength_is_identity() {
        let before = noisy_grey(32, 32, 0.2, 0.02);
        let mut img = before.clone();
        let nr = NoiseReduction {
            luminance: 0.0,
            color: 0.0,
        };
        reduce_noise(&mut img, &nr, false);
        assert_eq!(img.r, before.r);
    }

    #[test]
    fn transform_round_trips_on_a_neutral_image() {
        // Neutral pixels have zero chroma, so colour NR has nothing to remove and the
        // luma / chroma transform must invert to float precision.
        let mut img = ImageRgbF32::new(64, 64, ColorSpace::CameraRgb);
        for i in 0..img.len() {
            let v = 0.05 + 0.5 * (i % 64) as f32 / 64.0;
            (img.r[i], img.g[i], img.b[i]) = (v, v, v);
        }
        let before = img.clone();
        let nr = NoiseReduction {
            luminance: 0.0,
            color: 100.0,
        };
        reduce_noise(&mut img, &nr, false);
        for i in 0..img.len() {
            for (a, b) in [(img.r[i], before.r[i]), (img.g[i], before.g[i]), (img.b[i], before.b[i])] {
                assert!((a - b).abs() < 1e-5, "{i}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn colour_nr_removes_chroma_noise_and_keeps_level() {
        let before = noisy_grey(128, 128, 0.25, 0.03);
        let mut img = before.clone();
        reduce_noise(
            &mut img,
            &NoiseReduction {
                luminance: 0.0,
                color: 100.0,
            },
            false,
        );
        assert!(chroma_noise(&img) < 0.4 * chroma_noise(&before));
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!((mean(&img.g) - mean(&before.g)).abs() < 0.01);
    }

    #[test]
    fn luma_nr_smooths_and_preserves_a_strong_edge() {
        let mut img = noisy_grey(128, 64, 0.1, 0.02);
        for y in 0..64 {
            for x in 64..128 {
                let i = img.index(x, y);
                img.r[i] += 0.5;
                img.g[i] += 0.5;
                img.b[i] += 0.5;
            }
        }
        let before = img.clone();
        let nr = NoiseReduction {
            luminance: 100.0,
            color: 0.0,
        };
        reduce_noise(&mut img, &nr, false);

        let flat = |im: &ImageRgbF32| -> Vec<f32> {
            (16..48)
                .flat_map(|y| (8..48).map(move |x| (x, y)))
                .map(|(x, y)| luma(im, x, y))
                .collect()
        };
        let (after_sd, before_sd) = (std_dev(&flat(&img)), std_dev(&flat(&before)));
        assert!(after_sd < 0.5 * before_sd, "{after_sd} vs {before_sd}");
        // The step between x = 60 and x = 68 must survive almost intact.
        let step = |im: &ImageRgbF32| luma(im, 68, 32) - luma(im, 60, 32);
        assert!(step(&img) > 0.45, "edge softened to {}", step(&img));
    }

    #[test]
    fn mirror_reflects_without_repeating_the_edge() {
        assert_eq!(mirror(-1, 5), 1);
        assert_eq!(mirror(-2, 5), 2);
        assert_eq!(mirror(5, 5), 3);
        assert_eq!(mirror(12, 5), 4);
        assert_eq!(mirror(3, 1), 0);
    }
}
