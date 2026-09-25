//! Markesteijn X-Trans demosaic (single pass, four directions).
//!
//! Frank Markesteijn's algorithm as popularised by dcraw, RawTherapee and darktable:
//!
//! 1. Green is interpolated at every red/blue site in four directions (horizontal,
//!    vertical, both diagonals), each estimate clamped to the range of the surrounding
//!    green hexagon.
//! 2. Red and blue are filled from colour *differences* (R−G, B−G), which vary far more
//!    smoothly than the channels themselves; this is what suppresses colour fringing.
//! 3. Each directional result is converted to CIELab; the per-pixel homogeneity of each
//!    direction is measured, and only the most homogeneous directions are averaged, so
//!    interpolation follows edges instead of crossing them.
//!
//! Work is done in overlapping tiles so memory stays bounded and tiles run in parallel.
//! The outer 8-pixel border keeps the simple interpolator's result.

use epikos_core::{CfaPattern, ColorSpace, ImageRgbF32};
use rayon::prelude::*;

/// Tile size and overlap (each tile writes only its interior, `TS - 2·MARGIN` wide).
const TS: i32 = 512;
const MARGIN: i32 = 8;
const NDIR: usize = 4;

type Off = (i32, i32); // (dy, dx)

/// Per 3×3 phase: 8 neighbour offsets around the pixel. Index `[row%3][col%3]`.
type HexTable = [[[Off; 8]; 3]; 3];

pub(crate) fn markesteijn(
    data: &[f32],
    w: u32,
    h: u32,
    cfa: &CfaPattern,
    fallback: ImageRgbF32,
) -> ImageRgbF32 {
    let (wi, hi) = (w as i32, h as i32);
    if wi < TS.min(64) || hi < TS.min(64) || !cfa.is_xtrans() {
        return fallback;
    }
    let geo = Geometry::new(cfa);

    // Tile origins, dcraw-style: start at 3, advance by TS − 2·MARGIN.
    let step = TS - 2 * MARGIN;
    let mut origins = Vec::new();
    let mut top = 3;
    while top < hi - 19 {
        let mut left = 3;
        while left < wi - 19 {
            origins.push((top, left));
            left += step;
        }
        top += step;
    }

    let tiles: Vec<Tile> = origins
        .par_iter()
        .map(|&(top, left)| Tile::process(data, wi, hi, &geo, top, left))
        .collect();

    let mut out = fallback;
    for t in tiles {
        for (i, px) in t.out.iter().enumerate() {
            let (r, c) = (
                t.out_top + i as i32 / t.out_w,
                t.out_left + i as i32 % t.out_w,
            );
            let idx = (r * wi + c) as usize;
            out.r[idx] = px[0];
            out.g[idx] = px[1];
            out.b[idx] = px[2];
        }
    }
    out.space = ColorSpace::CameraRgb;
    out
}

struct Geometry {
    pattern: [[u8; 6]; 6],
    /// Row/column of a solitary green (green with four non-green orthogonal neighbours).
    sg: (i32, i32),
    /// `[0]`: offsets used at red/blue sites; `[1]`: at green sites.
    hex: [HexTable; 2],
}

