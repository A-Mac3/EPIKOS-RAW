use std::path::Path;

use epikos_core::{
    CameraFormat, CaptureMetadata, CfaPattern, Error, MosaicF32, Orientation, Result, SensorLayout,
    SensorProfile,
};
use rawler::rawimage::{RawImage, RawImageData, RawPhotometricInterpretation};

/// Fully decoded sensor data in 32-bit linear space (black/white scaled).
#[derive(Debug, Clone)]
pub struct DecodedRaw {
    pub profile: SensorProfile,
    pub mosaic: MosaicF32,
    pub source_sha256: String,
    pub source_path: String,
    /// EXIF capture metadata (camera, lens, exposure, time, GPS) for exports.
    pub metadata: CaptureMetadata,
}

pub fn from_rawler(
    path: &Path,
    raw: RawImage,
    digest: String,
    orientation: Orientation,
) -> Result<DecodedRaw> {
    let format =
        CameraFormat::from_extension(path.extension().and_then(|e| e.to_str()).unwrap_or(""))
            .refine_dng(&raw.clean_make, &raw.clean_model);

    let layout = match &raw.photometric {
        RawPhotometricInterpretation::Cfa(cfg) => SensorLayout::Cfa {
            cfa: cfa_from_rawler(&cfg.cfa),
        },
        RawPhotometricInterpretation::LinearRaw => SensorLayout::LinearRgb,
        RawPhotometricInterpretation::BlackIsZero => SensorLayout::Monochrome,
    };

    let xyz_to_cam = select_color_matrix(&raw);

    let data = match raw.data {
        RawImageData::Float(v) => v,
        RawImageData::Integer(_) => {
            return Err(Error::Decode(
                "expected floating-point samples after black/white scaling".into(),
            ));
        }
    };

    let expected = raw.width * raw.height * raw.cpp;
    if data.len() != expected {
        return Err(Error::InvalidImage {
            reason: format!(
                "pixel buffer length {} != {}×{}×{}",
                data.len(),
                raw.width,
                raw.height,
                raw.cpp
            ),
        });
    }

    // rawler leaves masked (black) sensor borders in place; crop to the recommended
    // area, or the active area when the camera has no recommended crop.
    let crop = raw
        .crop_area
        .or(raw.active_area)
        .map(|r| (r.p.x, r.p.y, r.d.w, r.d.h))
        .filter(|&(x, y, w, h)| w > 0 && h > 0 && x + w <= raw.width && y + h <= raw.height)
        .unwrap_or((0, 0, raw.width, raw.height));
    let (data, width, height) = crop_samples(data, raw.width, raw.cpp, crop);
    let layout = match layout {
        SensorLayout::Cfa { cfa } => SensorLayout::Cfa {
            cfa: shift_cfa(&cfa, crop.0, crop.1),
        },
        other => other,
    };

    let profile = SensorProfile {
        format,
        make: raw.make.clone(),
        model: raw.model.clone(),
        clean_make: raw.clean_make.clone(),
        clean_model: raw.clean_model.clone(),
        width: width as u32,
        height: height as u32,
        bits_per_sample: raw.bps as u32,
        samples_per_pixel: raw.cpp as u32,
        layout,
        as_shot_wb: raw.wb_coeffs,
        xyz_to_cam,
        orientation,
    };

    let cfa = match &profile.layout {
        SensorLayout::Cfa { cfa } => Some(cfa.clone()),
        _ => None,
    };

    Ok(DecodedRaw {
        profile,
        mosaic: MosaicF32 {
            width: width as u32,
            height: height as u32,
            data,
            samples_per_pixel: raw.cpp as u32,
            cfa,
        },
        source_sha256: digest,
        source_path: path.to_string_lossy().into_owned(),
        metadata: CaptureMetadata::default(),
    })
}

fn cfa_from_rawler(cfa: &rawler::cfa::CFA) -> CfaPattern {
    let mut pattern = Vec::with_capacity(cfa.width * cfa.height);
    for y in 0..cfa.height {
        for x in 0..cfa.width {
            pattern.push(cfa.color_at(y, x) as u8);
        }
    }
    CfaPattern {
        name: cfa.name.clone(),
        width: cfa.width,
        height: cfa.height,
        pattern,
    }
}

/// Copy the `(x, y, w, h)` window out of an interleaved `cpp`-channel buffer.
fn crop_samples(
    data: Vec<f32>,
    width: usize,
    cpp: usize,
    (x, y, w, h): (usize, usize, usize, usize),
) -> (Vec<f32>, usize, usize) {
    if x == 0 && y == 0 && w == width && data.len() == w * h * cpp {
        return (data, w, h);
    }
    let mut out = Vec::with_capacity(w * h * cpp);
    for row in y..y + h {
        let start = (row * width + x) * cpp;
        out.extend_from_slice(&data[start..start + w * cpp]);
    }
    (out, w, h)
}

/// Re-phase a CFA so `color_at(0, 0)` refers to the pixel at `(x, y)` of the uncropped sensor.
fn shift_cfa(cfa: &CfaPattern, x: usize, y: usize) -> CfaPattern {
    let mut pattern = Vec::with_capacity(cfa.pattern.len());
    for row in 0..cfa.height {
        for col in 0..cfa.width {
            pattern.push(cfa.color_at(row + y, col + x));
        }
    }
    CfaPattern {
        name: cfa.name.clone(),
        width: cfa.width,
        height: cfa.height,
        pattern,
    }
}

/// XYZ → camera matrix for daylight. rawler keeps one matrix per calibration illuminant
/// (DNG `ColorMatrix1/2`, or its camera database); the legacy `xyz_to_cam` field is
/// usually zero. Daylight calibrations are preferred because the working space is D65.
///
/// TODO: interpolate between the two DNG calibrations by white-balance temperature.
fn select_color_matrix(raw: &RawImage) -> [[f32; 3]; 3] {
    use rawler::imgop::xyz::Illuminant;
    let preference = [
        Illuminant::D65,
        Illuminant::D55,
        Illuminant::D50,
        Illuminant::Daylight,
        Illuminant::D75,
        Illuminant::FineWeather,
    ];
    let flat = preference
        .iter()
        .find_map(|ill| raw.color_matrix.get(ill))
        .or_else(|| raw.color_matrix.values().next())
        .filter(|m| m.len() >= 9);
    match flat {
        Some(m) => [[m[0], m[1], m[2]], [m[3], m[4], m[5]], [m[6], m[7], m[8]]],
        None => {
            let m = raw.xyz_to_cam;
            [m[0], m[1], m[2]]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_takes_the_requested_window() {
        // 4×3 single-channel sensor, values = index.
        let data: Vec<f32> = (0..12).map(|v| v as f32).collect();
        let (out, w, h) = crop_samples(data, 4, 1, (1, 1, 2, 2));
        assert_eq!((w, h), (2, 2));
        assert_eq!(out, vec![5.0, 6.0, 9.0, 10.0]);
    }

    #[test]
    fn odd_crop_offset_rephases_bayer() {
        let rggb = CfaPattern::rggb();
        // Cropping one column turns RGGB into GRBG.
        assert_eq!(shift_cfa(&rggb, 1, 0).pattern, vec![1, 0, 2, 1]);
        // Cropping one row and one column turns it into BGGR.
        assert_eq!(shift_cfa(&rggb, 1, 1).pattern, vec![2, 1, 1, 0]);
        assert_eq!(shift_cfa(&rggb, 2, 2).pattern, rggb.pattern);
    }
}
