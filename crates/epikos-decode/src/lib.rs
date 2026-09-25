//! Decode camera RAW and DNG files into a 32-bit linear mosaic / RGB buffer.
//!
//! Container parsers (CR3, ARW, NEF, RAF, DNG, ProRAW) are provided by
//! [`rawler`]. Black/white scaling and sensor metadata become [`epikos_core`] types.

mod convert;

use std::path::Path;

use epikos_core::{CameraFormat, Result};
use sha2::{Digest, Sha256};

pub use convert::DecodedRaw;

/// Identify a file from extension + optional maker strings without decoding pixels.
pub fn identify(path: &Path, make: Option<&str>, model: Option<&str>) -> CameraFormat {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let mut fmt = CameraFormat::from_extension(ext);
    if let (Some(make), Some(model)) = (make, model) {
        fmt = fmt.refine_dng(make, model);
    }
    fmt
}

/// Decode a RAW/DNG path into mosaiced 32-bit linear samples plus a sensor profile.
pub fn decode_file(path: impl AsRef<Path>) -> Result<DecodedRaw> {
    let path = path.as_ref();
    let digest = file_sha256(path)?;
    let mut raw = rawler::decode_file(path).map_err(|e| {
        epikos_core::Error::Decode(format!("{e}"))
    })?;
    raw.apply_scaling().map_err(|e| epikos_core::Error::Decode(format!("{e}")))?;
    convert::from_rawler(path, raw, digest)
}

/// Camera-embedded JPEG preview, downscaled so its longest side is at most `max_side`.
///
/// This is the camera's own rendering (not an EPIKOS develop), which makes it cheap
/// enough for browsing whole folders.
pub fn embedded_thumbnail(path: impl AsRef<Path>, max_side: u32) -> Result<image::RgbImage> {
    let params = rawler::decoders::RawDecodeParams::default();
    let img = rawler::analyze::extract_thumbnail_pixels(path.as_ref(), &params)
        .map_err(|e| epikos_core::Error::Decode(format!("{e}")))?;
    let img = if img.width().max(img.height()) > max_side {
        img.thumbnail(max_side, max_side)
    } else {
        img
    };
    Ok(img.to_rgb8())
}

fn file_sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let hash = Sha256::digest(&bytes);
    Ok(hex::encode(hash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn identifies_first_class_formats() {
        assert_eq!(identify(Path::new("a.ARW"), None, None), CameraFormat::SonyArw);
        assert_eq!(identify(Path::new("a.cr3"), None, None), CameraFormat::CanonCr3);
        assert_eq!(identify(Path::new("a.NEF"), None, None), CameraFormat::NikonNef);
        assert_eq!(identify(Path::new("a.RAF"), None, None), CameraFormat::FujifilmRaf);
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
