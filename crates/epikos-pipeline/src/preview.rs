use epikos_core::{CfaPattern, ColorSpace, ImageRgbF32, MosaicF32};
use rayon::prelude::*;

/// Sensor samples at or above this value are treated as clipped (matches highlight recovery).
const CLIP: f32 = 0.98;

/// Smallest block that contains every CFA colour: 2×2 for Bayer, 3×3 for X-Trans, 1×1
/// for monochrome / linear RGB.
pub fn base_block(mosaic: &MosaicF32) -> u32 {
    match &mosaic.cfa {
        Some(cfa) if mosaic.samples_per_pixel == 1 && cfa.is_xtrans() => 3,
        Some(_) if mosaic.samples_per_pixel == 1 => 2,
        _ => 1,
    }
}

/// Integer block size that brings the mosaic within `max_w × max_h` (at least the CFA block).
pub fn block_for_size(mosaic: &MosaicF32, max_w: u32, max_h: u32) -> u32 {
    let base = base_block(mosaic);
    let need = (mosaic.width.div_ceil(max_w.max(1))).max(mosaic.height.div_ceil(max_h.max(1)));
    need.div_ceil(base).max(1) * base
}

/// Downsampled camera-RGB image made by averaging each `block × block` cell of the
/// mosaic ("superpixel" demosaic). Far cheaper than a full demosaic and free of
/// interpolation artefacts, so it's used for the interactive preview.
///
/// `block` must be a multiple of [`base_block`] so every cell contains R, G and B. A
/// channel with any clipped sample in the cell stays at 1.0, so highlight recovery sees
/// clipping in the preview exactly as it does at full resolution.
pub fn bin_mosaic(mosaic: &MosaicF32, block: u32) -> ImageRgbF32 {
    let block = block.max(1);
    let out_w = (mosaic.width / block).max(1);
    let out_h = (mosaic.height / block).max(1);
    let mut img = ImageRgbF32::new(out_w, out_h, ColorSpace::CameraRgb);
    let cfa = mosaic
        .cfa
        .as_ref()
        .filter(|_| mosaic.samples_per_pixel == 1);
    let spp = mosaic.samples_per_pixel.max(1) as usize;

    let pixels: Vec<[f32; 3]> = (0..out_w * out_h)
        .into_par_iter()
        .map(|i| {
            let (ox, oy) = (i % out_w, i / out_w);
            cell(mosaic, cfa, spp, ox * block, oy * block, block)
        })
        .collect();
    for (i, [r, g, b]) in pixels.into_iter().enumerate() {
        img.r[i] = r;
        img.g[i] = g;
        img.b[i] = b;
    }
    img
}

fn cell(
    mosaic: &MosaicF32,
    cfa: Option<&CfaPattern>,
    spp: usize,
    x0: u32,
    y0: u32,
    block: u32,
) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let mut count = [0u32; 3];
    let mut clipped = [false; 3];
    let x1 = (x0 + block).min(mosaic.width);
    let y1 = (y0 + block).min(mosaic.height);
    for y in y0..y1 {
        for x in x0..x1 {
            let base = ((y * mosaic.width + x) as usize) * spp;
            match cfa {
                Some(cfa) => {
                    let ch = cfa.color_at(y as usize, x as usize) as usize;
                    if ch > 2 {
                        continue;
                    }
                    let v = mosaic.data[base];
                    sum[ch] += v;
                    count[ch] += 1;
                    clipped[ch] |= v >= CLIP;
                }
                None => {
                    for ch in 0..3 {
                        let v = mosaic.data[base + ch.min(spp - 1)];
                        sum[ch] += v;
                        count[ch] += 1;
                        clipped[ch] |= v >= CLIP;
                    }
                }
            }
        }
    }
    std::array::from_fn(|ch| {
        if clipped[ch] {
            1.0
        } else if count[ch] > 0 {
            sum[ch] / count[ch] as f32
        } else {
            0.0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rggb(w: u32, h: u32, r: f32, g: f32, b: f32) -> MosaicF32 {
        let cfa = CfaPattern::rggb();
        let data = (0..w * h)
            .map(|i| match cfa.color_at((i / w) as usize, (i % w) as usize) {
                0 => r,
                2 => b,
                _ => g,
            })
            .collect();
        MosaicF32 {
            width: w,
            height: h,
            data,
            samples_per_pixel: 1,
            cfa: Some(cfa),
        }
    }

    #[test]
    fn superpixel_recovers_flat_colour() {
        let img = bin_mosaic(&rggb(8, 8, 0.4, 0.3, 0.2), 2);
        assert_eq!((img.width, img.height), (4, 4));
        let p = img.get(1, 2);
        assert_eq!((p.r, p.g, p.b), (0.4, 0.3, 0.2));
    }

    #[test]
    fn block_size_fits_target_and_respects_cfa() {
        let m = rggb(6000, 4000, 0.1, 0.1, 0.1);
        let block = block_for_size(&m, 2000, 2000);
        assert_eq!(block % 2, 0);
        assert!(6000 / block <= 2000 && 4000 / block <= 2000);
        assert_eq!(block_for_size(&m, 10_000, 10_000), 2);
    }

    #[test]
    fn clipped_samples_stay_clipped() {
        let mut m = rggb(4, 4, 0.5, 0.3, 0.2);
        m.data[0] = 1.0; // one red sample in the first cell is clipped
        let img = bin_mosaic(&m, 2);
        assert_eq!(img.get(0, 0).r, 1.0);
        assert_eq!(img.get(1, 0).r, 0.5);
    }
}
