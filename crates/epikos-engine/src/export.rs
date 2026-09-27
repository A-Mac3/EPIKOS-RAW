//! Step 8 handoff (PRD Section 1.2): full-resolution exports for Photoshop, Lightroom,
//! Capture One and DxO, with the AI masks intact.
//!
//! - **TIFF**: 16-bit with ICC profile; masks and depth as extra channels, which
//!   Photoshop opens as named alpha channels (image-resource tag 34377).
//! - **PSD**: layered, the masks as masked layer groups (see [`crate::psd`]).
//! - **DNG**: enhanced linear DNG of the scene-referred image with semantic masks
//!   (see [`crate::dng`]). From a JPEG, PNG or TIFF it holds the decoded pixels made
//!   linear: raw-style editing, but no more latitude than the source had.
//! - **JPEG** (8-bit, quality 92) and **PNG** (16-bit): the finished look with ICC
//!   profile and EXIF, for sharing; they have no room for mask channels.

use std::borrow::Cow;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use epikos_core::{CaptureMetadata, Error, GpsInfo, Result};
use epikos_core::{resize_plane, ImageRgbF32};
use epikos_pipeline::{develop_adjustments_with, encode_rgb16, fit_to_image, OutputSpace};
use epikos_sidecar::Adjustments;
use serde::{Deserialize, Serialize};
use tiff::encoder::compression::DeflateLevel;
use tiff::encoder::colortype::ColorType;
use tiff::encoder::{
    colortype, Compression, DirectoryEncoder, Predictor, Rational, SRational, TiffEncoder,
    TiffKind, TiffValue,
};
use tiff::tags::{PhotometricInterpretation, SampleFormat, Tag, Type};

use crate::Loaded;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    #[default]
    Tiff,
    Psd,
    Dng,
    Jpeg,
    Png,
}

impl ExportFormat {
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Tiff => "16-bit TIFF",
            ExportFormat::Psd => "Layered PSD",
            ExportFormat::Dng => "Enhanced DNG",
            ExportFormat::Jpeg => "JPEG",
            ExportFormat::Png => "16-bit PNG",
        }
    }

    /// Whether the format can carry the AI masks and depth as extra channels.
    pub fn carries_masks(self) -> bool {
        !matches!(self, ExportFormat::Jpeg | ExportFormat::Png)
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            ExportFormat::Tiff => &["tif", "tiff"],
            ExportFormat::Psd => &["psd"],
            ExportFormat::Dng => &["dng"],
            ExportFormat::Jpeg => &["jpg", "jpeg"],
            ExportFormat::Png => &["png"],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// TIFF and PSD only; a DNG stays in its linear camera-independent space.
    pub color_space: OutputSpace,
    /// Copy GPS position into the export. On by default, as in Lightroom and Photoshop.
    pub include_location: bool,
    /// Add the subject, sky and skin masks as alpha channels.
    pub ai_masks: bool,
    /// Add the depth map as an alpha channel (e.g. for Photoshop's Lens Blur).
    pub depth_channel: bool,
    /// Downsize so the long edge is at most this many pixels; `None` = full size.
    pub long_edge: Option<u32>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Tiff,
            color_space: OutputSpace::Srgb,
            include_location: true,
            ai_masks: false,
            depth_channel: false,
            long_edge: None,
        }
    }
}

/// JPEG quality: visually lossless for photographs at a fraction of the size.
const JPEG_QUALITY: u8 = 92;

/// A low-resolution model output (0–1) to be written as a named alpha channel.
pub(crate) struct AuxPlane {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

/// At most this many extra channels (subject, sky, skin, eyes, hair, depth).
const MAX_EXTRA: usize = 6;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub format: ExportFormat,
    pub width: u32,
    pub height: u32,
    pub color_space: String,
    pub bytes: u64,
    /// Whether EXIF capture data (and GPS, if any and allowed) was written.
    pub wrote_exif: bool,
    pub wrote_location: bool,
    /// Names of the alpha channels written, in order.
    pub alpha_channels: Vec<String>,
    pub develop_ms: u64,
    pub write_ms: u64,
}

