//! Full-resolution 16-bit TIFF export with an embedded ICC profile (PRD Section 1.2 /
//! Step 8 handoff to Photoshop, Lightroom, Capture One, DxO).

use std::borrow::Cow;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::Instant;

use epikos_core::{Error, Result};
use epikos_pipeline::{develop_adjustments, encode_rgb16, OutputSpace};
use epikos_sidecar::Adjustments;
use serde::Serialize;
use tiff::encoder::compression::DeflateLevel;
use tiff::encoder::{colortype, Compression, Predictor, TiffEncoder, TiffValue};
use tiff::tags::{Tag, Type};

use crate::Loaded;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub color_space: String,
    pub bytes: u64,
    pub develop_ms: u64,
    pub write_ms: u64,
}

pub(crate) fn export_tiff(
    loaded: &Loaded,
    adjustments: &Adjustments,
    dest: &Path,
    space: OutputSpace,
) -> Result<ExportReport> {
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
    let result = write_tiff(&partial, width, height, &pixels, &space.icc_profile())
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
        develop_ms,
        write_ms: t.elapsed().as_millis() as u64,
    })
}

fn write_tiff(path: &Path, width: u32, height: u32, rgb16: &[u16], icc: &[u8]) -> Result<()> {
    let tiff_err = |e: tiff::TiffError| Error::Decode(format!("tiff: {e}"));
    let file = BufWriter::new(File::create(path)?);
    // Deflate ("ZIP" in Photoshop) with horizontal differencing: lossless, and roughly
    // halves 16-bit photographic data.
    let mut encoder = TiffEncoder::new(file)
        .map_err(tiff_err)?
        .with_compression(Compression::Deflate(DeflateLevel::Fast))
        .with_predictor(Predictor::Horizontal);
    let mut image = encoder
        .new_image::<colortype::RGB16>(width, height)
        .map_err(tiff_err)?;
    image
        .encoder()
        .write_tag(Tag::IccProfile, Undefined(icc))
        .map_err(tiff_err)?;
    image
        .encoder()
        .write_tag(Tag::Software, "EPIKOS RAW")
        .map_err(tiff_err)?;
    image.write_data(rgb16).map_err(tiff_err)?;
    Ok(())
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
