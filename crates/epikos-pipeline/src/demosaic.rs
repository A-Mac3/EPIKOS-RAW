use epikos_core::{CfaPattern, ColorSpace, ImageRgbF32, MosaicF32, Pixel};

/// Demosaic strategy. Bayer uses Malvar–He–Cutler; X-Trans uses a 6×6 weighted interpolator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemosaicAlgorithm {
    Auto,
    Malvar,
    Bilinear,
    Xtrans,
}

pub fn demosaic(mosaic: &MosaicF32, algorithm: DemosaicAlgorithm) -> ImageRgbF32 {
    if mosaic.samples_per_pixel >= 3 {
        return packed_rgb(mosaic);
    }
    let Some(cfa) = mosaic.cfa.clone() else {
        return monochrome(mosaic);
    };
    let algo = match algorithm {
        DemosaicAlgorithm::Auto if cfa.is_xtrans() => DemosaicAlgorithm::Xtrans,
        DemosaicAlgorithm::Auto => DemosaicAlgorithm::Malvar,
        other => other,
    };
    match algo {
        DemosaicAlgorithm::Xtrans => xtrans(&mosaic.data, mosaic.width, mosaic.height, &cfa),
        DemosaicAlgorithm::Bilinear => bilinear(&mosaic.data, mosaic.width, mosaic.height, &cfa),
        _ => malvar(&mosaic.data, mosaic.width, mosaic.height, &cfa),
    }
}

fn packed_rgb(mosaic: &MosaicF32) -> ImageRgbF32 {
    let mut img = ImageRgbF32::new(mosaic.width, mosaic.height, ColorSpace::CameraRgb);
    let spp = mosaic.samples_per_pixel as usize;
    for i in 0..img.len() {
        let base = i * spp;
        img.r[i] = mosaic.data[base];
        img.g[i] = mosaic.data.get(base + 1).copied().unwrap_or(mosaic.data[base]);
        img.b[i] = mosaic.data.get(base + 2).copied().unwrap_or(mosaic.data[base]);
    }
    img
}

/// Single-channel sensor without a CFA (e.g. Leica Monochrom): replicate into R = G = B.
fn monochrome(mosaic: &MosaicF32) -> ImageRgbF32 {
    let mut img = ImageRgbF32::new(mosaic.width, mosaic.height, ColorSpace::CameraRgb);
    let spp = mosaic.samples_per_pixel.max(1) as usize;
    for i in 0..img.len() {
        let v = mosaic.data[i * spp];
        img.r[i] = v;
        img.g[i] = v;
        img.b[i] = v;
    }
    img
}

fn at(data: &[f32], w: u32, h: u32, x: i32, y: i32) -> f32 {
    let x = x.clamp(0, w as i32 - 1) as u32;
    let y = y.clamp(0, h as i32 - 1) as u32;
    data[(y * w + x) as usize]
}

fn bilinear(data: &[f32], w: u32, h: u32, cfa: &CfaPattern) -> ImageRgbF32 {
    let mut img = ImageRgbF32::new(w, h, ColorSpace::CameraRgb);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            let mut cnt = [0.0f32; 3];
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let xx = x as i32 + dx;
                    let yy = y as i32 + dy;
                    if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 {
                        continue;
                    }
                    let ch = cfa.color_at(yy as usize, xx as usize) as usize;
                    if ch > 2 {
                        continue;
                    }
                    acc[ch] += at(data, w, h, xx, yy);
                    cnt[ch] += 1.0;
                }
            }
            let known = cfa.color_at(y as usize, x as usize) as usize;
            let v = at(data, w, h, x as i32, y as i32);
            let rgb = |c: usize| {
                if c == known {
                    v
                } else if cnt[c] > 0.0 {
                    acc[c] / cnt[c]
                } else {
                    v
                }
            };
            img.set(x, y, Pixel::new(rgb(0), rgb(1), rgb(2)));
        }
    }
    img
}