pub(crate) fn export(
    loaded: &Loaded,
    adjustments: &Adjustments,
    dest: &Path,
    options: ExportOptions,
    prepared: &crate::Prepared,
    aux: Vec<AuxPlane>,
) -> Result<ExportReport> {
    let format = options.format;
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !format.extensions().contains(&ext.as_str()) {
        return Err(Error::InvalidImage {
            reason: format!(
                "a {} export must be a .{} file, got {}",
                format.label(),
                format.extensions()[0],
                dest.display()
            ),
        });
    }
    if same_file(dest, Path::new(&loaded.raw.source_path)) {
        return Err(Error::InvalidImage {
            reason: "refusing to overwrite the original RAW file".into(),
        });
    }

    let t = Instant::now();
    // The DNG carries the scene (lens-corrected, since it has no opcodes of its own),
    // not the look: the reader applies its own rendering.
    let rgb = if format == ExportFormat::Dng {
        // Red-eye needs the eye mask; erase and healing are kept as they are.
        let masks = prepared.masks.iter().filter(|m| m.target == epikos_sidecar::MaskTarget::Eyes).cloned().collect();
        let scene = crate::Prepared {
            depth: None,
            masks,
            lens: prepared.lens.clone(),
            lut: None,
            fills: prepared.fills.clone(),
        };
        scene.with_inputs(|inputs| {
            // Framing and retouching are kept: they are part of the photo, not of the look.
            let scene = Adjustments {
                crop: adjustments.crop.clone(),
                retouch: adjustments.retouch.clone(),
                ..crate::scene_only(adjustments)
            };
            develop_adjustments_with(&loaded.raw.mosaic, &loaded.raw.profile, &scene, inputs)
        })?
    } else {
        prepared.with_inputs(|inputs| develop_adjustments_with(&loaded.raw.mosaic, &loaded.raw.profile, adjustments, inputs))?
    };
    let rgb = match options.long_edge {
        Some(edge) => downsize(rgb, edge),
        None => rgb,
    };
    let (width, height) = (rgb.width, rgb.height);
    let aux = aux.into_iter().take(MAX_EXTRA).collect::<Vec<_>>();
    // Fit each model output to this image's edges, then quantise to 16 bits.
    let channels: Vec<(String, Vec<u16>)> = aux
        .iter()
        .filter_map(|a| {
            let fitted = fit_to_image(&rgb, &a.data, a.width, a.height, 0.003)?;
            Some((a.name.clone(), fitted.iter().map(|v| (v * 65535.0).round() as u16).collect()))
        })
        .collect();
    let names: Vec<String> = channels.iter().map(|(n, _)| n.clone()).collect();
    let space = options.color_space;
    let meta = &loaded.raw.metadata;
    let gps = meta
        .gps
        .as_ref()
        .filter(|g| options.include_location && g.has_position());

    // Write beside the destination, then rename: a failed export never leaves a
    // truncated file where the user expects a finished one.
    let partial = partial_path(dest);
    let (develop_ms, t_write, result) = match format {
        ExportFormat::Tiff => {
            let pixels = interleave(encode_rgb16(&rgb, space), &channels);
            drop((rgb, channels));
            let develop_ms = t.elapsed().as_millis() as u64;
            let t = Instant::now();
            let image = TiffImage { width, height, pixels: &pixels, alpha_names: &names };
            let r = match names.len() {
                0 => write_tiff::<colortype::RGB16>(&partial, &image, space, meta, gps),
                1 => write_tiff::<Rgb16Plus1>(&partial, &image, space, meta, gps),
                2 => write_tiff::<Rgb16Plus2>(&partial, &image, space, meta, gps),
                3 => write_tiff::<Rgb16Plus3>(&partial, &image, space, meta, gps),
                4 => write_tiff::<Rgb16Plus4>(&partial, &image, space, meta, gps),
                5 => write_tiff::<Rgb16Plus5>(&partial, &image, space, meta, gps),
                6 => write_tiff::<Rgb16Plus6>(&partial, &image, space, meta, gps),
                n => unreachable!("{n} extra channels; at most {MAX_EXTRA} are kept"),
            };
            (develop_ms, t, r)
        }
        ExportFormat::Psd => {
            let pixels = encode_rgb16(&rgb, space);
            drop(rgb);
            let develop_ms = t.elapsed().as_millis() as u64;
            let t = Instant::now();
            let icc = space.icc_profile();
            let exif = exif_block(meta, space, gps)?;
            let xmp = xmp_packet(meta, gps);
            let image = crate::psd::PsdImage {
                width,
                height,
                rgb: &pixels,
                icc: &icc,
                exif: &exif,
                xmp: &xmp,
                ppi: 300,
                masks: channels
                    .iter()
                    .map(|(name, mask)| crate::psd::MaskLayer { name, mask })
                    .collect(),
            };
            let r = crate::psd::write_psd(&partial, &image).map_err(Error::from);
            (develop_ms, t, r)
        }
        ExportFormat::Dng => {
            let develop_ms = t.elapsed().as_millis() as u64;
            let t = Instant::now();
            let r = crate::dng::write_dng(&partial, &rgb, &channels, meta, gps);
            (develop_ms, t, r)
        }
        ExportFormat::Jpeg | ExportFormat::Png => {
            let pixels = encode_rgb16(&rgb, space);
            drop(rgb);
            let develop_ms = t.elapsed().as_millis() as u64;
            let t = Instant::now();
            let exif = exif_block(meta, space, gps)?;
            let r = write_shareable(&partial, format, width, height, &pixels, space.icc_profile(), exif);
            (develop_ms, t, r)
        }
    };
    let result = result.and_then(|()| fs::rename(&partial, dest).map_err(Error::from));
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result?;

    Ok(ExportReport {
        path: dest.to_string_lossy().into_owned(),
        format,
        width,
        height,
        color_space: if format == ExportFormat::Dng {
            "Linear Rec.2020 (DNG)".to_string()
        } else {
            space.label().to_string()
        },
        bytes: fs::metadata(dest)?.len(),
        wrote_exif: true,
        wrote_location: gps.is_some(),
        alpha_channels: names,
        develop_ms,
        write_ms: t_write.elapsed().as_millis() as u64,
    })
}

