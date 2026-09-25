//! Enhanced DNG handoff: a linear (demosaiced) DNG of the scene-referred image with
//! the AI masks as DNG 1.6 semantic masks and the depth map as a DNG 1.5 depth map.
//!
//! Like Adobe's own "Enhanced" DNGs, the pixels are demosaiced, denoised and lens-
//! corrected but not creatively graded, so Lightroom / Camera Raw / Capture One keep
//! full raw-style latitude. The data is linear Rec.2020 (white-balanced, D65), so
//! ColorMatrix1 is simply XYZ → Rec.2020 under D65 and AsShotNeutral is 1, 1, 1.
//! Values are stored at a quarter of scene level (two stops of highlight headroom)
//! and `BaselineExposure` +2 restores the brightness.
//!
//! Written with rawler's DNG writer (the same library that decodes our input), with
//! the main image losslessly JPEG-compressed.

use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use epikos_core::{CaptureMetadata, Error, ImageRgbF32, Result};
use epikos_pipeline::to_display_srgb;
use rawler::dng::writer::DngWriter;
use rawler::dng::DngCompression;
use rawler::formats::tiff::{Rational, SRational, Value};
use rawler::imgop::xyz::Illuminant;
use rawler::tags::{DngTag, ExifTag, TiffCommonTag};

/// Two stops of headroom above scene white.
const HEADROOM_STOPS: i32 = 2;
/// NewSubFileType values (DNG 1.5 / 1.6).
const SUBFILE_MAIN: u32 = 0;
const SUBFILE_PREVIEW: u32 = 1;
const SUBFILE_DEPTH: u32 = 8;
const SUBFILE_SEMANTIC_MASK: u32 = 0x10004;
/// PhotometricInterpretation values (DNG 1.5 / 1.6).
const PHOTOMETRIC_DEPTH: u16 = 51177;
const PHOTOMETRIC_SEMANTIC_MASK: u16 = 52527;
/// Masks are stored at half the main image's resolution, as Apple ProRAW does.
const MASK_SCALE: u32 = 2;

/// XYZ (D65) → linear Rec.2020.
const XYZ_TO_REC2020: [[f64; 3]; 3] = [
    [1.716_651, -0.355_671, -0.253_366],
    [-0.666_684, 1.616_481, 0.015_769],
    [0.017_640, -0.042_771, 0.942_103],
];

