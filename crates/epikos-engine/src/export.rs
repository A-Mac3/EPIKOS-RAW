//! Full-resolution 16-bit TIFF export with an embedded ICC profile (PRD Section 1.2 /
//! Step 8 handoff to Photoshop, Lightroom, Capture One, DxO).

use std::borrow::Cow;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use epikos_core::{CaptureMetadata, Error, GpsInfo, Result};
use epikos_pipeline::{develop_adjustments, encode_rgb16, OutputSpace};
use epikos_sidecar::Adjustments;
use serde::{Deserialize, Serialize};
use tiff::encoder::compression::DeflateLevel;
use tiff::encoder::{
    colortype, Compression, DirectoryEncoder, Predictor, Rational, SRational, TiffEncoder,
    TiffKind, TiffValue,
};
use tiff::tags::{Tag, Type};

use crate::Loaded;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportOptions {
    pub color_space: OutputSpace,
    /// Copy GPS position into the export. On by default, as in Lightroom and Photoshop.
    pub include_location: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            color_space: OutputSpace::Srgb,
            include_location: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub color_space: String,
    pub bytes: u64,
    /// Whether EXIF capture data (and GPS, if any and allowed) was written.
    pub wrote_exif: bool,
    pub wrote_location: bool,
    pub develop_ms: u64,
    pub write_ms: u64,
}

pub(crate) fn export_tiff(
    loaded: &Loaded,
    adjustments: &Adjustments,
    dest: &Path,
    options: ExportOptions,
) -> Result<ExportReport> {
    let space = options.color_space;
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !matches!(ext.to_ascii_lowercase().as_str(), "tif" | "tiff") {
        return Err(Error::InvalidImage {
            reason: format!("export must be a .tif/.tiff file, got {}", dest.display()),
        });
    }
    if same_file(dest, Path::new(&loaded.raw.source_path)) {
        return Err(Error::InvalidImage {
            reason: "refusing to overwrite the original RAW file".into(),
        });
    }

    let t = Instant::now();
    let rgb = develop_adjustments(&loaded.raw.mosaic, &loaded.raw.profile, adjustments)?;
    let (width, height) = (rgb.width, rgb.height);
    let pixels = encode_rgb16(&rgb, space);
    drop(rgb);
    let develop_ms = t.elapsed().as_millis() as u64;

    // Write beside the destination, then rename: a failed export never leaves a
    // truncated TIFF where the user expects a finished one.
    let t = Instant::now();
    let partial = partial_path(dest);
    let meta = &loaded.raw.metadata;
    let gps = meta
        .gps
        .as_ref()
        .filter(|g| options.include_location && g.has_position());
    let result = write_tiff(&partial, width, height, &pixels, space, meta, gps)
        .and_then(|()| fs::rename(&partial, dest).map_err(Error::from));
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result?;

    Ok(ExportReport {
        path: dest.to_string_lossy().into_owned(),
        width,
        height,
        color_space: space.label().to_string(),
        bytes: fs::metadata(dest)?.len(),
        wrote_exif: true,
        wrote_location: gps.is_some(),
        develop_ms,
        write_ms: t.elapsed().as_millis() as u64,
    })
}

fn write_tiff(
    path: &Path,
    width: u32,
    height: u32,
    rgb16: &[u16],
    space: OutputSpace,
    meta: &CaptureMetadata,
    gps: Option<&GpsInfo>,
) -> Result<()> {
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

    let mut image = encoder
        .new_image::<colortype::RGB16>(width, height)
        .map_err(tiff_err)?;
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
        dir.write_tag(Tag::ExifDirectory, exif.offset)?;
        if let Some(g) = &gps_dir {
            dir.write_tag(Tag::GpsDirectory, g.offset)?;
        }
        Ok(())
    })()
    .map_err(tiff_err)?;
    image.write_data(rgb16).map_err(tiff_err)?;
    Ok(())
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
