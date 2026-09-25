//! Layered 16-bit PSD writer for the Step 8 handoff ("smart-linked PSD").
//!
//! Layers, bottom to top:
//! - the developed image as a normal pixel layer ("EPIKOS RAW"),
//! - one empty layer group per AI mask ("Subject", "Sky", …), whose group mask is
//!   that mask. Anything the photographer drops into a group in Photoshop (a Curves
//!   layer, a Hue/Saturation layer…) is confined to the mask, which is how the masks
//!   stay live instead of being flattened into the pixels.
//!
//! Capture metadata travels as EXIF (resource 1058) and XMP (resource 1060).
//!
//! Follows the Adobe Photoshop File Format specification: big-endian throughout,
//! 16-bit layer data in the `Lr16` tagged block (the plain layer-info section stays
//! empty, as Photoshop itself writes it), layer channels ZIP-compressed with
//! prediction (compression 3), and the composite as raw planar data so any reader,
//! including Lightroom and macOS, can show the flattened image.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use flate2::write::ZlibEncoder;
use flate2::Compression;
use rayon::prelude::*;

/// A mask to become a masked layer group: 16-bit coverage, 65535 = inside.
pub(crate) struct MaskLayer<'a> {
    pub name: &'a str,
    pub mask: &'a [u16],
}

pub(crate) struct PsdImage<'a> {
    pub width: u32,
    pub height: u32,
    /// Interleaved 16-bit RGB.
    pub rgb: &'a [u16],
    pub icc: &'a [u8],
    /// EXIF block (TIFF-structured), image resource 1058.
    pub exif: &'a [u8],
    /// XMP packet, image resource 1060.
    pub xmp: &'a [u8],
    pub ppi: u32,
    pub masks: Vec<MaskLayer<'a>>,
}

/// Channel image data of one layer channel, ready to write.
struct ChannelData {
    id: i16,
    bytes: Vec<u8>,
}

pub(crate) fn write_psd(path: &Path, img: &PsdImage) -> std::io::Result<()> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);

    // Header.
    out.write_all(b"8BPS")?;
    out.write_all(&1u16.to_be_bytes())?;
    out.write_all(&[0; 6])?;
    out.write_all(&3u16.to_be_bytes())?; // composite channels
    out.write_all(&img.height.to_be_bytes())?;
    out.write_all(&img.width.to_be_bytes())?;
    out.write_all(&16u16.to_be_bytes())?;
    out.write_all(&3u16.to_be_bytes())?; // RGB
    out.write_all(&0u32.to_be_bytes())?; // colour mode data

    // Image resources: ICC profile (1039), resolution (1005), and the capture metadata
    // as EXIF (1058, read by Photoshop) and XMP (1060, read by Lightroom and Bridge).
    let mut res = Vec::new();
    resource(&mut res, 1039, img.icc);
    if !img.exif.is_empty() {
        resource(&mut res, 1058, img.exif);
    }
    if !img.xmp.is_empty() {
        resource(&mut res, 1060, img.xmp);
    }
    let fixed = (img.ppi << 16).to_be_bytes();
    let mut resolution = Vec::new();
    for _ in 0..2 {
        resolution.extend_from_slice(&fixed);
        resolution.extend_from_slice(&1u16.to_be_bytes()); // pixels per inch
        resolution.extend_from_slice(&1u16.to_be_bytes()); // display in inches
    }
    resource(&mut res, 1005, &resolution);
    out.write_all(&(res.len() as u32).to_be_bytes())?;
    out.write_all(&res)?;

    // Layer and mask information.
    let layers = layer_info(img, w, h);
    let mut lm = Vec::new();
    lm.extend_from_slice(&0u32.to_be_bytes()); // layer info: in Lr16 instead
    lm.extend_from_slice(&0u32.to_be_bytes()); // global layer mask info
    lm.extend_from_slice(b"8BIMLr16");
    lm.extend_from_slice(&(layers.len() as u32).to_be_bytes());
    lm.extend_from_slice(&layers);
    out.write_all(&(lm.len() as u32).to_be_bytes())?;
    out.write_all(&lm)?;
    drop(lm);

    // Composite: raw planar, big-endian.
    out.write_all(&0u16.to_be_bytes())?;
    for c in 0..3 {
        let plane: Vec<u8> = (0..w * h).flat_map(|i| img.rgb[i * 3 + c].to_be_bytes()).collect();
        out.write_all(&plane)?;
    }
    out.flush()
}

