//! Output colour spaces for export: primaries, transfer curves and ICC v2 profiles.
//!
//! Export applies the same view transform as the on-screen preview (highlight shoulder,
//! then the space's transfer curve), so an sRGB export matches what the editor shows.

use epikos_core::{ColorSpace, ImageRgbF32};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::display::shoulder;
use crate::matrix::{invert, mul, Mat3, REC2020_TO_XYZ};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum OutputSpace {
    /// IEC 61966-2-1. Safe default for the web and most apps.
    #[default]
    Srgb,
    /// DCI-P3 primaries, D65 white, sRGB curve (Apple displays).
    DisplayP3,
    /// ROMM RGB, D50 white, gamma 1.8. Widest gamut; the usual Lightroom/Photoshop handoff.
    ProPhoto,
}

type Xy = (f64, f64);
const D65: Xy = (0.3127, 0.3290);
const D50: Xy = (0.3457, 0.3585);
/// ICC profile connection space white (D50) as XYZ.
const PCS_D50: [f64; 3] = [0.9642, 1.0, 0.8249];

impl OutputSpace {
    pub fn label(self) -> &'static str {
        match self {
            Self::Srgb => "sRGB IEC61966-2.1",
            Self::DisplayP3 => "Display P3",
            Self::ProPhoto => "ProPhoto RGB",
        }
    }

    fn primaries(self) -> ([Xy; 3], Xy) {
        match self {
            Self::Srgb => ([(0.64, 0.33), (0.30, 0.60), (0.15, 0.06)], D65),
            Self::DisplayP3 => ([(0.680, 0.320), (0.265, 0.690), (0.150, 0.060)], D65),
            Self::ProPhoto => ([(0.7347, 0.2653), (0.1596, 0.8404), (0.0366, 0.0001)], D50),
        }
    }

    /// Linear light → encoded value, both on [0, 1].
    pub fn encode(self, v: f32) -> f32 {
        let v = v.clamp(0.0, 1.0);
        match self {
            Self::Srgb | Self::DisplayP3 => {
                if v <= 0.003_130_8 {
                    12.92 * v
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                }
            }
            Self::ProPhoto => {
                if v < 1.0 / 512.0 {
                    16.0 * v
                } else {
                    v.powf(1.0 / 1.8)
                }
            }
        }
    }

    /// Encoded value → linear light (inverse of [`Self::encode`]).
    pub fn decode(self, e: f64) -> f64 {
        let e = e.clamp(0.0, 1.0);
        match self {
            Self::Srgb | Self::DisplayP3 => {
                if e <= 0.04045 {
                    e / 12.92
                } else {
                    ((e + 0.055) / 1.055).powf(2.4)
                }
            }
            Self::ProPhoto => {
                if e < 16.0 / 512.0 {
                    e / 16.0
                } else {
                    e.powf(1.8)
                }
            }
        }
    }

    /// Linear Rec.2020 (D65) → this space's linear RGB, chromatically adapted if needed.
    pub fn from_rec2020(self) -> Mat3 {
        let (prims, white) = self.primaries();
        let to_xyz = rgb_to_xyz(prims, white);
        let mut xyz = to_f64(&REC2020_TO_XYZ);
        if white != D65 {
            xyz = mul64(&bradford(D65, white), &xyz);
        }
        let from_xyz = invert(&to_f32(&to_xyz)).expect("primaries are independent");
        mul(&from_xyz, &to_f32(&xyz))
    }

    /// ICC v2 display profile (matrix/TRC) describing this space.
    pub fn icc_profile(self) -> Vec<u8> {
        let (prims, white) = self.primaries();
        // ICC colorants are D50-relative.
        let mut m = rgb_to_xyz(prims, white);
        if white != D50 {
            m = mul64(&bradford(white, D50), &m);
        }
        let trc: Vec<u16> = (0..1024)
            .map(|i| (self.decode(i as f64 / 1023.0) * 65535.0).round() as u16)
            .collect();
        icc::build(self.label(), [col(&m, 0), col(&m, 1), col(&m, 2)], &trc)
    }
}

/// Develop result (linear Rec.2020) → interleaved 16-bit RGB in `space`.
pub fn encode_rgb16(image: &ImageRgbF32, space: OutputSpace) -> Vec<u16> {
    debug_assert_eq!(image.space, ColorSpace::LinearRec2020);
    let m = space.from_rec2020();
    (0..image.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let (r, g, b) = (image.r[i], image.g[i], image.b[i]);
            let px = [
                m[0][0] * r + m[0][1] * g + m[0][2] * b,
                m[1][0] * r + m[1][1] * g + m[1][2] * b,
                m[2][0] * r + m[2][1] * g + m[2][2] * b,
            ];
            px.map(|v| (space.encode(shoulder(v)) * 65535.0).round() as u16)
        })
        .collect()
}