/// A JPEG (8-bit) or 16-bit PNG with ICC profile and EXIF.
fn write_shareable(
    dest: &Path,
    format: ExportFormat,
    width: u32,
    height: u32,
    rgb16: &[u16],
    icc: Vec<u8>,
    exif: Vec<u8>,
) -> Result<()> {
    use image::{ExtendedColorType, ImageEncoder};
    let unsupported = |e: image::error::UnsupportedError| Error::InvalidImage { reason: e.to_string() };
    let encode_err = |e: image::ImageError| Error::InvalidImage { reason: e.to_string() };
    let file = BufWriter::new(File::create(dest)?);
    match format {
        ExportFormat::Jpeg => {
            let rgb8: Vec<u8> = rgb16.iter().map(|&v| ((v as u32 * 255 + 32_767) / 65_535) as u8).collect();
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(file, JPEG_QUALITY);
            enc.set_icc_profile(icc).map_err(unsupported)?;
            enc.set_exif_metadata(exif).map_err(unsupported)?;
            enc.write_image(&rgb8, width, height, ExtendedColorType::Rgb8).map_err(encode_err)
        }
        ExportFormat::Png => {
            // Native-endian samples; the encoder writes PNG's big-endian.
            let bytes: Vec<u8> = rgb16.iter().flat_map(|v| v.to_ne_bytes()).collect();
            let mut enc = image::codecs::png::PngEncoder::new_with_quality(
                file,
                image::codecs::png::CompressionType::Default,
                image::codecs::png::FilterType::Adaptive,
            );
            enc.set_icc_profile(icc).map_err(unsupported)?;
            enc.set_exif_metadata(exif).map_err(unsupported)?;
            enc.write_image(&bytes, width, height, ExtendedColorType::Rgb16).map_err(encode_err)
        }
        _ => unreachable!("{format:?} is not a shareable format"),
    }
}