/// Layer records plus channel image data (the body of `Lr16`), padded to 4 bytes.
fn layer_info(img: &PsdImage, w: usize, h: usize) -> Vec<u8> {
    let full = [0i32, 0, h as i32, w as i32];
    let empty = [0i32; 4];

    // The pixel layer: opaque transparency channel plus R, G, B.
    let opaque = vec![u16::MAX; w * h];
    let planes: Vec<Vec<u16>> = (0..3).map(|c| (0..w * h).map(|i| img.rgb[i * 3 + c]).collect()).collect();
    let pixel_channels: Vec<ChannelData> = [(-1i16, &opaque), (0, &planes[0]), (1, &planes[1]), (2, &planes[2])]
        .into_par_iter()
        .map(|(id, plane)| ChannelData { id, bytes: zip_predicted(plane, w) })
        .collect();
    drop((opaque, planes));

    let no_pixels = || -> Vec<ChannelData> {
        [-1i16, 0, 1, 2].map(|id| ChannelData { id, bytes: 0u16.to_be_bytes().to_vec() }).into()
    };

    let mut records = Vec::new();
    let mut data = Vec::new();
    let mut count = 0i16;
    let mut push = |records: &mut Vec<u8>, rect, channels: Vec<ChannelData>, blend: &[u8; 4], flags, mask, name: &str, section: Option<&[u8]>| {
        layer_record(records, rect, &channels, blend, flags, mask, name, section);
        for c in channels {
            data.extend_from_slice(&c.bytes);
        }
        count += 1;
    };

    push(&mut records, full, pixel_channels, b"norm", 0x08, None, "EPIKOS RAW", None);
    for m in &img.masks {
        // A group is two records: the hidden end marker below, the folder above it.
        push(&mut records, empty, no_pixels(), b"norm", 0x18, None, "</Layer group>", Some(&3u32.to_be_bytes()));
        let mut channels = no_pixels();
        channels.push(ChannelData { id: -2, bytes: zip_predicted(m.mask, w) });
        let mut folder = Vec::new();
        folder.extend_from_slice(&1u32.to_be_bytes()); // open folder
        folder.extend_from_slice(b"8BIMpass");
        push(&mut records, empty, channels, b"pass", 0x08, Some(full), m.name, Some(&folder));
    }

    let mut out = Vec::with_capacity(2 + records.len() + data.len());
    out.extend_from_slice(&count.to_be_bytes());
    out.extend_from_slice(&records);
    out.extend_from_slice(&data);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn layer_record(
    out: &mut Vec<u8>,
    rect: [i32; 4],
    channels: &[ChannelData],
    blend: &[u8; 4],
    flags: u8,
    mask: Option<[i32; 4]>,
    name: &str,
    section: Option<&[u8]>,
) {
    for v in rect {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&(channels.len() as u16).to_be_bytes());
    for c in channels {
        out.extend_from_slice(&c.id.to_be_bytes());
        out.extend_from_slice(&(c.bytes.len() as u32).to_be_bytes());
    }
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(blend);
    out.extend_from_slice(&[255, 0, flags, 0]); // opacity, clipping, flags, filler

    let mut extra = Vec::new();
    match mask {
        Some(r) => {
            extra.extend_from_slice(&20u32.to_be_bytes());
            for v in r {
                extra.extend_from_slice(&v.to_be_bytes());
            }
            extra.extend_from_slice(&[0, 0, 0, 0]); // default colour black, flags, padding
        }
        None => extra.extend_from_slice(&0u32.to_be_bytes()),
    }
    // Blending ranges: composite grey plus four channels, all "0–65535 blends".
    extra.extend_from_slice(&40u32.to_be_bytes());
    for _ in 0..10 {
        extra.extend_from_slice(&0x0000_ffffu32.to_be_bytes());
    }
    // Pascal name padded to a multiple of 4 (Latin-1; the full name is in `luni`).
    let latin: Vec<u8> = name.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).take(255).collect();
    extra.push(latin.len() as u8);
    extra.extend_from_slice(&latin);
    while extra.len() % 4 != 0 {
        extra.push(0);
    }
    // Unicode name.
    let utf16: Vec<u16> = name.encode_utf16().collect();
    let mut luni = (utf16.len() as u32).to_be_bytes().to_vec();
    luni.extend(utf16.iter().flat_map(|u| u.to_be_bytes()));
    tagged(&mut extra, b"luni", &luni);
    if let Some(s) = section {
        tagged(&mut extra, b"lsct", s);
    }

    out.extend_from_slice(&(extra.len() as u32).to_be_bytes());
    out.extend_from_slice(&extra);
}

/// Additional-layer-information block, padded to 4 bytes.
fn tagged(out: &mut Vec<u8>, key: &[u8; 4], data: &[u8]) {
    let padded = data.len().div_ceil(4) * 4;
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(key);
    out.extend_from_slice(&(padded as u32).to_be_bytes());
    out.extend_from_slice(data);
    out.resize(out.len() + padded - data.len(), 0);
}

/// Image resource block: signature, id, empty name, size, data padded to even.
fn resource(out: &mut Vec<u8>, id: u16, data: &[u8]) {
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

/// Compression 3: per row, each 16-bit sample minus the previous one, big-endian,
/// then zlib. Prefixed with the compression code.
fn zip_predicted(plane: &[u16], width: usize) -> Vec<u8> {
    let mut raw = Vec::with_capacity(plane.len() * 2);
    for row in plane.chunks(width.max(1)) {
        let mut prev = 0u16;
        for &v in row {
            raw.extend_from_slice(&v.wrapping_sub(prev).to_be_bytes());
            prev = v;
        }
    }
    let mut z = ZlibEncoder::new(3u16.to_be_bytes().to_vec(), Compression::fast());
    z.write_all(&raw).expect("writing to memory cannot fail");
    z.finish().expect("writing to memory cannot fail")
}
