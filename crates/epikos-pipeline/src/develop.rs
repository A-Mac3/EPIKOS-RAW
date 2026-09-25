use epikos_core::{ColorSpace, ImageRgbF32, MosaicF32, Result, SensorLayout, SensorProfile};
use epikos_sidecar::{Adjustments, DemosaicMode, DevelopDocument};

use crate::color_transform::{apply_white_balance, camera_to_linear_rec2020};
use crate::demosaic::{demosaic, DemosaicAlgorithm};
use crate::highlights::recover_highlights;
use crate::optics::{correct_chromatic_aberration, correct_distortion};
use crate::white_balance::gains_for_temperature;

/// Run the full 32-bit develop graph. The RAW buffer is never mutated on disk.
pub fn develop(
    mosaic: &MosaicF32,
    profile: &SensorProfile,
    doc: &DevelopDocument,
) -> Result<ImageRgbF32> {
    let adj = &doc.adjustments;
    let rgb = demosaic(mosaic, map_demosaic(adj, profile));
    develop_rgb(rgb, profile, adj)
}

/// Everything after demosaic: highlights → WB → optics → camera RGB → Rec.2020 → exposure.
///
/// Takes camera RGB at any resolution, so the full-size develop and the downsampled
/// interactive preview share one code path.
pub fn develop_rgb(
    mut rgb: ImageRgbF32,
    profile: &SensorProfile,
    adj: &Adjustments,
) -> Result<ImageRgbF32> {
    rgb.validate()?;

    if adj.highlight_recovery {
        recover_highlights(&mut rgb);
    }

    let monochrome = matches!(profile.layout, SensorLayout::Monochrome);
    if !monochrome {
        let wb = resolve_wb(profile, adj);
        apply_white_balance(&mut rgb, wb);
    }

    rgb = correct_chromatic_aberration(&rgb, &adj.lens.chromatic_aberration);
    rgb = correct_distortion(&rgb, &adj.lens.distortion);

    if monochrome {
        // R = G = B is already neutral in any RGB space; a colour matrix would tint it.
        rgb.space = ColorSpace::LinearRec2020;
    } else {
        camera_to_linear_rec2020(&mut rgb, profile);
    }

    if adj.exposure != 0.0 {
        let gain = 2f32.powf(adj.exposure.clamp(-10.0, 10.0));
        for plane in [&mut rgb.r, &mut rgb.g, &mut rgb.b] {
            plane.iter_mut().for_each(|v| *v *= gain);
        }
    }
    rgb.validate()?;
    Ok(rgb)
}

fn map_demosaic(adj: &Adjustments, profile: &SensorProfile) -> DemosaicAlgorithm {
    match adj.demosaic {
        DemosaicMode::Auto => {
            if profile.is_fuji_xtrans() {
                DemosaicAlgorithm::Xtrans
            } else {
                DemosaicAlgorithm::Auto
            }
        }
        DemosaicMode::Malvar => DemosaicAlgorithm::Malvar,
        DemosaicMode::Bilinear => DemosaicAlgorithm::Bilinear,
        DemosaicMode::Xtrans => DemosaicAlgorithm::Xtrans,
    }
}

