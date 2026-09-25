use epikos_core::{ColorSpace, ImageRgbF32};
use rayon::prelude::*;

/// Linear Rec.2020 → linear sRGB (both D65).
const REC2020_TO_SRGB: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_9],
    [-0.124_550_5, 1.132_899_9, -0.008_349_4],
    [-0.018_150_8, -0.100_578_9, 1.118_729_7],
];

/// Values below this pass through linearly; above it they roll off towards 1.0.
const SHOULDER: f32 = 0.8;

/// Display-referred 8-bit sRGB image plus per-channel histograms.
#[derive(Debug, Clone)]
pub struct DisplayImage {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA, 4 bytes per pixel, alpha always 255.
    pub rgba: Vec<u8>,
    /// 256-bin histograms of the displayed R, G and B values.
    pub histogram: [[u32; 256]; 3],
}

/// Baseline view transform: scene-referred linear Rec.2020 → sRGB for the screen.
///
/// Out-of-gamut negatives are clipped and highlights get a smooth shoulder instead of a
/// hard clip. This is a neutral viewing baseline; creative tone curves (PRD Step 7)
/// belong upstream of it.
pub fn to_display_srgb(image: &ImageRgbF32) -> DisplayImage {
    debug_assert_eq!(image.space, ColorSpace::LinearRec2020);
    let lut = encode_lut();
    let rgba: Vec<u8> = (0..image.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let (r, g, b) = (image.r[i], image.g[i], image.b[i]);
            let m = REC2020_TO_SRGB;
            let px = [
                m[0][0] * r + m[0][1] * g + m[0][2] * b,
                m[1][0] * r + m[1][1] * g + m[1][2] * b,
                m[2][0] * r + m[2][1] * g + m[2][2] * b,
            ];
            let [r8, g8, b8] = px.map(|v| encode(&lut, shoulder(v)));
            [r8, g8, b8, 255]
        })
        .collect();

    let mut histogram = [[0u32; 256]; 3];
    for px in rgba.as_chunks::<4>().0 {
        for ch in 0..3 {
            histogram[ch][px[ch] as usize] += 1;
        }
    }
    DisplayImage {
        width: image.width,
        height: image.height,
        rgba,
        histogram,
    }
}

/// Highlight roll-off shared by the preview and export view transforms.
pub(crate) fn shoulder(v: f32) -> f32 {
    if !v.is_finite() || v <= 0.0 {
        0.0
    } else if v <= SHOULDER {
        v
    } else {
        let range = 1.0 - SHOULDER;
        SHOULDER + range * (1.0 - (-(v - SHOULDER) / range).exp())
    }
}

const LUT_SIZE: usize = 4096;

/// sRGB OETF sampled on [0, 1]; 4096 entries keeps 8-bit output exact enough.
fn encode_lut() -> Vec<u8> {
    (0..LUT_SIZE)
        .map(|i| {
            let v = i as f32 / (LUT_SIZE - 1) as f32;
            let e = if v <= 0.003_130_8 {
                12.92 * v
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
            (e * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

fn encode(lut: &[u8], v: f32) -> u8 {
    lut[((v.clamp(0.0, 1.0) * (LUT_SIZE - 1) as f32) + 0.5) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::Pixel;

    fn single(r: f32, g: f32, b: f32) -> DisplayImage {
        let mut img = ImageRgbF32::new(1, 1, ColorSpace::LinearRec2020);
        img.set(0, 0, Pixel::new(r, g, b));
        to_display_srgb(&img)
    }

    #[test]
    fn neutral_grey_stays_neutral_and_encodes_as_srgb() {
        let d = single(0.18, 0.18, 0.18);
        // 18% linear grey is ~118 in sRGB.
        assert!(d.rgba[0].abs_diff(118) <= 1, "{:?}", &d.rgba[..4]);
        assert_eq!(d.rgba[0], d.rgba[1]);
        assert_eq!(d.rgba[1], d.rgba[2]);
        assert_eq!(d.rgba[3], 255);
        assert_eq!(d.histogram[0][d.rgba[0] as usize], 1);
    }

    #[test]
    fn highlights_roll_off_instead_of_clipping_hard() {
        assert!(shoulder(1.0) < 1.0 && shoulder(1.0) > 0.9);
        assert!(shoulder(4.0) > shoulder(2.0));
        assert!(shoulder(100.0) <= 1.0);
        assert_eq!(shoulder(0.5), 0.5);
    }
}