impl Geometry {
    fn new(cfa: &CfaPattern) -> Self {
        let mut pattern = [[0u8; 6]; 6];
        for (r, row) in pattern.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = cfa.color_at(r, c);
            }
        }
        let mut geo = Self {
            pattern,
            sg: (0, 0),
            hex: [[[[(0, 0); 8]; 3]; 3]; 2],
        };

        // Orthogonal directions in order down, left, up, right (then down again, to close
        // the cycle), and the rotation that maps the canonical pattern onto direction d.
        const ORTH: [i32; 12] = [1, 0, 0, 1, -1, 0, 0, -1, 1, 0, 0, 1];
        const PATT: [[i32; 16]; 2] = [
            [0, 1, 0, -1, 2, 0, -1, 0, 1, 1, 1, -1, 0, 0, 0, 0],
            [0, 1, 0, -2, 1, 0, -2, 0, 1, 1, -2, -2, 1, -1, -1, 1],
        ];
        for row in 0..3i32 {
            for col in 0..3i32 {
                let g = (geo.fcol(row, col) == 1) as usize;
                let mut ng = 0;
                for d in (0..10).step_by(2) {
                    if geo.fcol(row + ORTH[d], col + ORTH[d + 2]) == 1 {
                        ng = 0;
                    } else {
                        ng += 1;
                    }
                    if ng == 4 {
                        geo.sg = (row, col);
                    }
                    if ng == g + 1 {
                        for c in 0..8 {
                            let v = ORTH[d] * PATT[g][c * 2] + ORTH[d + 1] * PATT[g][c * 2 + 1];
                            let hh =
                                ORTH[d + 2] * PATT[g][c * 2] + ORTH[d + 3] * PATT[g][c * 2 + 1];
                            let slot = c ^ ((g * 2) & d);
                            geo.hex[g][row as usize][col as usize][slot] = (v, hh);
                        }
                    }
                }
            }
        }
        geo
    }

    fn fcol(&self, row: i32, col: i32) -> u8 {
        self.pattern[row.rem_euclid(6) as usize][col.rem_euclid(6) as usize]
    }

    fn hex(&self, green: bool, row: i32, col: i32) -> &[Off; 8] {
        &self.hex[green as usize][row.rem_euclid(3) as usize][col.rem_euclid(3) as usize]
    }
}

struct Tile {
    out_top: i32,
    out_left: i32,
    out_w: i32,
    out: Vec<[f32; 3]>,
}

