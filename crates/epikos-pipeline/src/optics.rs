use epikos_core::{ImageRgbF32, LensGeometry, LensProfile, LensVignetting, Pixel};
use epikos_sidecar::{ChromaticAberration, DistortionCoeffs};
use rayon::prelude::*;

/// Apply a lens profile (distortion, lateral CA, vignetting) in one resampling pass.
/// Runs in the sensor's own frame, before orientation, at any resolution: every model
/// works in coordinates relative to the frame size.
pub fn apply_lens_profile(image: &ImageRgbF32, profile: &LensProfile) -> ImageRgbF32 {
    if profile.is_empty() || image.width < 2 || image.height < 2 {
        return image.clone();
    }
    let (w, h) = (image.width as f64, image.height as f64);
    let warp = profile.geometry.as_ref().map(|g| Warp::new(g, w, h));
    let vignette = profile.vignetting.as_ref().map(|v| Vignette::new(v, w, h));
    let mut out = ImageRgbF32::new(image.width, image.height, image.space);
    let width = image.width as usize;
    out.r
        .par_chunks_mut(width)
        .zip(out.g.par_chunks_mut(width))
        .zip(out.b.par_chunks_mut(width))
        .enumerate()
        .for_each(|(y, ((rr, gg), bb))| {
            for x in 0..width {
                let (px, py) = (x as f64, y as f64);
                for (c, row) in [&mut *rr, &mut *gg, &mut *bb].into_iter().enumerate() {
                    let (sx, sy) = warp.as_ref().map_or((px, py), |wp| wp.source(c, px, py));
                    let mut v = image.sample_channel_bilinear(c as u8, sx as f32, sy as f32);
                    if let Some(vg) = &vignette {
                        v *= vg.gain(sx, sy) as f32;
                    }
                    row[x] = v;
                }
            }
        });
    out
}

/// A geometry model resolved for one frame size.
enum Warp {
    Dng { cx: f64, cy: f64, m: f64, planes: Vec<[f64; 6]> },
    Hugin { cx: f64, cy: f64, px_to_r: f64, g: LensGeometry },
}

impl Warp {
    fn new(g: &LensGeometry, w: f64, h: f64) -> Self {
        match g {
            LensGeometry::Dng { center, planes } => {
                // Pixel centres: normalised 0 is the left edge of pixel 0.
                let (cx, cy) = (center[0] * w - 0.5, center[1] * h - 0.5);
                let m = [(0.0, 0.0), (w - 1.0, 0.0), (0.0, h - 1.0), (w - 1.0, h - 1.0)]
                    .iter()
                    .map(|(x, y)| (x - cx).hypot(y - cy))
                    .fold(0.0, f64::max)
                    .max(1.0);
                Warp::Dng { cx, cy, m, planes: planes.clone() }
            }
            LensGeometry::Hugin { scale, calib_aspect, .. } => {
                let aspect = w.max(h) / w.min(h);
                // Hugin radius 1 = half the short side of the calibration frame; convert
                // through millimetres when crop factor or aspect ratio differ.
                let px_to_r = scale * calib_aspect.hypot(1.0) / aspect.hypot(1.0) / (w.min(h) / 2.0);
                Warp::Hugin { cx: (w - 1.0) / 2.0, cy: (h - 1.0) / 2.0, px_to_r, g: g.clone() }
            }
        }
    }

    /// Where to sample channel `c` for the corrected pixel `(x, y)`.
    fn source(&self, c: usize, x: f64, y: f64) -> (f64, f64) {
        match self {
            Warp::Dng { cx, cy, m, planes } => {
                let k = &planes[c.min(planes.len() - 1)];
                let (dx, dy) = ((x - cx) / m, (y - cy) / m);
                let r2 = dx * dx + dy * dy;
                let f = k[0] + k[1] * r2 + k[2] * r2 * r2 + k[3] * r2 * r2 * r2;
                let tx = k[4] * 2.0 * dx * dy + k[5] * (r2 + 2.0 * dx * dx);
                let ty = k[5] * 2.0 * dx * dy + k[4] * (r2 + 2.0 * dy * dy);
                (cx + m * (f * dx + tx), cy + m * (f * dy + ty))
            }
            Warp::Hugin { cx, cy, px_to_r, g } => {
                let LensGeometry::Hugin { distortion, tca, .. } = g else { unreachable!() };
                let (dx, dy) = (x - cx, y - cy);
                let r = dx.hypot(dy) * px_to_r;
                let mut f = distortion.map_or(1.0, |d| d.factor(r));
                if let (Some(t), 0 | 2) = (tca, c) {
                    f *= t[c / 2].factor(r * f);
                }
                (cx + dx * f, cy + dy * f)
            }
        }
    }
}

