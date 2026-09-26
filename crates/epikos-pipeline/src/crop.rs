//! PRD Step 1 crop: a rectangle of the upright frame (after straighten and
//! perspective), as fractions of its size. Applied before the look, so vignette, glow
//! and grain are laid out on the framed picture; the model outputs (masks, depth) and
//! light positions are made for the whole upright frame and are cropped to match.

use epikos_core::ImageRgbF32;
use epikos_sidecar::Crop;

/// Pixel bounds `(x0, y0, width, height)` of `crop` in a `w × h` frame.
pub(crate) fn crop_bounds(crop: &Crop, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let c = crop.clamped();
    let x0 = ((c.x * w as f32).round() as u32).min(w - 1);
    let y0 = ((c.y * h as f32).round() as u32).min(h - 1);
    let cw = ((c.width * w as f32).round() as u32).clamp(1, w - x0);
    let ch = ((c.height * h as f32).round() as u32).clamp(1, h - y0);
    (x0, y0, cw, ch)
}

/// Copy the cropped part of a `w × h` plane.
pub(crate) fn crop_plane(src: &[f32], w: u32, h: u32, crop: &Crop) -> (Vec<f32>, u32, u32) {
    let (x0, y0, cw, ch) = crop_bounds(crop, w, h);
    let mut out = Vec::with_capacity((cw * ch) as usize);
    for y in y0..y0 + ch {
        let row = (y * w + x0) as usize;
        out.extend_from_slice(&src[row..row + cw as usize]);
    }
    (out, cw, ch)
}

pub(crate) fn crop_image(rgb: &ImageRgbF32, crop: &Crop) -> ImageRgbF32 {
    let (w, h) = (rgb.width, rgb.height);
    let (r, cw, ch) = crop_plane(&rgb.r, w, h, crop);
    let mut out = ImageRgbF32::new(cw, ch, rgb.space);
    out.r = r;
    out.g = crop_plane(&rgb.g, w, h, crop).0;
    out.b = crop_plane(&rgb.b, w, h, crop).0;
    out
}

/// A position (0–1) in the upright frame, moved into the cropped frame.
pub(crate) fn to_cropped(crop: &Crop, x: f32, y: f32) -> (f32, f32) {
    let c = crop.clamped();
    ((x - c.x) / c.width, (y - c.y) / c.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    #[test]
    fn crop_takes_the_right_pixels() {
        let (w, h) = (10u32, 8u32);
        let mut img = ImageRgbF32::new(w, h, ColorSpace::LinearRec2020);
        for i in 0..img.len() {
            img.g[i] = i as f32;
        }
        let crop = Crop { x: 0.2, y: 0.25, width: 0.5, height: 0.5, aspect: "free".into() };
        let out = crop_image(&img, &crop);
        assert_eq!((out.width, out.height), (5, 4));
        assert_eq!(out.g[0], (2 * w + 2) as f32);
        assert_eq!(out.g[out.len() - 1], (5 * w + 6) as f32);
        let (x, y) = to_cropped(&crop, 0.45, 0.5);
        assert!((x - 0.5).abs() < 1e-5 && (y - 0.5).abs() < 1e-5, "{x} {y}");
        // Out-of-frame rectangles are pulled back inside.
        let wild = Crop { x: 0.9, y: -1.0, width: 0.5, height: 3.0, aspect: "free".into() };
        assert_eq!(crop_bounds(&wild, w, h), (5, 0, 5, 8));
        assert!(!Crop::default().is_active());
    }
}
