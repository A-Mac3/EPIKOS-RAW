//! Plane resampling shared by the mask models and the develop pipeline.

use rayon::prelude::*;

/// Separable resize: box-averages when shrinking (no aliasing), bilinear when enlarging.
pub fn resize_plane(src: &[f32], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<f32> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    // Horizontal pass: sh rows of dw.
    let mut tmp = vec![0.0f32; dw * sh];
    tmp.par_chunks_mut(dw).enumerate().for_each(|(y, row)| {
        let s = &src[y * sw..(y + 1) * sw];
        for (x, o) in row.iter_mut().enumerate() {
            *o = sample_1d(|i| s[i], sw, dw, x);
        }
    });
    // Vertical pass.
    let mut out = vec![0.0f32; dw * dh];
    out.par_chunks_mut(dw).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            *o = sample_1d(|i| tmp[i * dw + x], sh, dh, y);
        }
    });
    out
}

/// Output sample `i` of a length-`n` signal resampled to length `m`.
fn sample_1d(at: impl Fn(usize) -> f32, n: usize, m: usize, i: usize) -> f32 {
    let scale = n as f32 / m as f32;
    if scale > 1.0 {
        // Area average over [i·scale, (i+1)·scale).
        let (a, b) = (i as f32 * scale, (i + 1) as f32 * scale);
        let (first, last) = (a.floor() as usize, (b.ceil() as usize).min(n));
        let (mut sum, mut weight) = (0.0, 0.0);
        for j in first..last {
            let w = (b.min(j as f32 + 1.0) - a.max(j as f32)).max(0.0);
            sum += w * at(j);
            weight += w;
        }
        sum / weight.max(1e-6)
    } else {
        // Pixel-centre aligned bilinear.
        let x = ((i as f32 + 0.5) * scale - 0.5).clamp(0.0, (n - 1) as f32);
        let x0 = x.floor() as usize;
        let x1 = (x0 + 1).min(n - 1);
        let t = x - x0 as f32;
        at(x0) * (1.0 - t) + at(x1) * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_preserves_flat_fields_and_means() {
        let src = vec![0.25f32; 37 * 23];
        for (w, h) in [(10, 7), (80, 51), (37, 23)] {
            let out = resize_plane(&src, 37, 23, w, h);
            assert_eq!(out.len(), (w * h) as usize);
            assert!(out.iter().all(|v| (v - 0.25).abs() < 1e-5));
        }
        // Shrinking a checkerboard averages it instead of aliasing.
        let checker: Vec<f32> = (0..64 * 64).map(|i| ((i % 64 + i / 64) % 2) as f32).collect();
        let out = resize_plane(&checker, 64, 64, 16, 16);
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-5));
    }
}
