//! The camera's own lens corrections from a DNG's `OpcodeList3` (applied after
//! demosaicing): `WarpRectilinear` (distortion and lateral CA) and `FixVignetteRadial`.
//! Leica and Apple ProRAW DNGs carry them; rawler reads but does not apply them.

use epikos_core::{LensGeometry, LensProfile, LensVignetting};
use rawler::decoders::{Decoder, WellKnownIFD};
use rawler::tags::DngTag;

const WARP_RECTILINEAR: u32 = 1;
const FIX_VIGNETTE_RADIAL: u32 = 3;

/// The corrections in the DNG's `OpcodeList3`, if it has any we understand.
pub(crate) fn lens_profile(decoder: &dyn Decoder) -> Option<LensProfile> {
    let ifd = decoder.ifd(WellKnownIFD::VirtualDngRawTags).ok().flatten()?;
    let entry = ifd.get_entry(DngTag::OpcodeList3)?;
    let bytes = match &entry.value {
        rawler::formats::tiff::Value::Undefined(b) | rawler::formats::tiff::Value::Byte(b) => b.as_slice(),
        _ => return None,
    };
    let profile = parse_opcode_list(bytes);
    (!profile.is_empty()).then_some(profile)
}

/// DNG opcode lists are always big-endian: a count, then per opcode its id, DNG
/// version, flags, parameter byte count and parameters.
fn parse_opcode_list(bytes: &[u8]) -> LensProfile {
    let mut profile = LensProfile { source: "Camera (DNG)".into(), geometry: None, vignetting: None };
    let mut r = Reader { bytes, pos: 0 };
    let Some(count) = r.u32() else { return profile };
    for _ in 0..count.min(64) {
        let (Some(id), Some(_version), Some(_flags), Some(len)) = (r.u32(), r.u32(), r.u32(), r.u32()) else {
            break;
        };
        let Some(params) = r.take(len as usize) else { break };
        let mut p = Reader { bytes: params, pos: 0 };
        match id {
            WARP_RECTILINEAR => {
                let Some(n) = p.u32().filter(|n| (1..=4).contains(n)) else { continue };
                let planes: Option<Vec<[f64; 6]>> =
                    (0..n).map(|_| Some([p.f64()?, p.f64()?, p.f64()?, p.f64()?, p.f64()?, p.f64()?])).collect();
                if let (Some(planes), Some(cx), Some(cy)) = (planes, p.f64(), p.f64()) {
                    profile.geometry = Some(LensGeometry::Dng { center: [cx, cy], planes });
                }
            }
            FIX_VIGNETTE_RADIAL => {
                let k = [p.f64(), p.f64(), p.f64(), p.f64(), p.f64()];
                if let ([Some(k0), Some(k1), Some(k2), Some(k3), Some(k4)], Some(cx), Some(cy)) = (k, p.f64(), p.f64()) {
                    profile.vignetting = Some(LensVignetting::Dng { center: [cx, cy], k: [k0, k1, k2, k3, k4] });
                }
            }
            _ => {}
        }
    }
    profile
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let out = self.bytes.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(out)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_be_bytes(self.take(8)?.try_into().ok()?)).filter(|v| v.is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opcode(id: u32, params: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for v in [id, 0x0103_0000, 1, params.len() as u32] {
            out.extend(v.to_be_bytes());
        }
        out.extend(params);
        out
    }

    fn f64s(v: &[f64]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }

    #[test]
    fn reads_warp_and_vignette_and_skips_the_rest() {
        let mut warp = 1u32.to_be_bytes().to_vec();
        warp.extend(f64s(&[1.0, -0.02, 0.003, 0.0, 0.0, 0.0, 0.5, 0.48]));
        let vignette = f64s(&[0.3, 0.1, 0.0, 0.0, 0.0, 0.5, 0.5]);
        let mut list = 3u32.to_be_bytes().to_vec();
        list.extend(opcode(9, &[0; 12])); // an opcode we don't handle
        list.extend(opcode(WARP_RECTILINEAR, &warp));
        list.extend(opcode(FIX_VIGNETTE_RADIAL, &vignette));
        let p = parse_opcode_list(&list);
        assert_eq!(
            p.geometry,
            Some(LensGeometry::Dng { center: [0.5, 0.48], planes: vec![[1.0, -0.02, 0.003, 0.0, 0.0, 0.0]] })
        );
        assert_eq!(p.vignetting, Some(LensVignetting::Dng { center: [0.5, 0.5], k: [0.3, 0.1, 0.0, 0.0, 0.0] }));
    }

    #[test]
    fn truncated_lists_are_ignored_not_fatal() {
        let mut list = 1u32.to_be_bytes().to_vec();
        list.extend(opcode(WARP_RECTILINEAR, &[0, 0, 0, 1, 0, 0]));
        assert!(parse_opcode_list(&list).is_empty());
        assert!(parse_opcode_list(&[0, 0]).is_empty());
    }
}
