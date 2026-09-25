use epikos_core::{ImageRgbF32, Pixel};
use rayon::prelude::*;

/// Sensor samples at or above this are clipped.
pub(crate) const CLIP: f32 = 0.98;
/// Above this the pixel starts blending towards neutral, so fully clipped areas don't
/// meet their surroundings at a hard, coloured edge.
const KNEE: f32 = 0.9 * CLIP;

/// White-balance-aware highlight handling, in camera RGB before WB is applied.
///
/// A clipped pixel carries no colour information in its clipped channels, and white
/// balance then scales those channels by different gains; a fully clipped (1, 1, 1)
/// becomes e.g. (2.0, 1.0, 1.9): magenta. Per pixel:
///
/// * one or two channels clipped and `reconstruct` on: rebuild them from unclipped
///   neighbours of similar hue;
/// * otherwise clipped (all three, no usable neighbours, or `reconstruct` off): make it
///   neutral *after* WB, at the brightest channel's level (the lowest neutral that is
///   consistent with every clipped channel being at least at saturation);
/// * unclipped but above [`KNEE`]: blend smoothly towards that neutral.
///
/// `gains` are the green-normalised WB multipliers that will be applied next.
pub fn recover_highlights(image: &mut ImageRgbF32, gains: [f32; 3], reconstruct: bool) {
    let (w, h) = (image.width, image.height);
    let (r, g, b) = (image.r.clone(), image.g.clone(), image.b.clone());

    // Write straight into the image planes: a temporary Vec<Pixel> would add another
    // full-size copy (≈ 560 MB for a 47 MP frame).
    let (out_r, out_g, out_b) = (&mut image.r, &mut image.g, &mut image.b);
    out_r
        .par_iter_mut()
        .zip(out_g.par_iter_mut())
        .zip(out_b.par_iter_mut())
        .enumerate()
        .for_each(|(i, ((or, og), ob))| {
            let px = handle_pixel(i, w, h, &r, &g, &b, gains, reconstruct);
            (*or, *og, *ob) = (px.r, px.g, px.b);
        });
}

#[allow(clippy::too_many_arguments)]
fn handle_pixel(
    i: usize,
    w: u32,
    h: u32,
    r: &[f32],
    g: &[f32],
    b: &[f32],
    gains: [f32; 3],
    reconstruct: bool,
) -> Pixel {
    let px = Pixel::new(r[i], g[i], b[i]);
    let clipped = [px.r >= CLIP, px.g >= CLIP, px.b >= CLIP];
    let nclip = clipped.iter().filter(|c| **c).count();
    if nclip > 0 {
        if reconstruct && nclip < 3 {
            let (x, y) = ((i as u32) % w, (i as u32) / w);
            if let Some(p) = reconstruct_from_neighbours(w, h, r, g, b, x, y, px, clipped) {
                return p;
            }
        }
        return neutral_after_wb(px, gains);
    }
    let peak = px.r.max(px.g).max(px.b);
    if peak <= KNEE {
        return px;
    }
    let t = smoothstep((peak - KNEE) / (CLIP - KNEE));
    let n = neutral_after_wb(px, gains);
    Pixel::new(lerp(px.r, n.r, t), lerp(px.g, n.g, t), lerp(px.b, n.b, t))
}

/// Camera values that become `(v, v, v)` after WB, with `v` = brightest WB'd channel.
fn neutral_after_wb(px: Pixel, gains: [f32; 3]) -> Pixel {
    let v = (px.r * gains[0]).max(px.g * gains[1]).max(px.b * gains[2]);
    Pixel::new(v / gains[0], v / gains[1], v / gains[2])
}