fn xy_to_xyz((x, y): Xy) -> [f64; 3] {
    [x / y, 1.0, (1.0 - x - y) / y]
}

/// Standard RGB → XYZ from chromaticities (columns scaled so RGB white = `white`).
fn rgb_to_xyz(prims: [Xy; 3], white: Xy) -> [[f64; 3]; 3] {
    let p = prims.map(xy_to_xyz);
    let m = [
        [p[0][0], p[1][0], p[2][0]],
        [p[0][1], p[1][1], p[2][1]],
        [p[0][2], p[1][2], p[2][2]],
    ];
    let s = apply64(&invert64(&m), xy_to_xyz(white));
    std::array::from_fn(|i| std::array::from_fn(|j| m[i][j] * s[j]))
}

/// Bradford chromatic adaptation from white `src` to white `dst`.
fn bradford(src: Xy, dst: Xy) -> [[f64; 3]; 3] {
    const B: [[f64; 3]; 3] = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let (s, d) = (apply64(&B, xy_to_xyz(src)), apply64(&B, xy_to_xyz(dst)));
    let scale: [[f64; 3]; 3] =
        std::array::from_fn(|i| std::array::from_fn(|j| if i == j { d[i] / s[i] } else { 0.0 }));
    mul64(&invert64(&B), &mul64(&scale, &B))
}

fn col(m: &[[f64; 3]; 3], j: usize) -> [f64; 3] {
    [m[0][j], m[1][j], m[2][j]]
}