pub(crate) fn write_dng(
    path: &Path,
    rgb: &ImageRgbF32,
    channels: &[(String, Vec<u16>)],
    meta: &CaptureMetadata,
    _gps: bool,
) -> Result<()> {
    let (w, h) = (rgb.width as usize, rgb.height as usize);
    let scale = 0.25f32.powi(HEADROOM_STOPS / 2) * 65535.0; // 1 / 2^HEADROOM_STOPS
    let data: Vec<u16> = (0..w * h)
        .flat_map(|i| [rgb.r[i], rgb.g[i], rgb.b[i]].map(|v| (v.max(0.0) * scale).round().min(65535.0) as u16))
        .collect();

    // Preview and thumbnail show the scene as the reader will before its own edits.
    let display = to_display_srgb(rgb);
    let rgb8: Vec<u8> = display.rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
    let preview = image::RgbImage::from_raw(display.width, display.height, rgb8)
        .map(image::DynamicImage::ImageRgb8)
        .ok_or_else(|| Error::Decode("dng: preview size mismatch".into()))?;

    let file = BufWriter::new(File::create(path)?);
    let mut dng = DngWriter::new(file, [1, 4, 0, 0]).map_err(dng_err)?;
    {
        let root = dng.root_ifd_mut();
        root.add_tag(TiffCommonTag::Make, meta.make.as_str());
        root.add_tag(TiffCommonTag::Model, meta.model.as_str());
        root.add_tag(TiffCommonTag::Software, "EPIKOS RAW");
        root.add_tag(TiffCommonTag::Orientation, 1u16);
        // Not the camera's own colour space, so no camera-specific profile applies.
        root.add_tag(DngTag::UniqueCameraModel, "EPIKOS RAW Linear Rec.2020");
        root.add_tag(DngTag::BaselineExposure, SRational::new(HEADROOM_STOPS, 1));
        root.add_tag(DngTag::ProfileName, "EPIKOS Linear Rec.2020");
    }
    let matrix: Vec<SRational> = XYZ_TO_REC2020
        .iter()
        .flatten()
        .map(|v| SRational::new((v * 10_000.0).round() as i32, 10_000))
        .collect();
    dng.color_matrix(1, Illuminant::D65, matrix.as_slice());
    write_exif(&mut dng, meta);

    dng.thumbnail(&preview).map_err(dng_err)?;
    {
        let mut main = dng.subframe(SUBFILE_MAIN);
        main.rgb_image_u16(&data, w, h, DngCompression::Lossless, 1).map_err(dng_err)?;
        main.finalize().map_err(dng_err)?;
    }
    drop(data);
    {
        let mut sub = dng.subframe(SUBFILE_PREVIEW);
        sub.preview(&preview, 0.9).map_err(dng_err)?;
        sub.finalize().map_err(dng_err)?;
    }
    // After the image's own colour tags: rgb_image_u16 writes a neutral AsShotNeutral.
    dng.as_shot_neutral([Rational::new(1, 1), Rational::new(1, 1), Rational::new(1, 1)]);

    let (mw, mh) = ((w as u32).div_ceil(MASK_SCALE), (h as u32).div_ceil(MASK_SCALE));
    for (i, (name, mask)) in channels.iter().enumerate() {
        let small = epikos_core::resize_plane(
            &mask.iter().map(|&v| v as f32 / 65535.0).collect::<Vec<_>>(),
            w as u32,
            h as u32,
            mw,
            mh,
        );
        if name == "Depth" {
            let bytes: Vec<u8> = small
                .iter()
                .flat_map(|v| ((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes())
                .collect();
            let offset = dng.dng.write_data(&bytes).map_err(dng_err)?;
            {
                let root = dng.root_ifd_mut();
                // Relative inverse depth from a monocular model: no metric units.
                root.add_tag(DngTag::DepthFormat, 2u16);
                root.add_tag(DngTag::DepthNear, Rational::new(1, 1));
                root.add_tag(DngTag::DepthFar, Rational::new(100, 1));
                root.add_tag(DngTag::DepthUnits, 0u16);
                root.add_tag(DngTag::DepthMeasureType, 0u16);
            }
            let mut sub = dng.subframe(SUBFILE_DEPTH);
            plane_tags(sub.ifd_mut(), mw, mh, 16, PHOTOMETRIC_DEPTH, offset, bytes.len() as u32);
            sub.finalize().map_err(dng_err)?;
        } else {
            let bytes: Vec<u8> = small.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
            let offset = dng.dng.write_data(&bytes).map_err(dng_err)?;
            let mut sub = dng.subframe(SUBFILE_SEMANTIC_MASK);
            let ifd = sub.ifd_mut();
            plane_tags(ifd, mw, mh, 8, PHOTOMETRIC_SEMANTIC_MASK, offset, bytes.len() as u32);
            ifd.add_tag(DngTag::SemanticName, name.as_str());
            ifd.add_tag(DngTag::SemanticInstanceID, i.to_string().as_str());
            ifd.add_tag(DngTag::MaskSubArea, [0u32, 0, h as u32, w as u32]);
            sub.finalize().map_err(dng_err)?;
        }
    }
    dng.close().map_err(dng_err)
}

fn plane_tags(
    ifd: &mut rawler::formats::tiff::DirectoryWriter,
    w: u32,
    h: u32,
    bits: u16,
    photometric: u16,
    offset: u32,
    len: u32,
) {
    ifd.add_tag(TiffCommonTag::ImageWidth, w);
    ifd.add_tag(TiffCommonTag::ImageLength, h);
    ifd.add_tag(TiffCommonTag::BitsPerSample, bits);
    ifd.add_tag(TiffCommonTag::SamplesPerPixel, 1u16);
    ifd.add_tag(TiffCommonTag::Compression, 1u16);
    ifd.add_tag(TiffCommonTag::PhotometricInt, photometric);
    ifd.add_tag(TiffCommonTag::RowsPerStrip, h);
    ifd.add_value(TiffCommonTag::StripOffsets, Value::Long(vec![offset]));
    ifd.add_tag(TiffCommonTag::StripByteCounts, len);
}

fn write_exif<W: std::io::Write + std::io::Seek>(dng: &mut DngWriter<W>, m: &CaptureMetadata) {
    let exif = dng.exif_ifd_mut();
    let r = |(n, d): (u32, u32)| Rational::new(n, d);
    if let Some(v) = m.exposure_time {
        exif.add_tag(ExifTag::ExposureTime, r(v));
    }
    if let Some(v) = m.f_number {
        exif.add_tag(ExifTag::FNumber, r(v));
    }
    if let Some(v) = m.focal_length {
        exif.add_tag(ExifTag::FocalLength, r(v));
    }
    if let Some(iso) = m.iso {
        exif.add_tag(ExifTag::ISOSpeedRatings, iso.min(u16::MAX as u32) as u16);
    }
    if let Some(v) = &m.date_time_original {
        exif.add_tag(ExifTag::DateTimeOriginal, v.as_str());
    }
    if let Some(v) = &m.lens_model {
        exif.add_tag(ExifTag::LensModel, v.as_str());
    }
}

fn dng_err(e: impl std::fmt::Display) -> Error {
    Error::Decode(format!("dng: {e}"))
}
