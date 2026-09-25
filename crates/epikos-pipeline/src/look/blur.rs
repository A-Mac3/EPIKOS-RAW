//! O(N) box blurs and the guided filter used by the texture and glow passes. Cost is
//! independent of radius, so radii can scale with image size (preview and full-size
//! export then see the same structures).

use rayon::prelude::*;

/// Mean over a (2r+1)² window, edges clamped. Returns a new plane.
pub(crate) fn box_blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    if r == 0 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let mut rows = vec![0.0; src.len()];
    box_rows(src, &mut rows, w, r);
    let t = transpose(&rows, w, h);
    box_rows(&t, &mut rows, h, r);
    transpose(&rows, h, w)
}

/// Repeated box blur: two passes give a tent, three a close Gaussian.
pub(crate) fn smooth(src: &[f32], w: usize, h: usize, r: usize, passes: usize) -> Vec<f32> {
    let mut out = box_blur(src, w, h, r);
    for _ in 1..passes {
        out = box_blur(&out, w, h, r);
    }
    out
}

/// Self-guided edge-preserving smoothing (He, Sun & Tang 2010). Structures with
/// variance well above `eps` in the window are kept; weaker ones are averaged away.
pub(crate) fn guided(p: &[f32], w: usize, h: usize, r: usize, eps: f32) -> Vec<f32> {
    let mean = box_blur(p, w, h, r);
    let sq: Vec<f32> = p.par_iter().map(|v| v * v).collect();
    let mean_sq = box_blur(&sq, w, h, r);
    drop(sq);
    let (a, b): (Vec<f32>, Vec<f32>) = mean
        .par_iter()
        .zip(mean_sq.par_iter())
        .map(|(&m, &m2)| {
            let var = (m2 - m * m).max(0.0);
            let a = var / (var + eps);
            (a, m - a * m)
        })
        .unzip();
    drop((mean, mean_sq));
    let (ma, mb) = (box_blur(&a, w, h, r), box_blur(&b, w, h, r));
    p.par_iter()
        .zip(ma.par_iter().zip(mb.par_iter()))
        .map(|(&v, (&a, &b))| a * v + b)
        .collect()
}

fn box_rows(src: &[f32], dst: &mut [f32], w: usize, r: usize) {
    let norm = 1.0 / (2 * r + 1) as f64;
    dst.par_chunks_mut(w)
        .zip(src.par_chunks(w))
        .for_each(|(out, row)| {
            let last = w as isize - 1;
            let at = |i: isize| row[i.clamp(0, last) as usize] as f64;
            let r = r as isize;
            let mut sum: f64 = (-r..=r).map(at).sum();
            for (x, o) in out.iter_mut().enumerate() {
                *o = (sum * norm) as f32;
                let x = x as isize;
                sum += at(x + r + 1) - at(x - r);
            }
        });
}

/// `w × h` row-major → `h × w`, in cache-friendly tiles of output rows.
fn transpose(src: &[f32], w: usize, h: usize) -> Vec<f32> {
    const TILE: usize = 32;
    let mut out = vec![0.0; src.len()];
    out.par_chunks_mut(TILE * h)
        .enumerate()
        .for_each(|(tile, block)| {
            let x0 = tile * TILE;
            let cols = block.len() / h;
            for y in 0..h {
                let row = &src[y * w + x0..y * w + x0 + cols];
                for (dx, &v) in row.iter().enumerate() {
                    block[dx * h + y] = v;
                }
            }
        });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_box(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
        let r = r as isize;
        let at = |x: isize, y: isize| {
            src[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize]
        };
        let mut out = vec![0.0; w * h];
        for y in 0..h as isize {
            for x in 0..w as isize {
                let mut s = 0.0;
                for dy in -r..=r {
                    for dx in -r..=r {
                        s += at(x + dx, y + dy);
                    }
                }
                out[y as usize * w + x as usize] = s / ((2 * r + 1) * (2 * r + 1)) as f32;
            }
        }
        out
    }

    #[test]
    fn box_blur_matches_the_direct_sum() {
        let (w, h) = (37, 23);
        let src: Vec<f32> = (0..w * h).map(|i| ((i * 7919) % 101) as f32 / 100.0).collect();
        for r in [1, 3, 30] {
            let fast = box_blur(&src, w, h, r);
            let slow = naive_box(&src, w, h, r);
            for (a, b) in fast.iter().zip(&slow) {
                // The naive f32 sum over 3 721 terms is the less exact of the two.
                assert!((a - b).abs() < 1e-4, "r={r}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn guided_filter_keeps_edges_and_flattens_ripples() {
        let (w, h) = (64, 16);
        // Step of 0.5 at x = 32 plus a ±0.01 ripple.
        let src: Vec<f32> = (0..w * h)
            .map(|i| {
                let x = i % w;
                (if x < 32 { 0.2 } else { 0.7 }) + if x % 2 == 0 { 0.01 } else { -0.01 }
            })
            .collect();
        let out = guided(&src, w, h, 4, 0.01f32.powi(2) * 4.0);
        let at = |x: usize| out[8 * w + x];
        assert!((at(40) - at(41)).abs() < 0.005, "ripple survived");
        assert!(at(34) - at(29) > 0.45, "edge softened: {} {}", at(29), at(34));
    }
}