impl Tile {
    fn process(data: &[f32], w: i32, h: i32, geo: &Geometry, top: i32, left: i32) -> Tile {
        let mrow = (top + TS).min(h - 3);
        let mcol = (left + TS).min(w - 3);
        let (th, tw) = (mrow - top, mcol - left);
        let raw = |r: i32, c: i32| data[(r.clamp(0, h - 1) * w + c.clamp(0, w - 1)) as usize];

        // rgb[d]: tile image for direction d. Channel of the CFA colour holds the raw value.
        let n = (th * tw) as usize;
        let mut rgb = vec![vec![[0f32; 3]; n]; NDIR];
        let at = |r: i32, c: i32| {
            ((r - top).clamp(0, th - 1) * tw + (c - left).clamp(0, tw - 1)) as usize
        };

        // Green range (min, max) of the hexagon around each red/blue site.
        let green_range = |r: i32, c: i32| {
            let hex = geo.hex(false, r, c);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for &(dy, dx) in &hex[..6] {
                let v = raw(r + dy, c + dx);
                lo = lo.min(v);
                hi = hi.max(v);
            }
            (lo, hi)
        };

        for r in top..mrow {
            for c in left..mcol {
                let f = geo.fcol(r, c) as usize;
                let mut px = [0f32; 3];
                px[f] = raw(r, c);
                if f != 1 {
                    px[1] = green_range(r, c).0;
                }
                for buf in rgb.iter_mut() {
                    buf[at(r, c)] = px;
                }
            }
        }

        // 1. Green at red/blue sites in four directions.
        for r in top..mrow {
            for c in left..mcol {
                let f = geo.fcol(r, c) as usize;
                if f == 1 {
                    continue;
                }
                let hex = geo.hex(false, r, c);
                let g = |k: usize, s: i32| raw(r + hex[k].0 * s, c + hex[k].1 * s);
                let own = |k: usize, s: i32| raw(r + hex[k].0 * s, c + hex[k].1 * s);
                let p0 = raw(r, c);
                let mut est = [0f32; 4];
                est[0] = (174.0 * (g(1, 1) + g(0, 1)) - 46.0 * (g(1, 2) + g(0, 2))) / 256.0;
                est[1] = (223.0 * g(3, 1) + 33.0 * g(2, 1) + 92.0 * (p0 - own(2, -1))) / 256.0;
                for k in 0..2 {
                    est[2 + k] = (164.0 * g(4 + k, 1)
                        + 92.0 * g(4 + k, -2)
                        + 33.0 * (2.0 * p0 - own(4 + k, 3) - own(4 + k, -3)))
                        / 256.0;
                }
                let (lo, hi) = green_range(r, c);
                let swap = ((r - geo.sg.0).rem_euclid(3) == 0) as usize;
                for (k, e) in est.iter().enumerate() {
                    rgb[k ^ swap][at(r, c)][1] = e.clamp(lo, hi);
                }
            }
        }

        // 2a. Red and blue at solitary greens (from horizontal / vertical neighbours).
        let start = |from: i32, phase: i32| (from - phase + 4).div_euclid(3) * 3 + phase;
        let mut r = start(top, geo.sg.0);
        while r < mrow - 2 {
            let mut c = start(left, geo.sg.1);
            while c < mcol - 2 {
                solitary_green(&mut rgb, geo, &at, r, c);
                c += 3;
            }
            r += 3;
        }

        // 2b. Red at blue sites and blue at red sites.
        for r in top + 3..mrow - 3 {
            for c in left + 3..mcol - 3 {
                let f = 2 - geo.fcol(r, c) as i32;
                if f == 1 {
                    continue;
                }
                let f = f as usize;
                let near: Off = if (r - geo.sg.0).rem_euclid(3) != 0 {
                    (1, 0)
                } else {
                    (0, 1)
                };
                let far: Off = if near == (1, 0) { (0, 3) } else { (3, 0) };
                for (d, buf) in rgb.iter_mut().enumerate() {
                    let p = |o: Off, s: i32| buf[at(r + o.0 * s, c + o.1 * s)];
                    let g0 = buf[at(r, c)][1];
                    let along_near = (d == 0 && near == (0, 1)) || (d == 1 && near == (1, 0));
                    let use_near = d > 1
                        || along_near
                        || (g0 - p(near, 1)[1]).abs() + (g0 - p(near, -1)[1]).abs()
                            < 2.0 * ((g0 - p(far, 1)[1]).abs() + (g0 - p(far, -1)[1]).abs());
                    let o = if use_near { near } else { far };
                    let v = (p(o, 1)[f] + p(o, -1)[f] + 2.0 * g0 - p(o, 1)[1] - p(o, -1)[1]) / 2.0;
                    buf[at(r, c)][f] = v.max(0.0);
                }
            }
        }

        // 2c. Red and blue for the 2×2 green blocks.
        for r in top + 2..mrow - 2 {
            if (r - geo.sg.0).rem_euclid(3) == 0 {
                continue;
            }
            for c in left + 2..mcol - 2 {
                if (c - geo.sg.1).rem_euclid(3) == 0 {
                    continue;
                }
                let hex = geo.hex(true, r, c);
                for (k, buf) in rgb.iter_mut().enumerate() {
                    let (a, b) = (hex[2 * k], hex[2 * k + 1]);
                    let pa = buf[at(r + a.0, c + a.1)];
                    let pb = buf[at(r + b.0, c + b.1)];
                    let g0 = buf[at(r, c)][1];
                    let symmetric = a.0 + b.0 == 0 && a.1 + b.1 == 0;
                    for ch in [0, 2] {
                        let v = if symmetric {
                            (2.0 * g0 - pa[1] - pb[1] + pa[ch] + pb[ch]) / 2.0
                        } else {
                            (3.0 * g0 - 2.0 * pa[1] - pb[1] + 2.0 * pa[ch] + pb[ch]) / 3.0
                        };
                        buf[at(r, c)][ch] = v.max(0.0);
                    }
                }
            }
        }

        // 3. Homogeneity in CIELab and weighted selection.
        let lab: Vec<Vec<[f32; 3]>> = rgb
            .par_iter()
            .map(|buf| buf.iter().map(cielab).collect())
            .collect();
        let dirs: [Off; 4] = [(0, 1), (1, 0), (1, 1), (1, -1)];
        let idx = |r: i32, c: i32| (r * tw + c) as usize;
        let mut drv = vec![vec![f32::MAX; n]; NDIR];
        for d in 0..NDIR {
            let (dy, dx) = dirs[d];
            for r in 3..th - 3 {
                for c in 3..tw - 3 {
                    let (l0, lp, lm) = (
                        lab[d][idx(r, c)],
                        lab[d][idx(r + dy, c + dx)],
                        lab[d][idx(r - dy, c - dx)],
                    );
                    let sq = |k: usize| {
                        let v = 2.0 * l0[k] - lp[k] - lm[k];
                        v * v
                    };
                    drv[d][idx(r, c)] = sq(0) + sq(1) + sq(2);
                }
            }
        }
        let mut homo = vec![vec![0u8; n]; NDIR];
        for r in 4..th - 4 {
            for c in 4..tw - 4 {
                let tr = 8.0
                    * (0..NDIR)
                        .map(|d| drv[d][idx(r, c)])
                        .fold(f32::MAX, f32::min);
                for d in 0..NDIR {
                    let mut count = 0u8;
                    for v in -1..=1 {
                        for hh in -1..=1 {
                            count += (drv[d][idx(r + v, c + hh)] <= tr) as u8;
                        }
                    }
                    homo[d][idx(r, c)] = count;
                }
            }
        }

        // Output: the tile interior. Neighbouring tiles overlap by 2·MARGIN, so interiors
        // tile the image without gaps; the last tile in each axis extends to the edge of
        // valid homogeneity data (6 px), and the image border keeps the fallback.
        let last_row = mrow == h - 3;
        let last_col = mcol == w - 3;
        let (r0, c0) = (MARGIN, MARGIN);
        let r1 = if last_row { th - 6 } else { th - MARGIN };
        let c1 = if last_col { tw - 6 } else { tw - MARGIN };
        let out_w = (c1 - c0).max(0);
        let mut out = Vec::with_capacity(((r1 - r0).max(0) * out_w) as usize);
        for r in r0..r1 {
            for c in c0..c1 {
                let mut hm = [0u32; NDIR];
                for (d, s) in hm.iter_mut().enumerate() {
                    for v in -2..=2 {
                        for hh in -2..=2 {
                            *s += homo[d][idx(r + v, c + hh)] as u32;
                        }
                    }
                }
                let max = *hm.iter().max().unwrap();
                let thresh = max - (max >> 3);
                let (mut sum, mut cnt) = ([0f32; 3], 0f32);
                for d in 0..NDIR {
                    if hm[d] >= thresh {
                        let p = rgb[d][idx(r, c)];
                        sum[0] += p[0];
                        sum[1] += p[1];
                        sum[2] += p[2];
                        cnt += 1.0;
                    }
                }
                out.push(sum.map(|v| v / cnt));
            }
        }
        Tile {
            out_top: top + r0,
            out_left: left + c0,
            out_w,
            out,
        }
    }
}

