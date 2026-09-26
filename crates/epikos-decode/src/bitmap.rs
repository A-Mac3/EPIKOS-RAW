//! JPEG, PNG and TIFF as develop sources.
//!
//! The 8/16-bit pixels are taken as sRGB (an embedded ICC profile is not read, so a
//! Display P3 or ProPhoto TIFF shows its colours slightly off), linearised and presented to the pipeline as a
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
    if format == CameraFormat::Tiff {
        let t = read_tiff(bytes)?;
        let data = if t.linear { t.rgb } else { t.rgb.iter().map(|&v| srgb_to_linear(v)).collect() };
        let metadata = capture_metadata(bytes);
        return Ok(bitmap_raw(path, format, t.width, t.height, data, t.orientation, metadata, digest));
    }
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
    Ok(bitmap_raw(path, format, width, height, data, orientation, metadata, digest))
}

/// A decoded bitmap as a linear-RGB "sensor" (see the module docs).
#[allow(clippy::too_many_arguments)]
fn bitmap_raw(
    path: &Path,
    format: CameraFormat,
    width: u32,
    height: u32,
    data: Vec<f32>,
    orientation: Orientation,
    metadata: CaptureMetadata,
    digest: String,
) -> DecodedRaw {
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
    DecodedRaw {
        profile,
        mosaic: MosaicF32 { width, height, data, samples_per_pixel: 3, cfa: None },
        source_sha256: digest,
        source_path: path.to_string_lossy().into_owned(),
        metadata,
        lens_profile: None,
    }
}

struct TiffPixels {
    width: u32,
    height: u32,
    /// Interleaved RGB, 0–1 for integer samples.
    rgb: Vec<f32>,
    /// Float samples are taken as linear light; integer ones as sRGB-encoded.
    linear: bool,
    orientation: Orientation,
}

/// TIFF through the `tiff` crate, which (unlike `image`) accepts extra channels, such
/// as the named alpha channels of a Photoshop or EPIKOS export: the first three samples
/// are the colour (grey is replicated), the rest are ignored.
fn read_tiff(bytes: &[u8]) -> Result<TiffPixels> {
    use tiff::decoder::{Decoder, DecodingResult, Limits};
    use tiff::tags::Tag;
    use tiff::ColorType;
    let tiff_err = |e: tiff::TiffError| Error::Decode(format!("tiff: {e}"));
    let mut d = Decoder::new(Cursor::new(bytes)).map_err(tiff_err)?.with_limits(Limits::unlimited());
    let (width, height) = d.dimensions().map_err(tiff_err)?;
    let ct = d.colortype().map_err(tiff_err)?;
    let grey = match ct {
        ColorType::Gray(_) | ColorType::GrayA(_) => true,
        ColorType::RGB(_) | ColorType::RGBA(_) => false,
        ColorType::Multiband { num_samples, .. } if num_samples >= 3 => false,
        other => return Err(Error::Decode(format!("tiff: {other:?} images are not supported (RGB or grey only)"))),
    };
    let orientation = d.get_tag_u32(Tag::Orientation).map(|o| Orientation::from_exif(o as u16)).unwrap_or_default();
    // Unspecified extra channels (Photoshop / EPIKOS masks) with horizontal differencing:
    // `tiff` undoes the differencing over the colour samples only and scrambles the
    // image, so those strips are read here.
    let all_samples = d.get_tag_u32(Tag::SamplesPerPixel).unwrap_or(0) as usize;
    let predictor = d.get_tag_u32(Tag::Predictor).unwrap_or(1);
    if all_samples > ct.num_samples() as usize && predictor == 2 {
        return read_tiff_strips(&mut d, bytes, width, height, all_samples, grey, orientation);
    }
    let (samples, linear): (Vec<f32>, bool) = match d.read_image().map_err(tiff_err)? {
        DecodingResult::U8(v) => (v.iter().map(|&x| x as f32 / 255.0).collect(), false),
        DecodingResult::U16(v) => (v.iter().map(|&x| x as f32 / 65535.0).collect(), false),
        DecodingResult::F32(v) => (v, true),
        DecodingResult::F64(v) => (v.iter().map(|&x| x as f32).collect(), true),
        _ => return Err(Error::Decode("tiff: unsupported sample format".into())),
    };
    let n = (width * height) as usize;
    // Samples per pixel as decoded: `tiff` returns the colour (and alpha) samples and
    // drops unspecified extra channels.
    let spp = samples.len().checked_div(n).unwrap_or(0);
    if spp < if grey { 1 } else { 3 } {
        return Err(Error::Decode("tiff: truncated image data".into()));
    }
    let mut rgb = Vec::with_capacity(n * 3);
    for px in samples.chunks_exact(spp).take(n) {
        if grey {
            rgb.extend([px[0]; 3]);
        } else {
            rgb.extend_from_slice(&px[..3]);
        }
    }
    Ok(TiffPixels { width, height, rgb, linear, orientation })
}