fn resolve_wb(profile: &SensorProfile, adj: &Adjustments) -> [f32; 4] {
    match adj.white_balance.mode {
        epikos_sidecar::WbMode::AsShot => profile.as_shot_wb,
        epikos_sidecar::WbMode::Custom => gains_for_temperature(
            &profile.xyz_to_cam,
            adj.white_balance.temperature,
            adj.white_balance.tint,
        )
        .unwrap_or(profile.as_shot_wb),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::{
        CameraFormat, CfaPattern, ColorSpace, MosaicF32, Pixel, SensorLayout, SensorProfile,
    };
    use epikos_sidecar::SourceRef;

    fn synthetic_rggb() -> (MosaicF32, SensorProfile, DevelopDocument) {
        let w = 12u32;
        let h = 12u32;
        let cfa = CfaPattern::rggb();
        let mut data = vec![0.0; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let ch = cfa.color_at(y as usize, x as usize);
                data[(y * w + x) as usize] = match ch {
                    0 => 0.40,
                    2 => 0.20,
                    _ => 0.30,
                };
            }
        }
        let mosaic = MosaicF32 {
            width: w,
            height: h,
            data,
            samples_per_pixel: 1,
            cfa: Some(cfa),
        };
        let profile = SensorProfile {
            format: CameraFormat::SonyArw,
            make: "Sony".into(),
            model: "ILCE-7M4".into(),
            clean_make: "Sony".into(),
            clean_model: "ILCE-7M4".into(),
            width: w,
            height: h,
            bits_per_sample: 14,
            samples_per_pixel: 1,
            layout: SensorLayout::Cfa {
                cfa: CfaPattern::rggb(),
            },
            as_shot_wb: [1.0, 1.0, 1.0, 1.0],
            xyz_to_cam: [[0.0; 3]; 3],
        };
        let doc = DevelopDocument::new(SourceRef {
            path: "synthetic.ARW".into(),
            sha256: "0".into(),
            format: profile.format.label().into(),
            make: profile.make.clone(),
            model: profile.model.clone(),
        });
        (mosaic, profile, doc)
    }

    #[test]
    fn develop_emits_linear_rec2020() {
        let (mosaic, profile, doc) = synthetic_rggb();
        let rgb = develop(&mosaic, &profile, &doc).unwrap();
        assert_eq!(rgb.space, ColorSpace::LinearRec2020);
        assert_eq!(rgb.width, 12);
        let p = rgb.get(6, 6);
        assert!(p.r > 0.0 && p.g > 0.0 && p.b > 0.0);
    }

    /// End to end with a real camera matrix: a grey card lit by D65 must come out neutral.
    #[test]
    fn grey_card_under_d65_develops_neutral_with_real_matrix() {
        // Adobe DNG ColorMatrix2 (D65) for the Canon EOS 5D Mark IV.
        let xyz_to_cam = [
            [0.6446, -0.0366, -0.0864],
            [-0.4436, 1.2204, 0.2513],
            [-0.0952, 0.2496, 0.6348],
        ];
        let d65 = [0.9505_f32, 1.0, 1.089];
        let cam: Vec<f32> = xyz_to_cam
            .iter()
            .map(|row| 0.2 * (row[0] * d65[0] + row[1] * d65[1] + row[2] * d65[2]))
            .collect();
        let (mut mosaic, mut profile, doc) = synthetic_rggb();
        for (i, v) in mosaic.data.iter_mut().enumerate() {
            let (x, y) = (i % 12, i / 12);
            *v = cam[CfaPattern::rggb().color_at(y, x) as usize];
        }
        profile.xyz_to_cam = xyz_to_cam;
        // Camera as-shot multipliers for D65 equalise the three channels.
        profile.as_shot_wb = [cam[1] / cam[0], 1.0, cam[1] / cam[2], f32::NAN];

        let rgb = develop(&mosaic, &profile, &doc).unwrap();
        let p = rgb.get(6, 6);
        assert!(
            (p.r - p.g).abs() < 1e-3 && (p.b - p.g).abs() < 1e-3 && p.g.is_finite(),
            "{p:?}"
        );
    }

    #[test]
    fn monochrome_develops_to_neutral_grey() {
        let (_, mut profile, doc) = synthetic_rggb();
        profile.layout = SensorLayout::Monochrome;
        profile.as_shot_wb = [2.0, 1.0, 1.5, 1.0];
        profile.xyz_to_cam = [[0.6, 0.3, 0.1], [0.3, 0.8, -0.1], [0.0, -0.1, 1.1]];
        let mosaic = MosaicF32 {
            width: 8,
            height: 8,
            data: vec![0.25; 64],
            samples_per_pixel: 1,
            cfa: None,
        };
        let rgb = develop(&mosaic, &profile, &doc).unwrap();
        let p = rgb.get(4, 4);
        assert_eq!((p.r, p.g, p.b), (0.25, 0.25, 0.25));
    }

    #[test]
    fn sidecar_does_not_mutate_mosaic() {
        let (mosaic, profile, mut doc) = synthetic_rggb();
        let before = mosaic.data.clone();
        doc.adjustments.highlight_recovery = false;
        let _ = develop(&mosaic, &profile, &doc).unwrap();
        assert_eq!(before, mosaic.data);
        let _ = Pixel::new(0.0, 0.0, 0.0);
    }
}