/// Scale planes down so the long edge is at most `edge` (never up).
fn downsize(rgb: ImageRgbF32, edge: u32) -> ImageRgbF32 {
    let long = rgb.width.max(rgb.height);
    if edge == 0 || long <= edge {
        return rgb;
    }
    let scale = edge as f64 / long as f64;
    let w = ((rgb.width as f64 * scale).round() as u32).max(1);
    let h = ((rgb.height as f64 * scale).round() as u32).max(1);
    let mut out = ImageRgbF32::new(w, h, rgb.space);
    out.r = resize_plane(&rgb.r, rgb.width, rgb.height, w, h);
    out.g = resize_plane(&rgb.g, rgb.width, rgb.height, w, h);
    out.b = resize_plane(&rgb.b, rgb.width, rgb.height, w, h);
    out
}

/// RGB16 pixels followed by each channel's sample, pixel by pixel.
fn interleave(rgb: Vec<u16>, channels: &[(String, Vec<u16>)]) -> Vec<u16> {
    if channels.is_empty() {
        return rgb;
    }
    let n = rgb.len() / 3;
    let stride = 3 + channels.len();
    let mut out = Vec::with_capacity(n * stride);
    for i in 0..n {
        out.extend_from_slice(&rgb[i * 3..i * 3 + 3]);
        out.extend(channels.iter().map(|(_, c)| c[i]));
    }
    out
}

struct TiffImage<'a> {
    width: u32,
    height: u32,
    /// Interleaved RGB plus one sample per alpha channel.
    pixels: &'a [u16],
    alpha_names: &'a [String],
}

/// 16-bit RGB with `N` extra channels. The `tiff` crate's own `extra_samples` keeps
/// predicting with a 3-sample stride, which corrupts extra channels once decoded; these
/// types carry the full sample count, so the predictor strides over whole pixels.
macro_rules! rgb16_plus {
    ($name:ident, $n:literal) => {
        struct $name;
        impl ColorType for $name {
            type Inner = u16;
            const TIFF_VALUE: PhotometricInterpretation = PhotometricInterpretation::RGB;
            const BITS_PER_SAMPLE: &'static [u16] = &[16; 3 + $n];
            const SAMPLE_FORMAT: &'static [SampleFormat] = &[SampleFormat::Uint; 3 + $n];
            fn horizontal_predict(row: &[u16], result: &mut Vec<u16>) {
                predict(row, 3 + $n, result)
            }
        }
    };
}
rgb16_plus!(Rgb16Plus1, 1);
rgb16_plus!(Rgb16Plus2, 2);
rgb16_plus!(Rgb16Plus3, 3);
rgb16_plus!(Rgb16Plus4, 4);
rgb16_plus!(Rgb16Plus5, 5);
rgb16_plus!(Rgb16Plus6, 6);
// One colour type per possible channel count, up to MAX_EXTRA.
const _: () = assert!(MAX_EXTRA == 6);

/// TIFF horizontal differencing: each sample minus the same sample one pixel left.
fn predict(row: &[u16], stride: usize, result: &mut Vec<u16>) {
    result.extend_from_slice(&row[..stride.min(row.len())]);
    result.extend(row.iter().zip(row.iter().skip(stride)).map(|(prev, cur)| cur.wrapping_sub(*prev)));
}

/// Photoshop image resources naming the alpha channels: 1006 (Pascal strings, for
/// older readers) and 1045 (Unicode names).
fn photoshop_alpha_names(names: &[String]) -> Vec<u8> {
    fn block(out: &mut Vec<u8>, id: u16, data: &[u8]) {
        out.extend_from_slice(b"8BIM");
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&[0, 0]); // empty resource name, padded to even
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
    }
    let mut pascal = Vec::new();
    let mut unicode = Vec::new();
    for name in names {
        let latin: Vec<u8> = name.chars().map(|c| if c.is_ascii() { c as u8 } else { b'?' }).take(255).collect();
        pascal.push(latin.len() as u8);
        pascal.extend_from_slice(&latin);
        let utf16: Vec<u16> = name.encode_utf16().chain([0]).collect();
        unicode.extend_from_slice(&(utf16.len() as u32).to_be_bytes());
        unicode.extend(utf16.iter().flat_map(|u| u.to_be_bytes()));
    }
    let mut out = Vec::new();
    block(&mut out, 1006, &pascal);
    block(&mut out, 1045, &unicode);
    out
}