fn mul64(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn apply64(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn invert64(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let s = 1.0 / det;
    [
        [
            (e * i - f * h) * s,
            (c * h - b * i) * s,
            (b * f - c * e) * s,
        ],
        [
            (f * g - d * i) * s,
            (a * i - c * g) * s,
            (c * d - a * f) * s,
        ],
        [
            (d * h - e * g) * s,
            (b * g - a * h) * s,
            (a * e - b * d) * s,
        ],
    ]
}

fn to_f32(m: &[[f64; 3]; 3]) -> Mat3 {
    m.map(|row| row.map(|v| v as f32))
}

fn to_f64(m: &Mat3) -> [[f64; 3]; 3] {
    m.map(|row| row.map(|v| v as f64))
}

/// Minimal ICC v2.1 RGB display profile writer (matrix/TRC model).
mod icc {
    use super::PCS_D50;

    pub fn build(description: &str, colorants: [[f64; 3]; 3], trc: &[u16]) -> Vec<u8> {
        let tags: Vec<([u8; 4], Vec<u8>)> = vec![
            (*b"desc", desc(description)),
            (*b"cprt", text("No copyright, use freely")),
            (*b"wtpt", xyz(PCS_D50)),
            (*b"rXYZ", xyz(colorants[0])),
            (*b"gXYZ", xyz(colorants[1])),
            (*b"bXYZ", xyz(colorants[2])),
            (*b"rTRC", curv(trc)),
        ];
        // gTRC and bTRC share rTRC's data (allowed by the spec).
        let entries = tags.len() + 2;
        let mut data = Vec::new();
        let mut table = Vec::new();
        let mut offset = 128 + 4 + 12 * entries;
        let mut trc_at = (0, 0);
        for (sig, body) in &tags {
            table.push((*sig, offset, body.len()));
            if sig == b"rTRC" {
                trc_at = (offset, body.len());
            }
            data.extend_from_slice(body);
            offset += body.len();
            while !offset.is_multiple_of(4) {
                data.push(0);
                offset += 1;
            }
        }
        table.push((*b"gTRC", trc_at.0, trc_at.1));
        table.push((*b"bTRC", trc_at.0, trc_at.1));

        let size = offset;
        let mut out = Vec::with_capacity(size);
        be32(&mut out, size as u32);
        out.extend_from_slice(&[0; 4]); // preferred CMM
        be32(&mut out, 0x0210_0000); // version 2.1
        out.extend_from_slice(b"mntr");
        out.extend_from_slice(b"RGB ");
        out.extend_from_slice(b"XYZ ");
        out.extend_from_slice(&[0; 12]); // creation date (unset)
        out.extend_from_slice(b"acsp");
        out.extend_from_slice(&[0; 4]); // platform
        out.extend_from_slice(&[0; 4]); // flags
        out.extend_from_slice(&[0; 4]); // manufacturer
        out.extend_from_slice(&[0; 4]); // model
        out.extend_from_slice(&[0; 8]); // attributes
        be32(&mut out, 0); // rendering intent: perceptual
        for v in PCS_D50 {
            s15f16(&mut out, v);
        }
        out.extend_from_slice(&[0; 4]); // creator
        out.extend_from_slice(&[0; 44]); // profile ID (v4) + reserved
        debug_assert_eq!(out.len(), 128);

        be32(&mut out, table.len() as u32);
        for (sig, off, len) in table {
            out.extend_from_slice(&sig);
            be32(&mut out, off as u32);
            be32(&mut out, len as u32);
        }
        out.extend_from_slice(&data);
        debug_assert_eq!(out.len(), size);
        out
    }

    fn be32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_be_bytes());
    }

    fn s15f16(out: &mut Vec<u8>, v: f64) {
        out.extend_from_slice(&((v * 65536.0).round() as i32).to_be_bytes());
    }

    fn xyz(v: [f64; 3]) -> Vec<u8> {
        let mut out = b"XYZ \0\0\0\0".to_vec();
        for c in v {
            s15f16(&mut out, c);
        }
        out
    }

    fn curv(table: &[u16]) -> Vec<u8> {
        let mut out = b"curv\0\0\0\0".to_vec();
        be32(&mut out, table.len() as u32);
        for v in table {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out
    }

    fn text(s: &str) -> Vec<u8> {
        let mut out = b"text\0\0\0\0".to_vec();
        out.extend_from_slice(s.as_bytes());
        out.push(0);
        out
    }

    /// v2 `textDescriptionType`: ASCII part, empty Unicode and ScriptCode parts.
    fn desc(s: &str) -> Vec<u8> {
        let mut out = b"desc\0\0\0\0".to_vec();
        be32(&mut out, s.len() as u32 + 1);
        out.extend_from_slice(s.as_bytes());
        out.push(0);
        out.extend_from_slice(&[0; 8]); // Unicode language code + count
        out.extend_from_slice(&[0; 3]); // ScriptCode code + count
        out.extend_from_slice(&[0; 67]); // ScriptCode string
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::apply;

    #[test]
    fn srgb_matrix_matches_the_published_one() {
        // IEC 61966-2-1 linear sRGB → XYZ (D65), first row.
        let (p, w) = OutputSpace::Srgb.primaries();
        let m = rgb_to_xyz(p, w);
        for (a, b) in m[0].iter().zip([0.4124, 0.3576, 0.1805]) {
            assert!((a - b).abs() < 5e-4, "{:?}", m[0]);
        }
    }

    #[test]
    fn neutral_stays_neutral_in_every_space() {
        for space in [
            OutputSpace::Srgb,
            OutputSpace::DisplayP3,
            OutputSpace::ProPhoto,
        ] {
            let v = apply(&space.from_rec2020(), [0.4, 0.4, 0.4]);
            assert!(
                v.iter().all(|c| (c - 0.4).abs() < 2e-3),
                "{space:?} grey became {v:?}"
            );
        }
    }

    #[test]
    fn transfer_curves_round_trip() {
        for space in [
            OutputSpace::Srgb,
            OutputSpace::DisplayP3,
            OutputSpace::ProPhoto,
        ] {
            for i in 0..=100 {
                let v = i as f32 / 100.0;
                let back = space.decode(space.encode(v) as f64) as f32;
                assert!((back - v).abs() < 1e-4, "{space:?} {v} → {back}");
            }
        }
    }

    #[test]
    fn icc_profile_is_well_formed() {
        let icc = OutputSpace::ProPhoto.icc_profile();
        assert_eq!(
            u32::from_be_bytes(icc[0..4].try_into().unwrap()) as usize,
            icc.len()
        );
        assert_eq!(&icc[36..40], b"acsp");
        let count = u32::from_be_bytes(icc[128..132].try_into().unwrap()) as usize;
        assert_eq!(count, 9);
        for k in 0..count {
            let e = 132 + 12 * k;
            let off = u32::from_be_bytes(icc[e + 4..e + 8].try_into().unwrap()) as usize;
            let len = u32::from_be_bytes(icc[e + 8..e + 12].try_into().unwrap()) as usize;
            assert!(
                off.is_multiple_of(4) && off + len <= icc.len(),
                "tag {k} out of bounds"
            );
        }
    }
}
