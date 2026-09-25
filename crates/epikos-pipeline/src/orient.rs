use epikos_core::{ImageRgbF32, Orientation};
use rayon::prelude::*;

/// Rotate/flip into upright display orientation. Lossless: every output pixel is a copy
/// of exactly one input pixel.
pub fn apply_orientation(image: ImageRgbF32, orientation: Orientation) -> ImageRgbF32 {
    if orientation == Orientation::Normal {
        return image;
    }
    let (w, h) = (image.width, image.height);
    let (ow, oh) = if orientation.swaps_axes() {
        (h, w)
    } else {
        (w, h)
    };
    let src: Vec<usize> = (0..ow * oh)
        .into_par_iter()
        .map(|i| {
            let (sx, sy) = orientation.source_of(i % ow, i / ow, w, h);
            image.index(sx, sy)
        })
        .collect();
    let mut out = ImageRgbF32::new(ow, oh, image.space);
    for (dst, &s) in src.iter().enumerate() {
        out.r[dst] = image.r[s];
        out.g[dst] = image.g[s];
        out.b[dst] = image.b[s];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::{ColorSpace, Pixel};

    /// 3×2 image whose red channel encodes the source position: r = 10·y + x.
    fn sample() -> ImageRgbF32 {
        let mut img = ImageRgbF32::new(3, 2, ColorSpace::LinearRec2020);
        for y in 0..2 {
            for x in 0..3 {
                img.set(x, y, Pixel::new((10 * y + x) as f32, 0.0, 0.0));
            }
        }
        img
    }

    fn rows(img: &ImageRgbF32) -> Vec<Vec<u32>> {
        (0..img.height)
            .map(|y| (0..img.width).map(|x| img.get(x, y).r as u32).collect())
            .collect()
    }

    #[test]
    fn every_exif_orientation_maps_pixels_correctly() {
        // Source:  0  1  2
        //         10 11 12
        let cases = [
            (1, vec![vec![0, 1, 2], vec![10, 11, 12]]),
            (2, vec![vec![2, 1, 0], vec![12, 11, 10]]),
            (3, vec![vec![12, 11, 10], vec![2, 1, 0]]),
            (4, vec![vec![10, 11, 12], vec![0, 1, 2]]),
            (5, vec![vec![0, 10], vec![1, 11], vec![2, 12]]),
            (6, vec![vec![10, 0], vec![11, 1], vec![12, 2]]),
            (7, vec![vec![12, 2], vec![11, 1], vec![10, 0]]),
            (8, vec![vec![2, 12], vec![1, 11], vec![0, 10]]),
        ];
        for (tag, expected) in cases {
            let out = apply_orientation(sample(), Orientation::from_exif(tag));
            assert_eq!(rows(&out), expected, "EXIF orientation {tag}");
        }
    }
}