struct Vignette {
    cx: f64,
    cy: f64,
    m: f64,
    v: LensVignetting,
}

impl Vignette {
    fn new(v: &LensVignetting, w: f64, h: f64) -> Self {
        let [ccx, ccy] = v.center();
        let (cx, cy) = (ccx * w - 0.5, ccy * h - 0.5);
        let m = match v {
            // Radius 1 at the farthest corner.
            LensVignetting::Dng { .. } => [(0.0, 0.0), (w - 1.0, 0.0), (0.0, h - 1.0), (w - 1.0, h - 1.0)]
                .iter()
                .map(|(x, y)| (x - cx).hypot(y - cy))
                .fold(0.0, f64::max),
            // Radius 1 at the corner (half the diagonal).
            LensVignetting::Pa { .. } => w.hypot(h) / 2.0,
        };
        Self { cx, cy, m: m.max(1.0), v: v.clone() }
    }

    fn gain(&self, x: f64, y: f64) -> f64 {
        self.v.gain((x - self.cx).hypot(y - self.cy) / self.m)
    }
}

/// Lateral chromatic aberration: independently rescale R and B vs G around the optical center.
pub fn correct_chromatic_aberration(image: &ImageRgbF32, ca: &ChromaticAberration) -> ImageRgbF32 {
    if !ca.enabled || (ca.red.abs() < 1e-8 && ca.blue.abs() < 1e-8) {
        return image.clone();
    }
    let mut out = image.clone();
    let cx = (image.width as f32 - 1.0) * 0.5;
    let cy = (image.height as f32 - 1.0) * 0.5;
    let scale_r = 1.0 + ca.red;
    let scale_b = 1.0 + ca.blue;
    for y in 0..image.height {
        for x in 0..image.width {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let xr = cx + dx * scale_r;
            let yr = cy + dy * scale_r;
            let xb = cx + dx * scale_b;
            let yb = cy + dy * scale_b;
            let i = image.index(x, y);
            out.r[i] = image.sample_channel_bilinear(0, xr, yr);
            out.g[i] = image.g[i];
            out.b[i] = image.sample_channel_bilinear(2, xb, yb);
        }
    }
    out
}

/// Inverse Brown–Conrady distortion (optical undistort) with bilinear resampling.
pub fn correct_distortion(image: &ImageRgbF32, d: &DistortionCoeffs) -> ImageRgbF32 {
    if !d.enabled {
        return image.clone();
    }
    let mut out = ImageRgbF32::new(image.width, image.height, image.space);
    let w = image.width as f32;
    let h = image.height as f32;
    let cx = (w - 1.0) * 0.5 + d.cx;
    let cy = (h - 1.0) * 0.5 + d.cy;
    let norm = 0.5 * w.max(h);
    if norm <= 0.0 {
        return image.clone();
    }
    for y in 0..image.height {
        for x in 0..image.width {
            let xd = (x as f32 - cx) / norm;
            let yd = (y as f32 - cy) / norm;
            let (xu, yu) = undistort_point(xd, yd, d);
            let sx = xu * norm + cx;
            let sy = yu * norm + cy;
            out.set(x, y, image.sample_bilinear(sx, sy));
        }
    }
    let _ = Pixel::new(0.0, 0.0, 0.0);
    out
}