fn write_tiff<C: ColorType<Inner = u16>>(
    path: &Path,
    img: &TiffImage,
    space: OutputSpace,
    meta: &CaptureMetadata,
    gps: Option<&GpsInfo>,
) -> Result<()>
where
    [u16]: TiffValue,
{
    let (width, height) = (img.width, img.height);
    let file = BufWriter::new(File::create(path)?);
    // Deflate ("ZIP" in Photoshop) with horizontal differencing: lossless.
    let mut encoder = TiffEncoder::new(file)
        .map_err(tiff_err)?
        .with_compression(Compression::Deflate(DeflateLevel::Fast))
        .with_predictor(Predictor::Horizontal);

    // Sub-directories are written first so the image directory can point at them.
    let exif = {
        let mut dir = encoder.extra_directory().map_err(tiff_err)?;
        write_exif(&mut dir, meta, space).map_err(tiff_err)?;
        dir.finish_with_offsets().map_err(tiff_err)?
    };
    let gps_dir = match gps {
        Some(g) => {
            let mut dir = encoder.extra_directory().map_err(tiff_err)?;
            write_gps(&mut dir, g).map_err(tiff_err)?;
            Some(dir.finish_with_offsets().map_err(tiff_err)?)
        }
        None => None,
    };

    let mut image = encoder.new_image::<C>(width, height).map_err(tiff_err)?;
    let dir = image.encoder();
    (|| {
        dir.write_tag(Tag::IccProfile, Undefined(&space.icc_profile()))?;
        write_camera_tags(dir, meta)?;
        // The encoder's default is 1 dpi, which Photoshop reads as a metres-wide print.
        dir.write_tag(Tag::XResolution, Rational { n: 300, d: 1 })?;
        dir.write_tag(Tag::YResolution, Rational { n: 300, d: 1 })?;
        dir.write_tag(Tag::ResolutionUnit, 2u16)?; // inch
        dir.write_tag(Tag::ExifDirectory, exif.offset)?;
        if let Some(g) = &gps_dir {
            dir.write_tag(Tag::GpsDirectory, g.offset)?;
        }
        if !img.alpha_names.is_empty() {
            // 0 = unspecified: plain channels, not transparency.
            dir.write_tag(Tag::ExtraSamples, &vec![0u16; img.alpha_names.len()][..])?;
            dir.write_tag(Tag::Unknown(PHOTOSHOP_RESOURCES), Undefined(&photoshop_alpha_names(img.alpha_names)))?;
        }
        Ok(())
    })()
    .map_err(tiff_err)?;
    image.write_data(img.pixels).map_err(tiff_err)?;
    Ok(())
}

/// TIFF tag holding Photoshop image resources.
const PHOTOSHOP_RESOURCES: u16 = 34377;

/// Main-IFD identity tags shared by the TIFF export and the PSD's EXIF block.
fn write_camera_tags<W: std::io::Write + std::io::Seek, K: TiffKind>(
    dir: &mut DirectoryEncoder<'_, W, K>,
    meta: &CaptureMetadata,
) -> tiff::TiffResult<()> {
    dir.write_tag(Tag::Software, "EPIKOS RAW")?;
    if !meta.make.is_empty() {
        dir.write_tag(Tag::Make, meta.make.as_str())?;
    }
    if !meta.model.is_empty() {
        dir.write_tag(Tag::Model, meta.model.as_str())?;
    }
    if let Some(v) = &meta.artist {
        dir.write_tag(Tag::Artist, v.as_str())?;
    }
    if let Some(v) = &meta.copyright {
        dir.write_tag(Tag::Copyright, v.as_str())?;
    }
    // Pixels are already upright; stop viewers from rotating them again.
    dir.write_tag(Tag::Orientation, 1u16)
}