/// Malvar–He–Cutler 5×5 linear demosaic (IEEE ICASSP 2004), Bayer only.
///
/// Kernel weights are the published ones (all divided by 8). The 2-pixel border
/// keeps the bilinear estimate.
fn malvar(data: &[f32], w: u32, h: u32, cfa: &CfaPattern) -> ImageRgbF32 {
    if !cfa.is_bayer() {
        return bilinear(data, w, h, cfa);
    }
    let mut img = bilinear(data, w, h, cfa);
    let (iw, ih) = (w as i32, h as i32);
    for y in 2..ih - 2 {
        for x in 2..iw - 2 {
            let c = cfa.color_at(y as usize, x as usize);
            let s = |dx: i32, dy: i32| at(data, w, h, x + dx, y + dy);
            let c0 = s(0, 0);
            let axial1 = s(-1, 0) + s(1, 0) + s(0, -1) + s(0, 1);
            let axial2 = s(-2, 0) + s(2, 0) + s(0, -2) + s(0, 2);
            let diag1 = s(-1, -1) + s(1, -1) + s(-1, 1) + s(1, 1);

            // Green at a red/blue site.
            let g_at_rb = (4.0 * c0 + 2.0 * axial1 - axial2) / 8.0;
            // Opposite chroma (blue at red, red at blue).
            let opp_at_rb = (6.0 * c0 + 2.0 * diag1 - 1.5 * axial2) / 8.0;
            // Chroma at a green site whose same-colour neighbours are horizontal / vertical.
            let at_g_horiz = (5.0 * c0 + 4.0 * (s(-1, 0) + s(1, 0)) - (s(-2, 0) + s(2, 0))
                + 0.5 * (s(0, -2) + s(0, 2))
                - diag1)
                / 8.0;
            let at_g_vert = (5.0 * c0 + 4.0 * (s(0, -1) + s(0, 1)) - (s(0, -2) + s(0, 2))
                + 0.5 * (s(-2, 0) + s(2, 0))
                - diag1)
                / 8.0;

            let (r, g, b) = match c {
                0 => (c0, g_at_rb, opp_at_rb),
                2 => (opp_at_rb, g_at_rb, c0),
                _ => {
                    let red_is_horizontal = cfa.color_at(y as usize, x as usize + 1) == 0;
                    if red_is_horizontal {
                        (at_g_horiz, c0, at_g_vert)
                    } else {
                        (at_g_vert, c0, at_g_horiz)
                    }
                }
            };
            img.set(x as u32, y as u32, Pixel::new(r.max(0.0), g.max(0.0), b.max(0.0)));
        }
    }
    img
}