fn undistort_point(xd: f32, yd: f32, d: &DistortionCoeffs) -> (f32, f32) {
    let mut x = xd;
    let mut y = yd;
    for _ in 0..8 {
        let r2 = x * x + y * y;
        let r4 = r2 * r2;
        let r6 = r4 * r2;
        let radial = 1.0 + d.k1 * r2 + d.k2 * r4 + d.k3 * r6;
        let x_tangential = 2.0 * d.p1 * x * y + d.p2 * (r2 + 2.0 * x * x);
        let y_tangential = d.p1 * (r2 + 2.0 * y * y) + 2.0 * d.p2 * x * y;
        let pred_x = x * radial + x_tangential;
        let pred_y = y * radial + y_tangential;
        x += xd - pred_x;
        y += yd - pred_y;
    }
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::{ColorSpace, Radial};

    fn grid(w: u32, h: u32) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(w, h, ColorSpace::CameraRgb);
        for y in 0..h {
            for x in 0..w {
                img.set(x, y, Pixel::new(x as f32, y as f32, (x + y) as f32));
            }
        }
        img
    }

    #[test]
    fn identity_profiles_leave_the_image_alone() {
        let img = grid(20, 12);
        for geometry in [
            LensGeometry::Dng { center: [0.5, 0.5], planes: vec![[1.0, 0.0, 0.0, 0.0, 0.0, 0.0]] },
            LensGeometry::Hugin {
                scale: 1.0,
                calib_aspect: 1.5,
                distortion: Some(Radial::PtLens { a: 0.0, b: 0.0, c: 0.0 }),
                tca: Some([Radial::Linear { k: 1.0 }, Radial::Linear { k: 1.0 }]),
            },
        ] {
            let p = LensProfile { source: String::new(), geometry: Some(geometry), vignetting: None };
            let out = apply_lens_profile(&img, &p);
            for i in 0..img.len() {
                assert!((out.r[i] - img.r[i]).abs() < 1e-3 && (out.b[i] - img.b[i]).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn dng_warp_below_one_samples_towards_the_centre() {
        // kr0 < 1: every corrected pixel looks nearer the centre (the frame is enlarged).
        let img = grid(21, 11);
        let p = LensProfile {
            source: String::new(),
            geometry: Some(LensGeometry::Dng { center: [0.5, 0.5], planes: vec![[0.9, 0.0, 0.0, 0.0, 0.0, 0.0]] }),
            vignetting: None,
        };
        let out = apply_lens_profile(&img, &p);
        // Corner (0,0) samples 10% of the way in: x = 10 - 0.9·10.5 ≈ 0.55 … (pixel centres).
        let (x, y) = (out.r[0], out.g[0]);
        assert!(x > 0.5 && x < 1.5 && y > 0.3 && y < 1.0, "{x} {y}");
        // The centre stays put.
        let c = out.index(10, 5);
        assert!((out.r[c] - 10.0).abs() < 1e-3 && (out.g[c] - 5.0).abs() < 1e-3);
    }

    #[test]
    fn tca_moves_red_and_blue_but_not_green() {
        let img = grid(41, 41);
        let p = LensProfile {
            source: String::new(),
            geometry: Some(LensGeometry::Hugin {
                scale: 1.0,
                calib_aspect: 1.0,
                distortion: None,
                tca: Some([Radial::Linear { k: 1.01 }, Radial::Linear { k: 0.99 }]),
            }),
            vignetting: None,
        };
        let out = apply_lens_profile(&img, &p);
        let i = out.index(40, 20);
        assert!(out.r[i] > 40.0 - 1e-3 && out.b[i] < 40.0 + 20.0 - 0.1, "r {} b {}", out.r[i], out.b[i]);
        assert!((out.g[i] - 20.0).abs() < 1e-3);
    }

    #[test]
    fn vignetting_correction_brightens_corners_only() {
        let mut img = ImageRgbF32::new(9, 9, ColorSpace::CameraRgb);
        img.r.fill(0.5);
        img.g.fill(0.5);
        img.b.fill(0.5);
        let p = LensProfile {
            source: String::new(),
            geometry: None,
            vignetting: Some(LensVignetting::Pa { k1: -0.4, k2: 0.0, k3: 0.0 }),
        };
        let out = apply_lens_profile(&img, &p);
        assert!((out.g[out.index(4, 4)] - 0.5).abs() < 1e-4);
        assert!(out.g[0] > 0.6, "{}", out.g[0]);
    }

    #[test]
    fn identity_distortion_is_noop_values() {
        let mut img = ImageRgbF32::new(8, 8, ColorSpace::CameraRgb);
        for y in 0..8 {
            for x in 0..8 {
                img.set(x, y, Pixel::new(x as f32 / 7.0, y as f32 / 7.0, 0.5));
            }
        }
        let d = DistortionCoeffs {
            enabled: true,
            k1: 0.0,
            k2: 0.0,
            k3: 0.0,
            p1: 0.0,
            p2: 0.0,
            cx: 0.0,
            cy: 0.0,
        };
        let out = correct_distortion(&img, &d);
        let a = img.get(3, 4);
        let b = out.get(3, 4);
        assert!((a.r - b.r).abs() < 1e-4);
        assert!((a.g - b.g).abs() < 1e-4);
    }
}
