//! Industry-standard 3D LUTs (`.cube`, Adobe / Resolve format), applied after Step 7.
//!
//! A `.cube` LUT maps display-referred RGB to display-referred RGB, so the image is
//! taken to what the screen would show (linear Rec.2020 → sRGB primaries → the display
//! shoulder → sRGB encoding), looked up with trilinear interpolation, and brought
//! back to linear Rec.2020. `amount` blends the result with the original in linear
//! light. 1D shaper LUTs are not supported.

use epikos_core::{Error, ImageRgbF32, Result};

use crate::display::shoulder;
use rayon::prelude::*;

/// Linear Rec.2020 ↔ linear sRGB (D65).
const REC2020_TO_SRGB: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_9],
    [-0.124_550_5, 1.132_899_9, -0.008_349_4],
    [-0.018_150_8, -0.100_578_9, 1.118_729_7],
];
const SRGB_TO_REC2020: [[f32; 3]; 3] = [
    [0.627_404, 0.329_283, 0.043_313],
    [0.069_097, 0.919_54, 0.011_362],
    [0.016_391, 0.088_013, 0.895_595],
];
const MAX_SIZE: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct Lut3d {
    /// `TITLE`, if the file has one.
    pub title: Option<String>,
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// `size³` entries, red changing fastest (the `.cube` order).
    pub data: Vec<[f32; 3]>,
}

impl Lut3d {
    /// Parse a `.cube` file.
    pub fn parse_cube(text: &str) -> Result<Self> {
        let bad = |msg: String| Error::Decode(format!(".cube: {msg}"));
        let mut lut = Lut3d { title: None, size: 0, domain_min: [0.0; 3], domain_max: [1.0; 3], data: Vec::new() };
        let triple = |rest: &str| -> Option<[f32; 3]> {
            let v: Vec<f32> = rest.split_whitespace().map(|x| x.parse().ok()).collect::<Option<_>>()?;
            v.try_into().ok()
        };
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (key, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            match key.to_ascii_uppercase().as_str() {
                "TITLE" => lut.title = Some(rest.trim().trim_matches('"').to_string()),
                "LUT_3D_SIZE" => {
                    let size: usize = rest.trim().parse().map_err(|_| bad(format!("line {}: bad LUT_3D_SIZE", n + 1)))?;
                    if !(2..=MAX_SIZE).contains(&size) {
                        return Err(bad(format!("LUT_3D_SIZE {size} is outside 2–{MAX_SIZE}")));
                    }
                    lut.size = size;
                }
                "LUT_1D_SIZE" => return Err(bad("1D LUTs are not supported; use a 3D LUT".into())),
                "DOMAIN_MIN" => lut.domain_min = triple(rest).ok_or_else(|| bad(format!("line {}: bad DOMAIN_MIN", n + 1)))?,
                "DOMAIN_MAX" => lut.domain_max = triple(rest).ok_or_else(|| bad(format!("line {}: bad DOMAIN_MAX", n + 1)))?,
                "LUT_3D_INPUT_RANGE" | "LUT_1D_INPUT_RANGE" => {
                    let v: Vec<f32> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    if let [lo, hi] = v[..] {
                        lut.domain_min = [lo; 3];
                        lut.domain_max = [hi; 3];
                    }
                }
                _ => match triple(line) {
                    Some(rgb) => lut.data.push(rgb),
                    // Other keywords (Resolve's, comments without #) are ignored.
                    None if key.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) => {}
                    None => return Err(bad(format!("line {}: expected three numbers", n + 1))),
                },
            }
        }
        if lut.size == 0 {
            return Err(bad("no LUT_3D_SIZE".into()));
        }
        let want = lut.size.pow(3);
        if lut.data.len() != want {
            return Err(bad(format!("{} entries for a {}³ LUT (expected {want})", lut.data.len(), lut.size)));
        }
        if (0..3).any(|c| lut.domain_max[c] <= lut.domain_min[c]) {
            return Err(bad("DOMAIN_MAX must be above DOMAIN_MIN".into()));
        }
        Ok(lut)
    }

    /// Look up one display-referred colour (trilinear).
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let last = (n - 1) as f32;
        let pos: [f32; 3] = std::array::from_fn(|c| {
            let t = (rgb[c] - self.domain_min[c]) / (self.domain_max[c] - self.domain_min[c]);
            t.clamp(0.0, 1.0) * last
        });
        let i0: [usize; 3] = pos.map(|p| (p.floor() as usize).min(n - 2));
        let f: [f32; 3] = std::array::from_fn(|c| pos[c] - i0[c] as f32);
        let at = |r: usize, g: usize, b: usize| self.data[r + n * (g + n * b)];
        let mut out = [0.0f32; 3];
        for (dr, wr) in [(0, 1.0 - f[0]), (1, f[0])] {
            for (dg, wg) in [(0, 1.0 - f[1]), (1, f[1])] {
                for (db, wb) in [(0, 1.0 - f[2]), (1, f[2])] {
                    let w = wr * wg * wb;
                    if w == 0.0 {
                        continue;
                    }
                    let v = at(i0[0] + dr, i0[1] + dg, i0[2] + db);
                    for c in 0..3 {
                        out[c] += w * v[c];
                    }
                }
            }
        }
        out
    }
}

