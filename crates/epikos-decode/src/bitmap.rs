//! JPEG and PNG as develop sources.
//!
//! The 8/16-bit sRGB pixels are linearised and presented to the pipeline as a
//! linear-RGB "sensor" whose camera space is linear sRGB (its XYZ → camera matrix is
//! the sRGB one and its as-shot white balance is neutral). White balance then works
//! as a relative shift and every later step runs unchanged. What a bitmap can't give
//! back: highlights clipped in the file stay clipped, and the camera's tone curve is
//! already baked in.

use std::io::Cursor;
use std::path::Path;

use epikos_core::{
    CameraFormat, CaptureMetadata, Error, MosaicF32, Orientation, Result, SensorLayout, SensorProfile,
};
use image::{DynamicImage, ImageDecoder, ImageReader};
use rawler::decoders::RawMetadata;
use rawler::exif::Exif;
use rawler::formats::tiff::reader::TiffReader;
use rawler::formats::tiff::GenericTiffReader;
use rawler::tags::TiffCommonTag;

use crate::convert::DecodedRaw;

/// XYZ (D65) → linear sRGB, i.e. the "camera" matrix of a bitmap.
const XYZ_TO_SRGB: [[f32; 3]; 3] = [
    [3.240_97, -1.537_383, -0.498_611],
    [-0.969_244, 1.875_968, 0.041_555],
    [0.055_63, -0.203_977, 1.056_972],
];

pub(crate) fn decode(path: &Path, bytes: &[u8], digest: String) -> Result<DecodedRaw> {
    let format = CameraFormat::from_extension(path.extension().and_then(|e| e.to_str()).unwrap_or(""));
    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| Error::Decode(format!("{}: {e}", path.display())))?
        .into_decoder()
        .map_err(image_err)?;
    let orientation = decoder.orientation().ok().map(orientation_from).unwrap_or_default();
    let exif = decoder.exif_metadata().ok().flatten();
    let img = DynamicImage::from_decoder(decoder).map_err(image_err)?;
    let (width, height) = (img.width(), img.height());
    // 16 bits per channel keeps a 16-bit PNG's precision; an 8-bit file loses nothing.
    let rgb = img.into_rgb16();
    let lut: Vec<f32> = (0..=u16::MAX).map(|v| srgb_to_linear(v as f32 / 65535.0)).collect();
    let data: Vec<f32> = rgb.as_raw().iter().map(|&v| lut[v as usize]).collect();

    let metadata = exif.as_deref().map(capture_metadata).unwrap_or_default();
    let profile = SensorProfile {
        format,
        make: metadata.make.clone(),
        model: metadata.model.clone(),
        clean_make: metadata.make.clone(),
        clean_model: metadata.model.clone(),
        width,
        height,
        bits_per_sample: 16,
        samples_per_pixel: 3,
        layout: SensorLayout::LinearRgb,
        as_shot_wb: [1.0, 1.0, 1.0, f32::NAN],
        xyz_to_cam: XYZ_TO_SRGB,
        orientation,
    };
    Ok(DecodedRaw {
        profile,
        mosaic: MosaicF32 { width, height, data, samples_per_pixel: 3, cfa: None },
        source_sha256: digest,
        source_path: path.to_string_lossy().into_owned(),
        metadata,
        lens_profile: None,
    })
}

/// The file itself, downscaled, upright: bitmaps are their own preview.
pub(crate) fn thumbnail(path: &Path, max_side: u32) -> Result<image::RgbImage> {
    let bytes = std::fs::read(path)?;
    let mut decoder = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| Error::Decode(format!("{}: {e}", path.display())))?
        .into_decoder()
        .map_err(image_err)?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(image_err)?;
    if img.width().max(img.height()) > max_side {
        img = img.thumbnail(max_side, max_side);
    }
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img.to_rgb8())
}

/// Capture metadata from a JPEG/PNG's EXIF block, without decoding pixels.
pub(crate) fn read_metadata(path: &Path) -> Result<CaptureMetadata> {
    let bytes = std::fs::read(path)?;
    let mut decoder = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| Error::Decode(format!("{}: {e}", path.display())))?
        .into_decoder()
        .map_err(image_err)?;
    Ok(decoder.exif_metadata().ok().flatten().as_deref().map(capture_metadata).unwrap_or_default())
}

/// EXIF (a TIFF structure) → capture metadata; anything unreadable is left empty.
fn capture_metadata(tiff: &[u8]) -> CaptureMetadata {
    let Ok(reader) = GenericTiffReader::new_with_buffer(tiff, 0, 0, None) else {
        return CaptureMetadata::default();
    };
    // (`root_ifd` panics on an empty chain.)
    let Some(root) = reader.chains().first() else {
        return CaptureMetadata::default();
    };
    let text = |tag: TiffCommonTag| {
        root.get_entry(tag)
            .and_then(|e| e.value.as_string().cloned())
            .map(|s| s.trim().trim_end_matches('\0').to_string())
            .unwrap_or_default()
    };
    let (make, model) = (text(TiffCommonTag::Make), text(TiffCommonTag::Model));
    match Exif::new(root) {
        Ok(exif) => crate::metadata::convert(
            RawMetadata { exif, make: make.clone(), model: model.clone(), lens: None, unique_image_id: None, rating: None },
            &make,
            &model,
        ),
        Err(_) => CaptureMetadata { make, model, ..Default::default() },
    }
}

fn orientation_from(o: image::metadata::Orientation) -> Orientation {
    use image::metadata::Orientation as O;
    match o {
        O::NoTransforms => Orientation::Normal,
        O::Rotate90 => Orientation::Rotate90,
        O::Rotate180 => Orientation::Rotate180,
        O::Rotate270 => Orientation::Rotate270,
        O::FlipHorizontal => Orientation::FlipHorizontal,
        O::FlipVertical => Orientation::FlipVertical,
        O::Rotate90FlipH => Orientation::Transpose,
        O::Rotate270FlipH => Orientation::Transverse,
    }
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn image_err(e: image::ImageError) -> Error {
    Error::Decode(format!("image: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32, px: [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb(px));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn png_becomes_linear_rgb_with_srgb_primaries() {
        let d = decode(Path::new("x.png"), &png_bytes(4, 2, [255, 128, 0]), "0".into()).unwrap();
        assert_eq!((d.mosaic.width, d.mosaic.height, d.mosaic.samples_per_pixel), (4, 2, 3));
        assert_eq!(d.profile.layout, SensorLayout::LinearRgb);
        assert_eq!(d.profile.format, CameraFormat::Png);
        let [r, g, b] = [d.mosaic.data[0], d.mosaic.data[1], d.mosaic.data[2]];
        assert!((r - 1.0).abs() < 1e-5 && (g - 0.2158).abs() < 1e-3 && b.abs() < 1e-6, "{r} {g} {b}");
    }

    #[test]
    fn sensor_matrix_is_the_inverse_of_srgb_to_xyz() {
        // sRGB → XYZ (D65), rows; XYZ_TO_SRGB × it ≈ identity.
        let m = [[0.412_456, 0.357_576, 0.180_437], [0.212_673, 0.715_152, 0.072_175], [0.019_334, 0.119_192, 0.950_304]];
        let product: Vec<f32> = XYZ_TO_SRGB
            .iter()
            .flat_map(|row| (0..3).map(move |j| (0..3).map(|k| row[k] * m[k][j]).sum::<f32>()))
            .collect();
        for (n, v) in product.iter().enumerate() {
            let want = if n % 4 == 0 { 1.0 } else { 0.0 };
            assert!((v - want).abs() < 1e-3, "{n} {v}");
        }
    }
}