/// Red and blue at a solitary green. Buffer 0 takes the horizontal estimate, buffer 1 the
/// vertical one; the diagonal buffers take whichever is smoother in colour difference.
/// Each buffer reads its own (direction-specific) interpolated greens.
fn solitary_green(
    rgb: &mut [Vec<[f32; 3]>],
    geo: &Geometry,
    at: &impl Fn(i32, i32) -> usize,
    r: i32,
    c: i32,
) {
    // Colour of the horizontal ±1 neighbours; the vertical ±1 neighbours carry the other.
    let hcol = geo.fcol(r, c + 1) as usize;
    for (b, buf) in rgb.iter_mut().enumerate() {
        let g0 = buf[at(r, c)][1];
        let mut est = [[0f32; 2]; 2]; // [H/V][R/B]
        let mut diff = [0f32; 2];
        for (d, step) in [(0i32, 1i32), (1, 0)].into_iter().enumerate() {
            let near = if d == 0 { hcol } else { 2 - hcol };
            for (k, ch) in [near, 2 - near].into_iter().enumerate() {
                let s = k as i32 + 1;
                let p = buf[at(r + step.0 * s, c + step.1 * s)];
                let m = buf[at(r - step.0 * s, c - step.1 * s)];
                let g = 2.0 * g0 - p[1] - m[1];
                est[d][ch / 2] = (g + p[ch] + m[ch]) / 2.0;
                let t = p[1] - m[1] - p[ch] + m[ch];
                diff[d] += t * t + g * g;
            }
        }
        let e = match b {
            0 => est[0],
            1 => est[1],
            _ if diff[0] < diff[1] => est[0],
            _ => est[1],
        };
        let px = &mut buf[at(r, c)];
        px[0] = e[0].max(0.0);
        px[2] = e[1].max(0.0);
    }
}