pub(crate) fn apply_lut(rgb: &mut ImageRgbF32, lut: &Lut3d, amount: f32) {
    let k = amount.clamp(0.0, 1.0);
    if k <= 0.0 {
        return;
    }
    rgb.r
        .par_iter_mut()
        .zip(rgb.g.par_iter_mut())
        .zip(rgb.b.par_iter_mut())
        .for_each(|((r, g), b)| {
            let lin = [*r, *g, *b];
            let srgb = mul(&REC2020_TO_SRGB, lin).map(|v| encode(shoulder(v.max(0.0))));
            let out = lut.sample(srgb).map(|v| decode(v.clamp(0.0, 1.0)));
            let looked = mul(&SRGB_TO_REC2020, out);
            [*r, *g, *b] = std::array::from_fn(|c| lin[c] + k * (looked[c] - lin[c]));
        });
}

fn mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn encode(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

fn decode(v: f32) -> f32 {
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    /// An `n`³ identity LUT, or one that swaps red and blue.
    fn cube(n: usize, swap: bool) -> String {
        let mut s = format!("TITLE \"test\"\n# comment\nLUT_3D_SIZE {n}\n");
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let f = |v: usize| v as f32 / (n - 1) as f32;
                    let (r, g, b) = (f(r), f(g), f(b));
                    let (x, y, z) = if swap { (b, g, r) } else { (r, g, b) };
                    s += &format!("{x:.6} {y:.6} {z:.6}\n");
                }
            }
        }
        s
    }

    #[test]
    fn parses_and_samples_exactly_on_the_grid_and_between() {
        let lut = Lut3d::parse_cube(&cube(17, true)).unwrap();
        assert_eq!((lut.size, lut.title.as_deref()), (17, Some("test")));
        let v = lut.sample([0.25, 0.5, 0.75]);
        assert!((v[0] - 0.75).abs() < 1e-5 && (v[2] - 0.25).abs() < 1e-5, "{v:?}");
        let v = lut.sample([0.3, 0.1, 0.9]);
        assert!((v[0] - 0.9).abs() < 1e-4 && (v[1] - 0.1).abs() < 1e-4, "{v:?}");
    }

    #[test]
    fn identity_lut_leaves_displayable_colours_alone() {
        let lut = Lut3d::parse_cube(&cube(33, false)).unwrap();
        let mut img = ImageRgbF32::new(3, 1, ColorSpace::LinearRec2020);
        for (i, v) in [0.05f32, 0.18, 0.5].iter().enumerate() {
            (img.r[i], img.g[i], img.b[i]) = (*v, v * 0.8, v * 0.6);
        }
        let before = img.clone();
        apply_lut(&mut img, &lut, 1.0);
        for i in 0..3 {
            assert!((img.g[i] - before.g[i]).abs() < 2e-3 * before.g[i].max(0.05), "{} {}", img.g[i], before.g[i]);
        }
    }

    #[test]
    fn amount_blends_and_bad_files_are_refused() {
        let lut = Lut3d::parse_cube(&cube(9, true)).unwrap();
        let mut full = ImageRgbF32::new(1, 1, ColorSpace::LinearRec2020);
        (full.r[0], full.g[0], full.b[0]) = (0.4, 0.2, 0.05);
        let mut half = full.clone();
        let orig = full.clone();
        apply_lut(&mut full, &lut, 1.0);
        apply_lut(&mut half, &lut, 0.5);
        assert!(((half.r[0] - orig.r[0]) - 0.5 * (full.r[0] - orig.r[0])).abs() < 1e-5);
        assert!(Lut3d::parse_cube("LUT_1D_SIZE 4\n0 0 0\n").is_err());
        assert!(Lut3d::parse_cube("LUT_3D_SIZE 2\n0 0 0\n1 1 1\n").is_err());
        assert!(Lut3d::parse_cube("0 0 0\n").is_err());
    }
}