fn xtrans(data: &[f32], w: u32, h: u32, cfa: &CfaPattern) -> ImageRgbF32 {
    let mut img = ImageRgbF32::new(w, h, ColorSpace::CameraRgb);
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0.0f32; 3];
            let mut wsum = [0.0f32; 3];
            for dy in -3i32..=3 {
                for dx in -3i32..=3 {
                    let xx = x as i32 + dx;
                    let yy = y as i32 + dy;
                    if xx < 0 || yy < 0 || xx >= w as i32 || yy >= h as i32 {
                        continue;
                    }
                    let ch = cfa.color_at(yy as usize, xx as usize) as usize;
                    if ch > 2 {
                        continue;
                    }
                    let dist2 = (dx * dx + dy * dy) as f32;
                    let weight = 1.0 / (1.0 + dist2);
                    acc[ch] += at(data, w, h, xx, yy) * weight;
                    wsum[ch] += weight;
                }
            }
            let known = cfa.color_at(y as usize, x as usize) as usize;
            let v = at(data, w, h, x as i32, y as i32);
            let ch = |c: usize| {
                if c == known {
                    v
                } else if wsum[c] > 0.0 {
                    acc[c] / wsum[c]
                } else {
                    v
                }
            };
            img.set(x, y, Pixel::new(ch(0), ch(1), ch(2)));
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::CfaPattern;

    fn make_rggb(w: u32, h: u32, f: impl Fn(u32, u32, u8) -> f32) -> MosaicF32 {
        make_bayer(CfaPattern::rggb(), w, h, f)
    }

    fn make_bayer(cfa: CfaPattern, w: u32, h: u32, f: impl Fn(u32, u32, u8) -> f32) -> MosaicF32 {
        let mut data = vec![0.0; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                data[(y * w + x) as usize] = f(x, y, cfa.color_at(y as usize, x as usize));
            }
        }
        MosaicF32 {
            width: w,
            height: h,
            data,
            samples_per_pixel: 1,
            cfa: Some(cfa),
        }
    }

    #[test]
    fn constant_field_survives_malvar() {
        let mosaic = make_rggb(16, 16, |_, _, ch| match ch {
            0 => 0.4,
            2 => 0.2,
            _ => 0.3,
        });
        let rgb = demosaic(&mosaic, DemosaicAlgorithm::Malvar);
        let p = rgb.get(8, 8);
        assert!((p.r - 0.4).abs() < 0.08, "r={}", p.r);
        assert!((p.g - 0.3).abs() < 0.08, "g={}", p.g);
        assert!((p.b - 0.2).abs() < 0.08, "b={}", p.b);
    }

    fn bayer(name: &str, pattern: [u8; 4]) -> CfaPattern {
        CfaPattern {
            name: name.into(),
            width: 2,
            height: 2,
            pattern: pattern.to_vec(),
        }
    }

    /// A flat colour must reproduce exactly at every site type, for every Bayer phase.
    #[test]
    fn flat_field_is_exact_at_every_site_for_all_bayer_phases() {
        let phases = [
            bayer("RGGB", [0, 1, 1, 2]),
            bayer("BGGR", [2, 1, 1, 0]),
            bayer("GRBG", [1, 0, 2, 1]),
            bayer("GBRG", [1, 2, 0, 1]),
        ];
        for cfa in phases {
            let name = cfa.name.clone();
            let mosaic = make_bayer(cfa, 16, 16, |_, _, ch| match ch {
                0 => 0.4,
                2 => 0.2,
                _ => 0.3,
            });
            let rgb = demosaic(&mosaic, DemosaicAlgorithm::Malvar);
            for y in 0..16 {
                for x in 0..16 {
                    let p = rgb.get(x, y);
                    assert!(
                        (p.r - 0.4).abs() < 1e-5 && (p.g - 0.3).abs() < 1e-5 && (p.b - 0.2).abs() < 1e-5,
                        "{name} ({x},{y}) = {p:?}"
                    );
                }
            }
        }
    }

    /// MHC is exact on a linear luminance ramp in the interior (no zipper overshoot).
    #[test]
    fn grey_ramp_has_no_overshoot() {
        let mosaic = make_rggb(24, 24, |x, _, _| 0.02 * x as f32);
        let rgb = demosaic(&mosaic, DemosaicAlgorithm::Malvar);
        for y in 2..22 {
            for x in 2..22 {
                let p = rgb.get(x, y);
                let v = 0.02 * x as f32;
                assert!(
                    (p.r - v).abs() < 1e-5 && (p.g - v).abs() < 1e-5 && (p.b - v).abs() < 1e-5,
                    "({x},{y}) = {p:?}, expected {v}"
                );
            }
        }
    }

    #[test]
    fn monochrome_sensor_is_replicated_not_demosaiced() {
        let mosaic = MosaicF32 {
            width: 4,
            height: 4,
            data: (0..16).map(|i| i as f32 / 16.0).collect(),
            samples_per_pixel: 1,
            cfa: None,
        };
        let rgb = demosaic(&mosaic, DemosaicAlgorithm::Auto);
        for i in 0..16 {
            let v = i as f32 / 16.0;
            assert_eq!((rgb.r[i], rgb.g[i], rgb.b[i]), (v, v, v));
        }
    }
}
