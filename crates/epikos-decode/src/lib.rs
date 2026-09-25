//! Decode camera RAW and DNG files into a 32-bit linear mosaic / RGB buffer.
//!
//! Container parsers (CR3, ARW, NEF, RAF, DNG, ProRAW) are provided by
//! [`rawler`]. Black/white scaling and sensor metadata become [`epikos_core`] types.

mod convert;

use std::path::Path;
use std::sync::Arc;

use epikos_core::{CameraFormat, Orientation, Result};
use rawler::decoders::RawDecodeParams;
use rawler::rawsource::RawSource;
use sha2::{Digest, Sha256};

pub use convert::DecodedRaw;

/// Identify a file from extension + optional maker strings without decoding pixels.
pub fn identify(path: &Path, make: Option<&str>, model: Option<&str>) -> CameraFormat {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mut fmt = CameraFormat::from_extension(ext);
    if let (Some(make), Some(model)) = (make, model) {
        fmt = fmt.refine_dng(make, model);
    }
    fmt
}

/// Decode a RAW/DNG path into mosaiced 32-bit linear samples plus a sensor profile.
///
/// The file is read once; the same bytes feed the SHA-256 and the decoder.
pub fn decode_file(path: impl AsRef<Path>) -> Result<DecodedRaw> {
    let path = path.as_ref();
    let bytes = Arc::new(std::fs::read(path)?);
    let digest = hex::encode(Sha256::digest(bytes.as_slice()));
    let src = RawSource::new_from_shared_vec(bytes);
    let params = RawDecodeParams::default();
    let mut raw = rawler::decode(&src, &params).map_err(decode_err)?;
    raw.apply_scaling().map_err(decode_err)?;
    // rawler 0.8 leaves `RawImage::orientation` as Normal; the EXIF tag is authoritative.
    let orientation = exif_orientation(&src, &params);
    convert::from_rawler(path, raw, digest, orientation)
}

/// Camera-embedded JPEG preview, upright, downscaled so its longest side is at most
/// `max_side`.
///
/// This is the camera's own rendering (not an EPIKOS develop), which makes it cheap
/// enough for browsing whole folders.
pub fn embedded_thumbnail(path: impl AsRef<Path>, max_side: u32) -> Result<image::RgbImage> {
    let path = path.as_ref();
    let params = RawDecodeParams::default();
    let img = rawler::analyze::extract_thumbnail_pixels(path, &params).map_err(decode_err)?;
    let img = if img.width().max(img.height()) > max_side {
        img.thumbnail(max_side, max_side)
    } else {
        img
    };
    let orientation = RawSource::new(path)
        .map(|src| exif_orientation(&src, &params))
        .unwrap_or_default();
    Ok(orient_dynamic(img, orientation).to_rgb8())
}

fn exif_orientation(src: &RawSource, params: &RawDecodeParams) -> Orientation {
    rawler::get_decoder(src)
        .and_then(|d| d.raw_metadata(src, params))
        .ok()
        .and_then(|m| m.exif.orientation)
        .map(Orientation::from_exif)
        .unwrap_or_default()
}

fn orient_dynamic(img: image::DynamicImage, o: Orientation) -> image::DynamicImage {
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

fn decode_err(e: rawler::RawlerError) -> epikos_core::Error {
    epikos_core::Error::Decode(format!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn identifies_first_class_formats() {
        assert_eq!(
            identify(Path::new("a.ARW"), None, None),
            CameraFormat::SonyArw
        );
        assert_eq!(
            identify(Path::new("a.cr3"), None, None),
            CameraFormat::CanonCr3
        );
        assert_eq!(
            identify(Path::new("a.NEF"), None, None),
            CameraFormat::NikonNef
        );
        assert_eq!(
            identify(Path::new("a.RAF"), None, None),
            CameraFormat::FujifilmRaf
        );
        assert_eq!(
            identify(Path::new("a.dng"), Some("Leica"), Some("M11")),
            CameraFormat::LeicaDng
        );
        assert_eq!(
            identify(Path::new("IMG.DNG"), Some("Apple"), Some("iPhone 15 Pro")),
            CameraFormat::AppleProRaw
        );
    }
}