/// Standalone EXIF block: a TIFF header and a main IFD pointing at the EXIF (and,
/// when allowed, GPS) sub-IFDs, with no image. The same layout as a JPEG's APP1
/// payload; Photoshop stores it in image resource 1058.
pub(crate) fn exif_block(meta: &CaptureMetadata, space: OutputSpace, gps: Option<&GpsInfo>) -> Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut encoder = TiffEncoder::new(&mut buf).map_err(tiff_err)?;
        let exif = {
            let mut dir = encoder.extra_directory().map_err(tiff_err)?;
            write_exif(&mut dir, meta, space).map_err(tiff_err)?;
            dir.finish_with_offsets().map_err(tiff_err)?
        };
        let gps_dir = match gps {
            Some(g) => {
                let mut dir = encoder.extra_directory().map_err(tiff_err)?;
                write_gps(&mut dir, g).map_err(tiff_err)?;
                Some(dir.finish_with_offsets().map_err(tiff_err)?)
            }
            None => None,
        };
        let mut dir = encoder.image_directory().map_err(tiff_err)?;
        (|| {
            write_camera_tags(&mut dir, meta)?;
            dir.write_tag(Tag::ExifDirectory, exif.offset)?;
            if let Some(g) = &gps_dir {
                dir.write_tag(Tag::GpsDirectory, g.offset)?;
            }
            Ok(())
        })()
        .map_err(tiff_err)?;
        dir.finish().map_err(tiff_err)?;
    }
    Ok(buf.into_inner())
}

/// XMP packet with the capture metadata, which Lightroom and Bridge read from a PSD
/// (image resource 1060). Dates follow XMP's ISO 8601 form; GPS uses its
/// "DD,MM.mmmmmmK" form.
pub(crate) fn xmp_packet(meta: &CaptureMetadata, gps: Option<&GpsInfo>) -> Vec<u8> {
    let esc = |s: &str| {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    };
    let ratio = |(n, d): (u32, u32)| format!("{n}/{d}");
    let mut attrs = vec![("xmp:CreatorTool".to_string(), "EPIKOS RAW".to_string()), ("tiff:Orientation".into(), "1".into())];
    let mut push = |k: &str, v: String| attrs.push((k.to_string(), esc(&v)));
    if !meta.make.is_empty() {
        push("tiff:Make", meta.make.clone());
    }
    if !meta.model.is_empty() {
        push("tiff:Model", meta.model.clone());
    }
    if let Some(date) = meta.date_time_original.as_deref().and_then(xmp_date) {
        let mut date = date;
        if let Some(sub) = &meta.sub_sec_time_original {
            date += &format!(".{}", sub.trim());
        }
        if let Some(off) = &meta.offset_time_original {
            date += off.trim();
        }
        push("exif:DateTimeOriginal", date.clone());
        push("xmp:CreateDate", date.clone());
        push("photoshop:DateCreated", date);
    }
    if let Some(v) = meta.exposure_time {
        push("exif:ExposureTime", ratio(v));
    }
    if let Some(v) = meta.f_number {
        push("exif:FNumber", ratio(v));
    }
    if let Some(v) = meta.focal_length {
        push("exif:FocalLength", ratio(v));
    }
    if let Some((n, d)) = meta.exposure_bias {
        push("exif:ExposureBiasValue", format!("{n}/{d}"));
    }
    if let Some(v) = &meta.lens_model {
        push("aux:Lens", v.clone());
        push("exifEX:LensModel", v.clone());
    }
    if let Some(v) = &meta.lens_make {
        push("exifEX:LensMake", v.clone());
    }
    if let Some(v) = &meta.serial_number {
        push("exifEX:BodySerialNumber", v.clone());
    }
    if let Some(g) = gps {
        let coord = |v: Option<[(u32, u32); 3]>, r: &Option<String>| {
            let [d, m, s] = v?;
            let f = |(n, d): (u32, u32)| if d == 0 { 0.0 } else { n as f64 / d as f64 };
            let minutes = f(m) + f(s) / 60.0;
            Some(format!("{},{:.6}{}", f(d).trunc(), minutes, r.as_deref()?.trim()))
        };
        if let Some(v) = coord(g.latitude, &g.latitude_ref) {
            push("exif:GPSLatitude", v);
        }
        if let Some(v) = coord(g.longitude, &g.longitude_ref) {
            push("exif:GPSLongitude", v);
        }
        if let Some(v) = g.altitude {
            push("exif:GPSAltitude", ratio(v));
            push("exif:GPSAltitudeRef", g.altitude_ref.unwrap_or(0).to_string());
        }
    }
    let mut elements = String::new();
    if let Some(iso) = meta.iso {
        elements += &format!("   <exif:ISOSpeedRatings><rdf:Seq><rdf:li>{iso}</rdf:li></rdf:Seq></exif:ISOSpeedRatings>\n");
    }
    if let Some(a) = &meta.artist {
        elements += &format!("   <dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>\n", esc(a));
    }
    if let Some(c) = &meta.copyright {
        elements += &format!(
            "   <dc:rights><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:rights>\n",
            esc(c)
        );
    }
    let attrs: String = attrs.iter().map(|(k, v)| format!("\n    {k}=\"{v}\"")).collect();
    format!(
        r#"<?xpacket begin="\u{{feff}}" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="EPIKOS RAW">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:tiff="http://ns.adobe.com/tiff/1.0/"
    xmlns:exif="http://ns.adobe.com/exif/1.0/"
    xmlns:exifEX="http://cipa.jp/exif/1.0/"
    xmlns:aux="http://ns.adobe.com/exif/1.0/aux/"
    xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"{attrs}>
{elements}  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#
    )
    .into_bytes()
}