/// Estimate clipped channels from the mean colour of unclipped pixels in a 5×5 window.
/// `None` when no usable neighbour exists.
#[allow(clippy::too_many_arguments)]
fn reconstruct_from_neighbours(
    w: u32,
    h: u32,
    r: &[f32],
    g: &[f32],
    b: &[f32],
    x: u32,
    y: u32,
    px: Pixel,
    [clip_r, clip_g, clip_b]: [bool; 3],
) -> Option<Pixel> {
    let (mut sum_r, mut sum_g, mut sum_b, mut count) = (0.0, 0.0, 0.0, 0u32);
    for yy in y.saturating_sub(2)..=(y + 2).min(h - 1) {
        for xx in x.saturating_sub(2)..=(x + 2).min(w - 1) {
            if xx == x && yy == y {
                continue;
            }
            let i = (yy * w + xx) as usize;
            let (nr, ng, nb) = (r[i], g[i], b[i]);
            if nr >= CLIP || ng >= CLIP || nb >= CLIP || nr + ng + nb < 1e-5 {
                continue;
            }
            sum_r += nr;
            sum_g += ng;
            sum_b += nb;
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let c = count as f32;
    let (hr, hg, hb) = (
        (sum_r / c).max(1e-6),
        (sum_g / c).max(1e-6),
        (sum_b / c).max(1e-6),
    );

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
    // A clipped channel is at least at saturation; never let reconstruction darken it.
    out.r = if clip_r { out.r.max(px.r) } else { out.r };
    out.g = if clip_g { out.g.max(px.g) } else { out.g };
    out.b = if clip_b { out.b.max(px.b) } else { out.b };
    Some(out)
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::ColorSpace;

    const FUJI_GAINS: [f32; 3] = [1.818, 1.0, 1.944];

    fn filled(w: u32, h: u32, px: Pixel) -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(w, h, ColorSpace::CameraRgb);
        for y in 0..h {
            for x in 0..w {
                img.set(x, y, px);
            }
        }
        img
    }

    fn after_wb(px: Pixel, g: [f32; 3]) -> [f32; 3] {
        [px.r * g[0], px.g * g[1], px.b * g[2]]
    }

    fn assert_neutral(v: [f32; 3]) {
        assert!(
            (v[0] - v[1]).abs() < 1e-4 && (v[2] - v[1]).abs() < 1e-4,
            "not neutral: {v:?}"
        );
    }

    #[test]
    fn recovers_clipped_red_from_neighbors() {
        // Neighbours have r:g = 1.6, so the clipped red is really 0.7 × 1.6 = 1.12.
        let mut img = filled(5, 5, Pixel::new(0.8, 0.5, 0.3));
        img.set(2, 2, Pixel::new(1.0, 0.7, 0.42));
        recover_highlights(&mut img, [1.0; 3], true);
        let p = img.get(2, 2);
        assert!((p.r - 1.12).abs() < 1e-3, "recovered r={}", p.r);
        assert!((p.g - 0.7).abs() < 1e-6 && (p.b - 0.42).abs() < 1e-6);
    }

    #[test]
    fn reconstruction_never_darkens_a_clipped_channel() {
        let mut img = filled(5, 5, Pixel::new(0.5, 0.25, 0.125));
        img.set(2, 2, Pixel::new(1.0, 0.25, 0.125));
        recover_highlights(&mut img, [1.0; 3], true);
        assert!(img.get(2, 2).r >= 1.0);
    }

    #[test]
    fn fully_clipped_pixel_is_white_not_magenta_after_wb() {
        let mut img = filled(3, 3, Pixel::new(1.0, 1.0, 1.0));
        recover_highlights(&mut img, FUJI_GAINS, true);
        let v = after_wb(img.get(1, 1), FUJI_GAINS);
        assert_neutral(v);
        // Never darker than the brightest saturated channel.
        assert!(v[1] >= 1.944 - 1e-4, "{v:?}");
    }

    #[test]
    fn partly_clipped_without_neighbours_goes_neutral() {
        // Red and blue clipped everywhere: nothing to reconstruct from.
        let mut img = filled(5, 5, Pixel::new(1.0, 0.7, 1.0));
        recover_highlights(&mut img, FUJI_GAINS, true);
        assert_neutral(after_wb(img.get(2, 2), FUJI_GAINS));
    }

    #[test]
    fn reconstruction_off_still_prevents_magenta() {
        let mut img = filled(5, 5, Pixel::new(0.3, 0.3, 0.3));
        img.set(2, 2, Pixel::new(1.0, 1.0, 1.0));
        recover_highlights(&mut img, FUJI_GAINS, false);
        assert_neutral(after_wb(img.get(2, 2), FUJI_GAINS));
        assert_eq!(img.get(0, 0).r, 0.3, "midtones untouched");
    }

    #[test]
    fn approach_to_clipping_is_smooth() {
        let colour = |peak: f32| {
            let mut img = filled(1, 1, Pixel::new(peak, peak * 0.6, peak * 0.4));
            recover_highlights(&mut img, FUJI_GAINS, true);
            let v = after_wb(img.get(0, 0), FUJI_GAINS);
            (v[0] - v[2]).abs() / v[1] // chroma proxy
        };
        let (mid, knee, near) = (colour(0.5), colour(KNEE), colour(CLIP - 1e-3));
        assert!((mid - knee).abs() < 1e-5, "no change below the knee");
        assert!(
            near < 0.02 * knee.max(1e-3) + 1e-3,
            "nearly neutral at clip: {near}"
        );
        // Monotonic desaturation across the knee.
        let mut last = knee;
        for k in 1..10 {
            let c = colour(KNEE + (CLIP - KNEE) * k as f32 / 10.0);
            assert!(c <= last + 1e-6, "chroma rose at step {k}");
            last = c;
        }
    }
}