/// Strip-by-strip TIFF read for integer images with extra channels and horizontal
/// differencing: decompress (none, LZW, Deflate), undo the differencing over whole
/// pixels, keep the colour.
fn read_tiff_strips<R: std::io::Read + std::io::Seek>(
    d: &mut tiff::decoder::Decoder<R>,
    bytes: &[u8],
    width: u32,
    height: u32,
    spp: usize,
    grey: bool,
    orientation: Orientation,
) -> Result<TiffPixels> {
    use tiff::tags::Tag;
    let tiff_err = |e: tiff::TiffError| Error::Decode(format!("tiff: {e}"));
    let bits = d.get_tag_u32_vec(Tag::BitsPerSample).map_err(tiff_err)?;
    let bytes_per = match bits.first() {
        Some(8) => 1,
        Some(16) => 2,
        other => return Err(Error::Decode(format!("tiff: {other:?}-bit samples with extra channels are not supported"))),
    };
    if d.get_tag_u32(Tag::SampleFormat).unwrap_or(1) != 1 {
        return Err(Error::Decode("tiff: only integer samples are supported with extra channels".into()));
    }
    if d.find_tag(Tag::TileWidth).map_err(tiff_err)?.is_some() {
        return Err(Error::Decode("tiff: tiled images with extra channels are not supported".into()));
    }
    let offsets = d.get_tag_u64_vec(Tag::StripOffsets).map_err(tiff_err)?;
    let counts = d.get_tag_u64_vec(Tag::StripByteCounts).map_err(tiff_err)?;
    let compression = d.get_tag_u32(Tag::Compression).unwrap_or(1);
    let little = bytes.starts_with(b"II");
    let row_len = width as usize * spp * bytes_per;
    let mut raw = Vec::with_capacity(row_len * height as usize);
    for (&o, &n) in offsets.iter().zip(&counts) {
        let strip = bytes
            .get(o as usize..(o + n) as usize)
            .ok_or_else(|| Error::Decode("tiff: strip outside the file".into()))?;
        match compression {
            1 => raw.extend_from_slice(strip),
            5 => raw.extend(
                weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
                    .decode(strip)
                    .map_err(|e| Error::Decode(format!("tiff LZW: {e}")))?,
            ),
            8 | 32946 => {
                use std::io::Read;
                flate2::read::ZlibDecoder::new(strip).read_to_end(&mut raw)?;
            }
            c => return Err(Error::Decode(format!("tiff: compression {c} is not supported with extra channels"))),
        }
    }
    if raw.len() < row_len * height as usize {
        return Err(Error::Decode("tiff: truncated image data".into()));
    }
    let n = (width * height) as usize;
    let mut rgb = Vec::with_capacity(n * 3);
    for row in raw.chunks_exact(row_len).take(height as usize) {
        let mut values: Vec<u32> = if bytes_per == 1 {
            row.iter().map(|&b| b as u32).collect()
        } else {
            row.as_chunks::<2>()
                .0
                .iter()
                .map(|&b| if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) } as u32)
                .collect()
        };
        let modulo = if bytes_per == 1 { 0xFF } else { 0xFFFF };
        for i in spp..values.len() {
            values[i] = (values[i] + values[i - spp]) & modulo;
        }
        let max = modulo as f32;
        for px in values.chunks_exact(spp) {
            if grey {
                rgb.extend([px[0] as f32 / max; 3]);
            } else {
                rgb.extend(px[..3].iter().map(|&v| v as f32 / max));
            }
        }
    }
    Ok(TiffPixels { width, height, rgb, linear: false, orientation })
}