/// EXIF "YYYY:MM:DD HH:MM:SS" → XMP "YYYY-MM-DDTHH:MM:SS".
fn xmp_date(exif: &str) -> Option<String> {
    let (date, time) = exif.trim().split_once(' ')?;
    let date = date.replace(':', "-");
    (date.len() == 10 && time.len() >= 8).then(|| format!("{date}T{}", &time[..8]))
}

fn tiff_err(e: tiff::TiffError) -> Error {
    Error::Decode(format!("tiff: {e}"))
}

/// EXIF 2.32 private IFD tags.
mod exif_tag {
    pub const EXPOSURE_TIME: u16 = 33434;
    pub const F_NUMBER: u16 = 33437;
    pub const EXPOSURE_PROGRAM: u16 = 34850;
    pub const ISO: u16 = 34855;
    pub const SENSITIVITY_TYPE: u16 = 34864;
    pub const RECOMMENDED_EXPOSURE_INDEX: u16 = 34866;
    pub const EXIF_VERSION: u16 = 36864;
    pub const DATE_TIME_ORIGINAL: u16 = 36867;
    pub const DATE_TIME_DIGITIZED: u16 = 36868;
    pub const OFFSET_TIME_ORIGINAL: u16 = 36881;
    pub const EXPOSURE_BIAS: u16 = 37380;
    pub const MAX_APERTURE: u16 = 37381;
    pub const METERING_MODE: u16 = 37383;
    pub const FLASH: u16 = 37385;
    pub const FOCAL_LENGTH: u16 = 37386;
    pub const SUB_SEC_TIME_ORIGINAL: u16 = 37521;
    pub const COLOR_SPACE: u16 = 40961;
    pub const EXPOSURE_MODE: u16 = 41986;
    pub const WHITE_BALANCE: u16 = 41987;
    pub const BODY_SERIAL: u16 = 42033;
    pub const LENS_SPEC: u16 = 42034;
    pub const LENS_MAKE: u16 = 42035;
    pub const LENS_MODEL: u16 = 42036;
    pub const LENS_SERIAL: u16 = 42037;
}

