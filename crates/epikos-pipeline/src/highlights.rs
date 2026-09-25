use epikos_core::{ImageRgbF32, Pixel};
use rayon::prelude::*;

const CLIP: f32 = 0.98;

/// Reconstruct clipped channels from unclipped ratios (sensor-level highlight recovery).
///
/// For each pixel with one or two channels at/above `CLIP`, the missing energy is
/// estimated from a 5×5 neighborhood of unclipped pixels that share a similar hue.
pub fn recover_highlights(image: &mut ImageRgbF32) {
    let w = image.width;
    let h = image.height;
    let n = image.len();
    let r = image.r.clone();
    let g = image.g.clone();
    let b = image.b.clone();

    let recovered: Vec<Pixel> = (0..n)
        .into_par_iter()
        .map(|i| {
            let x = (i as u32) % w;
            let y = (i as u32) / w;
            let px = Pixel::new(r[i], g[i], b[i]);
            reconstruct(w, h, &r, &g, &b, x, y, px)
        })
        .collect();

    for (i, px) in recovered.into_iter().enumerate() {
        image.r[i] = px.r;
        image.g[i] = px.g;
        image.b[i] = px.b;
    }
}

fn reconstruct(
    w: u32,
    h: u32,
    r: &[f32],
    g: &[f32],
    b: &[f32],
    x: u32,
    y: u32,
    px: Pixel,
) -> Pixel {
    let clip_r = px.r >= CLIP;
    let clip_g = px.g >= CLIP;
    let clip_b = px.b >= CLIP;
    let nclip = clip_r as u8 + clip_g as u8 + clip_b as u8;
    if nclip == 0 {
        return px;
    }
    if nclip == 3 {
        return px;
    }

    let mut sum_r = 0.0;
    let mut sum_g = 0.0;
    let mut sum_b = 0.0;
    let mut count = 0.0;
    let x0 = x.saturating_sub(2);
    let y0 = y.saturating_sub(2);
    let x1 = (x + 2).min(w - 1);
    let y1 = (y + 2).min(h - 1);
    for yy in y0..=y1 {
        for xx in x0..=x1 {
            if xx == x && yy == y {
                continue;
            }
            let i = (yy * w + xx) as usize;
            let nr = r[i];
            let ng = g[i];
            let nb = b[i];
            if nr >= CLIP || ng >= CLIP || nb >= CLIP {
                continue;
            }
            if nr + ng + nb < 1e-5 {
                continue;
            }
            sum_r += nr;
            sum_g += ng;
            sum_b += nb;
            count += 1.0;
        }
    }
    if count < 1.0 {
        return px;
    }
    let hr = (sum_r / count).max(1e-6);
    let hg = (sum_g / count).max(1e-6);
    let hb = (sum_b / count).max(1e-6);

    let mut out = px;
    if clip_r && !clip_g {
        out.r = px.g * (hr / hg);
    } else if clip_r && !clip_b {
        out.r = px.b * (hr / hb);
    }
    if clip_g && !clip_r {
        out.g = px.r * (hg / hr);
    } else if clip_g && !clip_b {
        out.g = px.b * (hg / hb);
    }
    if clip_b && !clip_g {
        out.b = px.g * (hb / hg);
    } else if clip_b && !clip_r {
        out.b = px.r * (hb / hr);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    #[test]
    fn recovers_clipped_red_from_neighbors() {
        let mut img = ImageRgbF32::new(5, 5, ColorSpace::CameraRgb);
        for y in 0..5 {
            for x in 0..5 {
                img.set(x, y, Pixel::new(0.5, 0.25, 0.125));
            }
        }
        img.set(2, 2, Pixel::new(1.0, 0.25, 0.125));
        recover_highlights(&mut img);
        let p = img.get(2, 2);
        assert!(p.r > 0.4 && p.r < 0.7, "recovered r={}", p.r);
        assert!((p.g - 0.25).abs() < 1e-4);
    }
}