/// Camera RGB → CIELab (sRGB-like primaries; only relative smoothness matters here).
fn cielab(px: &[f32; 3]) -> [f32; 3] {
    const M: [[f32; 3]; 3] = [
        [0.412_4 / 0.950_47, 0.357_6 / 0.950_47, 0.180_5 / 0.950_47],
        [0.212_6, 0.715_2, 0.072_2],
        [0.019_3 / 1.088_83, 0.119_2 / 1.088_83, 0.950_5 / 1.088_83],
    ];
    let f = |t: f32| {
        if t > 0.008_856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let [r, g, b] = px.map(|v| v.max(0.0));
    let [x, y, z] = M.map(|row| f(row[0] * r + row[1] * g + row[2] * b));
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demosaic::xtrans as simple;

    /// Fujifilm X-T5 layout as reported by rawler for DSCF0385.RAF.
    fn xt5() -> CfaPattern {
        let s = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";
        CfaPattern {
            name: s.into(),
            width: 6,
            height: 6,
            pattern: s
                .bytes()
                .map(|b| match b {
                    b'R' => 0,
                    b'G' => 1,
                    _ => 2,
                })
                .collect(),
        }
    }

    const N: u32 = 192;
    const BORDER: u32 = 12;

    fn mosaic(truth: impl Fn(u32, u32) -> [f32; 3]) -> (Vec<f32>, Vec<[f32; 3]>) {
        let cfa = xt5();
        let mut data = vec![0.0; (N * N) as usize];
        let mut gt = vec![[0.0; 3]; (N * N) as usize];
        for y in 0..N {
            for x in 0..N {
                let t = truth(x, y);
                let i = (y * N + x) as usize;
                gt[i] = t;
                data[i] = t[cfa.color_at(y as usize, x as usize) as usize];
            }
        }
        (data, gt)
    }

    fn run(data: &[f32]) -> (ImageRgbF32, ImageRgbF32) {
        let cfa = xt5();
        let old = simple(data, N, N, &cfa);
        let new = markesteijn(data, N, N, &cfa, old.clone());
        (old, new)
    }

    fn interior() -> impl Iterator<Item = usize> {
        (BORDER..N - BORDER).flat_map(|y| (BORDER..N - BORDER).map(move |x| (y * N + x) as usize))
    }

    fn px(img: &ImageRgbF32, i: usize) -> [f32; 3] {
        [img.r[i], img.g[i], img.b[i]]
    }

    fn mse(img: &ImageRgbF32, gt: &[[f32; 3]]) -> f64 {
        let (mut s, mut n) = (0.0, 0.0);
        for i in interior() {
            let p = px(img, i);
            for c in 0..3 {
                s += ((p[c] - gt[i][c]) as f64).powi(2);
                n += 1.0;
            }
        }
        s / n
    }

    /// Mean |R−G| + |B−G|: zero for a perfect result on a neutral scene.
    fn false_colour(img: &ImageRgbF32) -> f64 {
        let (mut s, mut n) = (0.0, 0.0);
        for i in interior() {
            let [r, g, b] = px(img, i);
            s += ((r - g).abs() + (b - g).abs()) as f64;
            n += 1.0;
        }
        s / n
    }

    #[test]
    fn flat_colour_is_exact() {
        let (data, _) = mosaic(|_, _| [0.4, 0.3, 0.2]);
        let (_, new) = run(&data);
        for i in interior() {
            let p = px(&new, i);
            assert!(
                (p[0] - 0.4).abs() < 1e-5 && (p[1] - 0.3).abs() < 1e-5 && (p[2] - 0.2).abs() < 1e-5,
                "pixel {i}: {p:?}"
            );
        }
    }

    #[test]
    fn output_is_finite_everywhere() {
        let (data, _) = mosaic(|x, y| {
            [
                ((x * 7 + y * 3) % 17) as f32 / 17.0,
                0.5,
                (y % 5) as f32 / 5.0,
            ]
        });
        let (_, new) = run(&data);
        assert!(new
            .r
            .iter()
            .chain(&new.g)
            .chain(&new.b)
            .all(|v| v.is_finite() && *v >= 0.0));
    }

    /// Black/white bars, a diagonal and a ring: every bit of colour is an artefact.
    #[test]
    fn neutral_edges_have_much_less_false_colour() {
        let scene = |x: u32, y: u32| {
            let (fx, fy) = (x as f32, y as f32);
            let bars = (x / 5).is_multiple_of(2) && y < N / 2;
            let diag = y >= N / 2 && ((fx - fy).abs() < 3.0);
            let (dx, dy) = (fx - 140.0, fy - 140.0);
            let ring = (dx * dx + dy * dy).sqrt();
            let ring = y >= N / 2 && (22.0..30.0).contains(&ring);
            let v = if bars || diag || ring { 0.8 } else { 0.08 };
            [v, v, v]
        };
        let (data, gt) = mosaic(scene);
        let (old, new) = run(&data);
        let (fo, fnew) = (false_colour(&old), false_colour(&new));
        let (mo, mn) = (mse(&old, &gt), mse(&new, &gt));
        eprintln!(
            "neutral edges: false colour old {fo:.5} new {fnew:.5}; MSE old {mo:.6} new {mn:.6}"
        );
        assert!(fnew < 0.5 * fo, "false colour {fnew} vs {fo}");
        assert!(mn < mo, "MSE {mn} vs {mo}");
    }

    /// Smooth gradients plus saturated, hard-edged colour patches.
    #[test]
    fn colour_scene_has_lower_error() {
        let scene = |x: u32, y: u32| {
            let (u, v) = (x as f32 / N as f32, y as f32 / N as f32);
            let base = [0.15 + 0.5 * u, 0.2 + 0.3 * v, 0.6 - 0.4 * u];
            let patch = match ((x / 24) % 4, (y / 24) % 3) {
                (0, 0) => Some([0.7, 0.1, 0.1]),
                (2, 1) => Some([0.1, 0.6, 0.15]),
                (1, 2) => Some([0.1, 0.15, 0.7]),
                (3, 0) => Some([0.75, 0.7, 0.1]),
                _ => None,
            };
            patch.unwrap_or(base)
        };
        let (data, gt) = mosaic(scene);
        let (old, new) = run(&data);
        let (mo, mn) = (mse(&old, &gt), mse(&new, &gt));
        eprintln!(
            "colour scene: MSE old {mo:.6} new {mn:.6} (PSNR old {:.1} dB new {:.1} dB)",
            -10.0 * mo.log10(),
            -10.0 * mn.log10()
        );
        assert!(mn < 0.8 * mo, "MSE {mn} vs {mo}");
    }

    /// Across tile seams every pixel must get the edge-aware result. On a scene with no
    /// flat areas the two algorithms never agree bit-for-bit, so any interior pixel equal
    /// to the fallback was never written by a tile.
    #[test]
    fn tiles_cover_the_image_without_gaps() {
        let (w, h) = (1100u32, 700u32);
        let cfa = xt5();
        let mut data = vec![0.0; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32, y as f32);
                let t = [
                    0.4 + 0.25 * (fx * 0.31).sin() * (fy * 0.17).cos(),
                    0.4 + 0.25 * (fx * 0.23 + 1.0).sin() * (fy * 0.29).cos(),
                    0.4 + 0.25 * (fx * 0.19 + 2.0).cos() * (fy * 0.37).sin(),
                ];
                data[(y * w + x) as usize] = t[cfa.color_at(y as usize, x as usize) as usize];
            }
        }
        let old = simple(&data, w, h, &cfa);
        let new = markesteijn(&data, w, h, &cfa, old.clone());
        let b = 12;
        let untouched: Vec<(u32, u32)> = (b..h - b)
            .flat_map(|y| (b..w - b).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let i = (y * w + x) as usize;
                new.r[i] == old.r[i] && new.g[i] == old.g[i] && new.b[i] == old.b[i]
            })
            .collect();
        assert!(
            untouched.is_empty(),
            "{} interior pixels never written, e.g. {:?}",
            untouched.len(),
            &untouched[..untouched.len().min(5)]
        );
    }
}