fn write_exif<W: std::io::Write + std::io::Seek, K: TiffKind>(
    dir: &mut DirectoryEncoder<'_, W, K>,
    m: &CaptureMetadata,
    space: OutputSpace,
) -> tiff::TiffResult<()> {
    use exif_tag::*;
    let t = Tag::Unknown;
    dir.write_tag(t(EXIF_VERSION), Undefined(b"0232"))?;
    // 1 = sRGB; everything else is "uncalibrated" and relies on the ICC profile.
    let cs: u16 = if space == OutputSpace::Srgb {
        1
    } else {
        0xFFFF
    };
    dir.write_tag(t(COLOR_SPACE), cs)?;

    macro_rules! opt {
        ($tag:expr, $v:expr, |$x:ident| $conv:expr) => {
            if let Some($x) = &$v {
                dir.write_tag(t($tag), $conv)?;
            }
        };
    }
    opt!(EXPOSURE_TIME, m.exposure_time, |x| rational(*x));
    opt!(F_NUMBER, m.f_number, |x| rational(*x));
    opt!(FOCAL_LENGTH, m.focal_length, |x| rational(*x));
    opt!(MAX_APERTURE, m.max_aperture, |x| rational(*x));
    opt!(EXPOSURE_BIAS, m.exposure_bias, |x| SRational {
        n: x.0,
        d: x.1
    });
    if let Some(iso) = m.iso {
        dir.write_tag(t(ISO), iso.min(u16::MAX as u32) as u16)?;
        if iso > u16::MAX as u32 {
            // ISO above 65535 doesn't fit the legacy SHORT field.
            dir.write_tag(t(SENSITIVITY_TYPE), 2u16)?;
            dir.write_tag(t(RECOMMENDED_EXPOSURE_INDEX), iso)?;
        }
    }
    opt!(EXPOSURE_PROGRAM, m.exposure_program, |x| *x);
    opt!(METERING_MODE, m.metering_mode, |x| *x);
    opt!(FLASH, m.flash, |x| *x);
    opt!(EXPOSURE_MODE, m.exposure_mode, |x| *x);
    opt!(WHITE_BALANCE, m.white_balance, |x| *x);
    opt!(DATE_TIME_ORIGINAL, m.date_time_original, |x| x.as_str());
    opt!(DATE_TIME_DIGITIZED, m.date_time_digitized, |x| x.as_str());
    opt!(OFFSET_TIME_ORIGINAL, m.offset_time_original, |x| x.as_str());
    opt!(SUB_SEC_TIME_ORIGINAL, m.sub_sec_time_original, |x| x
        .as_str());
    opt!(BODY_SERIAL, m.serial_number, |x| x.as_str());
    opt!(LENS_MAKE, m.lens_make, |x| x.as_str());
    opt!(LENS_MODEL, m.lens_model, |x| x.as_str());
    opt!(LENS_SERIAL, m.lens_serial_number, |x| x.as_str());
    opt!(LENS_SPEC, m.lens_spec, |x| x.map(rational));
    Ok(())
}

fn write_gps<W: std::io::Write + std::io::Seek, K: TiffKind>(
    dir: &mut DirectoryEncoder<'_, W, K>,
    g: &GpsInfo,
) -> tiff::TiffResult<()> {
    let t = Tag::Unknown;
    dir.write_tag(t(0), g.version.unwrap_or([2, 3, 0, 0]))?;
    macro_rules! opt {
        ($tag:expr, $v:expr, |$x:ident| $conv:expr) => {
            if let Some($x) = &$v {
                dir.write_tag(t($tag), $conv)?;
            }
        };
    }
    opt!(1, g.latitude_ref, |x| x.as_str());
    opt!(2, g.latitude, |x| x.map(rational));
    opt!(3, g.longitude_ref, |x| x.as_str());
    opt!(4, g.longitude, |x| x.map(rational));
    opt!(5, g.altitude_ref, |x| *x);
    opt!(6, g.altitude, |x| rational(*x));
    opt!(7, g.time_stamp, |x| x.map(rational));
    opt!(8, g.satellites, |x| x.as_str());
    opt!(9, g.status, |x| x.as_str());
    opt!(10, g.measure_mode, |x| x.as_str());
    opt!(11, g.dop, |x| rational(*x));
    opt!(16, g.img_direction_ref, |x| x.as_str());
    opt!(17, g.img_direction, |x| rational(*x));
    opt!(18, g.map_datum, |x| x.as_str());
    opt!(29, g.date_stamp, |x| x.as_str());
    opt!(31, g.h_positioning_error, |x| rational(*x));
    Ok(())
}

fn rational((n, d): (u32, u32)) -> Rational {
    Rational { n, d }
}

/// Raw bytes written with TIFF type UNDEFINED, as the ICC tag (34675) requires.
struct Undefined<'a>(&'a [u8]);

impl TiffValue for Undefined<'_> {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: Type = Type::UNDEFINED;

    fn count(&self) -> usize {
        self.0.len()
    }

    fn data(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(self.0)
    }
}

fn partial_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(".partial");
    PathBuf::from(s)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
