//! Step 8 handoff (PRD Section 1.2): full-resolution exports for Photoshop, Lightroom,
//! Capture One and DxO, with the AI masks intact.
//!
//! - **TIFF**: 16-bit with ICC profile; masks and depth as extra channels, which
//!   Photoshop opens as named alpha channels (image-resource tag 34377).
//! - **PSD**: layered, the masks as masked layer groups (see [`crate::psd`]).
//! - **DNG**: enhanced linear DNG of the scene-referred image with semantic masks
//!   (see [`crate::dng`]).

use std::borrow::Cow;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use epikos_core::{CaptureMetadata, Error, GpsInfo, Result};
use epikos_masks::DepthMap;
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
}

impl ExportFormat {
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Tiff => "16-bit TIFF",
            ExportFormat::Psd => "Layered PSD",
            ExportFormat::Dng => "Enhanced DNG",
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            ExportFormat::Tiff => &["tif", "tiff"],
            ExportFormat::Psd => &["psd"],
            ExportFormat::Dng => &["dng"],
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

/// A low-resolution model output (0–1) to be written as a named alpha channel.
pub(crate) struct AuxPlane {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
}

/// At most this many extra channels (subject, sky, skin, depth).
const MAX_EXTRA: usize = 4;

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
    depth: Option<&DepthMap>,
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
    // The DNG carries the scene, not the look (the reader applies its own rendering).
    let rgb = if format == ExportFormat::Dng {
        develop_adjustments_with(&loaded.raw.mosaic, &loaded.raw.profile, &crate::scene_only(adjustments), &Default::default())?
    } else {
        let inputs = crate::look_inputs(depth);
        develop_adjustments_with(&loaded.raw.mosaic, &loaded.raw.profile, adjustments, &inputs)?
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
                _ => write_tiff::<Rgb16Plus4>(&partial, &image, space, meta, gps),
            };
            (develop_ms, t, r)
        }
        ExportFormat::Psd => {
            let pixels = encode_rgb16(&rgb, space);
            drop(rgb);
            let develop_ms = t.elapsed().as_millis() as u64;
            let t = Instant::now();
            let icc = space.icc_profile();
            let image = crate::psd::PsdImage {
                width,
                height,
                rgb: &pixels,
                icc: &icc,
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
            let r = crate::dng::write_dng(&partial, &rgb, &channels, meta, gps.is_some());
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
        wrote_exif: format != ExportFormat::Psd,
        wrote_location: gps.is_some() && format == ExportFormat::Tiff,
        alpha_channels: names,
        develop_ms,
        write_ms: t_write.elapsed().as_millis() as u64,
    })
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
    let ascii = |dir: &mut DirectoryEncoder<'_, _, _>, tag: Tag, v: &Option<String>| match v {
        Some(v) => dir.write_tag(tag, v.as_str()),
        None => Ok(()),
    };
    (|| {
        dir.write_tag(Tag::IccProfile, Undefined(&space.icc_profile()))?;
        dir.write_tag(Tag::Software, "EPIKOS RAW")?;
        if !meta.make.is_empty() {
            dir.write_tag(Tag::Make, meta.make.as_str())?;
        }
        if !meta.model.is_empty() {
            dir.write_tag(Tag::Model, meta.model.as_str())?;
        }
        ascii(dir, Tag::Artist, &meta.artist)?;
        ascii(dir, Tag::Copyright, &meta.copyright)?;
        // Pixels are already upright; stop viewers from rotating them again.
        dir.write_tag(Tag::Orientation, 1u16)?;
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
