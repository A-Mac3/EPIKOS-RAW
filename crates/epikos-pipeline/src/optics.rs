use epikos_core::{ImageRgbF32, Pixel};
use epikos_sidecar::{ChromaticAberration, DistortionCoeffs};

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
    use epikos_core::ColorSpace;

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