/// The file itself, downscaled, upright: bitmaps are their own preview.
pub(crate) fn thumbnail(path: &Path, max_side: u32) -> Result<image::RgbImage> {
    let bytes = std::fs::read(path)?;
    if CameraFormat::from_extension(path.extension().and_then(|e| e.to_str()).unwrap_or("")) == CameraFormat::Tiff {
        let t = read_tiff(&bytes)?;
        let encode = |v: f32| {
            let v = if t.linear { linear_to_srgb(v) } else { v };
            (v.clamp(0.0, 1.0) * 255.0).round() as u8
        };
        let img = image::RgbImage::from_raw(t.width, t.height, t.rgb.iter().map(|&v| encode(v)).collect())
            .ok_or_else(|| Error::Decode("tiff: size mismatch".into()))?;
        let img = DynamicImage::ImageRgb8(img);
        let img = if img.width().max(img.height()) > max_side { img.thumbnail(max_side, max_side) } else { img };
        return Ok(orient(img, t.orientation).to_rgb8());
    }
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

/// Capture metadata from a JPEG/PNG's EXIF block (a TIFF's own tags), without
/// decoding pixels.
pub(crate) fn read_metadata(path: &Path) -> Result<CaptureMetadata> {
    let bytes = std::fs::read(path)?;
    if CameraFormat::from_extension(path.extension().and_then(|e| e.to_str()).unwrap_or("")) == CameraFormat::Tiff {
        return Ok(capture_metadata(&bytes));
    }
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

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

fn orient(img: DynamicImage, o: Orientation) -> DynamicImage {
    match o {
        Orientation::Normal => img,
        Orientation::FlipHorizontal => img.fliph(),
        Orientation::Rotate180 => img.rotate180(),
        Orientation::FlipVertical => img.flipv(),
        Orientation::Transpose => img.rotate90().fliph(),
        Orientation::Rotate90 => img.rotate90(),
        Orientation::Transverse => img.rotate270().fliph(),
        Orientation::Rotate270 => img.rotate270(),
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

    /// A 16-bit RGB TIFF with two extra (unspecified) channels, like a masked export.
    fn tiff_with_extra_channels(w: u32, h: u32) -> Vec<u8> {
        use tiff::encoder::{colortype::ColorType as Ct, TiffEncoder};
        struct Rgb16x5;
        impl Ct for Rgb16x5 {
            type Inner = u16;
            const TIFF_VALUE: tiff::tags::PhotometricInterpretation = tiff::tags::PhotometricInterpretation::RGB;
            const BITS_PER_SAMPLE: &'static [u16] = &[16; 5];
            const SAMPLE_FORMAT: &'static [tiff::tags::SampleFormat] = &[tiff::tags::SampleFormat::Uint; 5];
            fn horizontal_predict(row: &[u16], result: &mut Vec<u16>) {
                result.extend_from_slice(row);
            }
        }
        let px: Vec<u16> = (0..w * h).flat_map(|_| [65535, 32768, 0, 1234, 4321]).collect();
        let mut out = Cursor::new(Vec::new());
        let mut enc = TiffEncoder::new(&mut out).unwrap();
        let mut img = enc.new_image::<Rgb16x5>(w, h).unwrap();
        img.encoder().write_tag(tiff::tags::Tag::ExtraSamples, &[0u16, 0][..]).unwrap();
        img.write_data(&px).unwrap();
        out.into_inner()
    }

    #[test]
    fn tiff_with_extra_channels_keeps_the_colour() {
        let d = decode(Path::new("x.tif"), &tiff_with_extra_channels(4, 3), "0".into()).unwrap();
        assert_eq!((d.mosaic.width, d.mosaic.height, d.profile.format), (4, 3, CameraFormat::Tiff));
        let [r, g, b] = [d.mosaic.data[0], d.mosaic.data[1], d.mosaic.data[2]];
        // 32768 / 65535 ≈ 0.5 sRGB ≈ 0.214 linear.
        assert!((r - 1.0).abs() < 1e-5 && (g - 0.2140).abs() < 1e-3 && b.abs() < 1e-6, "{r} {g} {b}");
        assert_eq!(d.mosaic.data.len(), 4 * 3 * 3);
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
