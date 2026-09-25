//! Session layer shared by the desktop app and the CLI.
//!
//! Keeps recently opened RAW files decoded in memory, renders fast downsampled previews
//! for interactive editing, and owns sidecar load/save policy. All methods are
//! synchronous and thread-safe; callers run them off the UI thread.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use epikos_core::{CameraFormat, Error, ImageRgbF32, Result, SensorLayout};
use epikos_decode::{decode_file, embedded_thumbnail, DecodedRaw};
use epikos_pipeline::{
    bin_mosaic, block_for_size, develop_rgb, temperature_for_gains, to_display_srgb, DisplayImage,
};
use epikos_sidecar::{
    load_json, load_xmp, save_json, save_xmp, sidecar_json_path, sidecar_xmp_path, Adjustments,
    DevelopDocument, SourceRef,
};
use serde::Serialize;

mod export;
pub use epikos_pipeline::OutputSpace;
pub use export::ExportReport;

/// A RAW/DNG file found while browsing a folder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub format: String,
    pub has_edits: bool,
}

/// Everything the editor needs when an image is opened.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    pub path: String,
    pub name: String,
    pub format: String,
    pub make: String,
    pub model: String,
    /// Upright (display-oriented) size in pixels.
    pub width: u32,
    pub height: u32,
    pub monochrome: bool,
    /// Camera as-shot white balance expressed as temperature/tint, to seed the sliders.
    pub as_shot: Option<TemperatureTint>,
    pub document: DevelopDocument,
    /// Which sidecar the document was loaded from, if any.
    pub loaded_from: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct TemperatureTint {
    pub temperature: f32,
    pub tint: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveReport {
    pub json_path: String,
    pub xmp_path: Option<String>,
    /// Set when the XMP could not be written (e.g. a Lightroom XMP already exists).
    pub warning: Option<String>,
}

struct Loaded {
    raw: DecodedRaw,
    /// Binned camera-RGB preview bases, keyed by block size.
    bases: Mutex<Vec<(u32, Arc<ImageRgbF32>)>>,
}

impl Loaded {
    fn base(&self, max_w: u32, max_h: u32) -> Arc<ImageRgbF32> {
        let block = block_for_size(&self.raw.mosaic, max_w, max_h);
        let mut bases = self.bases.lock().unwrap();
        if let Some((_, img)) = bases.iter().find(|(b, _)| *b == block) {
            return img.clone();
        }
        let img = Arc::new(bin_mosaic(&self.raw.mosaic, block));
        bases.push((block, img.clone()));
        img
    }
}

pub struct Engine {
    capacity: usize,
    cache: Mutex<VecDeque<(PathBuf, Arc<Loaded>)>>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(3)
    }
}

impl Engine {
    /// `capacity` decoded images are kept in memory (≈ 4 bytes × sensor pixels each).
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            cache: Mutex::new(VecDeque::new()),
        }
    }

    /// Supported RAW/DNG files directly inside `dir`, sorted by name.
    pub fn list_folder(&self, dir: &Path) -> Result<Vec<FileEntry>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let format = CameraFormat::from_extension(ext);
            if format == CameraFormat::Unknown {
                continue;
            }
            entries.push(FileEntry {
                path: path.to_string_lossy().into_owned(),
                name: name.to_string(),
                format: format.label().to_string(),
                has_edits: sidecar_json_path(&path).exists(),
            });
        }
        entries.sort_by_key(|a| a.name.to_lowercase());
        Ok(entries)
    }

    /// Decode (or fetch from cache) and load the edit document for `path`.
    ///
    /// Sidecar precedence: EPIKOS JSON, then XMP (EPIKOS or Camera Raw white balance),
    /// then defaults.
    pub fn open(&self, path: &Path) -> Result<ImageInfo> {
        let loaded = self.load(path)?;
        let raw = &loaded.raw;
        let source = source_ref(raw);

        let json = sidecar_json_path(path);
        let xmp = sidecar_xmp_path(path);
        let (mut document, loaded_from) = if json.exists() {
            (load_json(&json)?, Some(json))
        } else if xmp.exists() {
            (load_xmp(&xmp)?, Some(xmp))
        } else {
            (DevelopDocument::new(source.clone()), None)
        };
        // The sidecar may have moved with the file; describe the file actually opened.
        document.source = source;

        let p = &raw.profile;
        let monochrome = matches!(p.layout, SensorLayout::Monochrome);
        let upright_swapped = p.orientation.swaps_axes();
        let as_shot = (!monochrome)
            .then(|| temperature_for_gains(&p.xyz_to_cam, p.as_shot_wb))
            .flatten()
            .map(|(temperature, tint)| TemperatureTint { temperature, tint });

        Ok(ImageInfo {
            path: raw.source_path.clone(),
            name: file_name(path),
            format: p.format.label().to_string(),
            make: p.clean_make.clone(),
            model: p.clean_model.clone(),
            width: if upright_swapped { p.height } else { p.width },
            height: if upright_swapped { p.width } else { p.height },
            monochrome,
            as_shot,
            document,
            loaded_from: loaded_from.map(|p| p.to_string_lossy().into_owned()),
        })
    }

    /// Develop a preview no larger than `max_w × max_h` for interactive editing.
    pub fn render_preview(
        &self,
        path: &Path,
        adjustments: &Adjustments,
        max_w: u32,
        max_h: u32,
    ) -> Result<DisplayImage> {
        let loaded = self.load(path)?;
        render(&loaded, adjustments, max_w, max_h)
    }

    /// Write the JSON sidecar (canonical) and, when safe, the XMP sidecar.
    pub fn save(&self, path: &Path, document: &DevelopDocument) -> Result<SaveReport> {
        let json = sidecar_json_path(path);
        save_json(&json, document)?;
        let xmp = sidecar_xmp_path(path);
        let (xmp_path, warning) = match save_xmp(&xmp, document) {
            Ok(()) => (Some(xmp.to_string_lossy().into_owned()), None),
            Err(Error::Sidecar(msg)) => (None, Some(msg)),
            Err(e) => return Err(e),
        };
        Ok(SaveReport {
            json_path: json.to_string_lossy().into_owned(),
            xmp_path,
            warning,
        })
    }

    /// Develop at full resolution and write a 16-bit TIFF in `space` to `dest`.
    pub fn export_tiff(
        &self,
        path: &Path,
        adjustments: &Adjustments,
        dest: &Path,
        space: OutputSpace,
    ) -> Result<ExportReport> {
        let loaded = self.load(path)?;
        export::export_tiff(&loaded, adjustments, dest, space)
    }

    /// JPEG thumbnail: the camera's embedded preview when available, otherwise a
    /// default EPIKOS develop.
    pub fn thumbnail_jpeg(&self, path: &Path, max_side: u32) -> Result<Vec<u8>> {
        let rgb = match embedded_thumbnail(path, max_side) {
            Ok(img) => img,
            Err(_) => {
                let d = self.render_preview(path, &Adjustments::default(), max_side, max_side)?;
                rgba_to_rgb(&d)
            }
        };
        encode_jpeg(&rgb)
    }

    fn load(&self, path: &Path) -> Result<Arc<Loaded>> {
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(i) = cache.iter().position(|(p, _)| p == path) {
                let entry = cache.remove(i).unwrap();
                let loaded = entry.1.clone();
                cache.push_front(entry);
                return Ok(loaded);
            }
        }
        // Decode outside the lock so other images stay responsive.
        let loaded = Arc::new(Loaded {
            raw: decode_file(path)?,
            bases: Mutex::new(Vec::new()),
        });
        let mut cache = self.cache.lock().unwrap();
        cache.retain(|(p, _)| p != path);
        cache.push_front((path.to_path_buf(), loaded.clone()));
        cache.truncate(self.capacity);
        Ok(loaded)
    }
}

fn render(
    loaded: &Loaded,
    adjustments: &Adjustments,
    max_w: u32,
    max_h: u32,
) -> Result<DisplayImage> {
    let base = loaded.base(max_w, max_h);
    let developed = develop_rgb((*base).clone(), &loaded.raw.profile, adjustments)?;
    Ok(to_display_srgb(&developed))
}

fn source_ref(raw: &DecodedRaw) -> SourceRef {
    SourceRef {
        path: raw.source_path.clone(),
        sha256: raw.source_sha256.clone(),
        format: raw.profile.format.label().to_string(),
        make: raw.profile.clean_make.clone(),
        model: raw.profile.clean_model.clone(),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn rgba_to_rgb(d: &DisplayImage) -> image::RgbImage {
    let rgb: Vec<u8> = d
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    image::RgbImage::from_raw(d.width, d.height, rgb).expect("RGBA buffer matches dimensions")
}

fn encode_jpeg(img: &image::RgbImage) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85)
        .encode_image(img)
        .map_err(|e| Error::Decode(format!("jpeg encode: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use epikos_core::{CfaPattern, MosaicF32, SensorProfile};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("epikos-engine-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn synthetic_loaded(w: u32, h: u32) -> Loaded {
        let cfa = CfaPattern::rggb();
        let data = vec![0.3; (w * h) as usize];
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
            layout: SensorLayout::Cfa { cfa: cfa.clone() },
            as_shot_wb: [1.0, 1.0, 1.0, 1.0],
            xyz_to_cam: [[0.0; 3]; 3],
            orientation: Default::default(),
        };
        Loaded {
            raw: DecodedRaw {
                profile,
                mosaic: MosaicF32 {
                    width: w,
                    height: h,
                    data,
                    samples_per_pixel: 1,
                    cfa: Some(cfa),
                },
                source_sha256: String::new(),
                source_path: "synthetic.ARW".into(),
            },
            bases: Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn preview_fits_requested_size_and_reuses_base() {
        let loaded = synthetic_loaded(600, 400);
        let d = render(&loaded, &Adjustments::default(), 160, 160).unwrap();
        assert!(
            d.width <= 160 && d.height <= 160,
            "{}×{}",
            d.width,
            d.height
        );
        assert_eq!(d.rgba.len(), (d.width * d.height * 4) as usize);
        let _ = render(&loaded, &Adjustments::default(), 160, 160).unwrap();
        assert_eq!(loaded.bases.lock().unwrap().len(), 1);
    }

    #[test]
    fn exposure_brightens_preview() {
        let loaded = synthetic_loaded(64, 64);
        let dark = render(&loaded, &Adjustments::default(), 32, 32).unwrap();
        let bright = render(
            &loaded,
            &Adjustments {
                exposure: 1.0,
                ..Adjustments::default()
            },
            32,
            32,
        )
        .unwrap();
        assert!(bright.rgba[1] > dark.rgba[1]);
    }

    #[test]
    fn export_writes_a_16_bit_tiff_with_icc_and_no_partial_file() {
        use tiff::decoder::Decoder;
        use tiff::tags::Tag;

        let dir = temp_dir("export");
        let dest = dir.join("out.tif");
        let loaded = synthetic_loaded(64, 48);
        let report = export::export_tiff(
            &loaded,
            &Adjustments::default(),
            &dest,
            OutputSpace::ProPhoto,
        )
        .unwrap();
        assert_eq!((report.width, report.height), (64, 48));
        assert!(!dir.join("out.tif.partial").exists());

        let mut dec = Decoder::new(fs::File::open(&dest).unwrap()).unwrap();
        assert_eq!(dec.dimensions().unwrap(), (64, 48));
        assert_eq!(dec.colortype().unwrap(), tiff::ColorType::RGB(16));
        let icc = dec.get_tag_u8_vec(Tag::IccProfile).unwrap();
        assert_eq!(&icc[36..40], b"acsp");
        assert_eq!(icc, OutputSpace::ProPhoto.icc_profile());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_refuses_non_tiff_and_the_source_file() {
        let dir = temp_dir("export-guard");
        let raw = dir.join("IMG.ARW");
        fs::write(&raw, b"").unwrap();
        let mut loaded = synthetic_loaded(8, 8);
        loaded.raw.source_path = raw.to_string_lossy().into_owned();
        let adj = Adjustments::default();
        assert!(export::export_tiff(&loaded, &adj, &dir.join("x.jpg"), OutputSpace::Srgb).is_err());
        assert!(export::export_tiff(&loaded, &adj, &raw, OutputSpace::Srgb).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_folder_finds_raw_files_only() {
        let dir = temp_dir("list");
        for name in ["b.NEF", "a.cr3", "notes.txt", ".hidden.ARW", "c.dng"] {
            fs::write(dir.join(name), b"").unwrap();
        }
        fs::write(dir.join("a.cr3.epikos.json"), b"{}").unwrap();
        let names: Vec<_> = Engine::default()
            .list_folder(&dir)
            .unwrap()
            .into_iter()
            .map(|e| (e.name, e.has_edits))
            .collect();
        assert_eq!(
            names,
            vec![
                ("a.cr3".into(), true),
                ("b.NEF".into(), false),
                ("c.dng".into(), false)
            ]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_keeps_a_foreign_xmp_and_reports_it() {
        let dir = temp_dir("save");
        let raw = dir.join("IMG_1.CR3");
        let foreign = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core"/>"#;
        fs::write(dir.join("IMG_1.xmp"), foreign).unwrap();
        let doc = DevelopDocument::new(SourceRef {
            path: raw.to_string_lossy().into_owned(),
            sha256: String::new(),
            format: "Canon CR3".into(),
            make: "Canon".into(),
            model: "EOS R5".into(),
        });
        let report = Engine::default().save(&raw, &doc).unwrap();
        assert!(report.xmp_path.is_none() && report.warning.is_some());
        assert!(Path::new(&report.json_path).exists());
        assert_eq!(fs::read_to_string(dir.join("IMG_1.xmp")).unwrap(), foreign);
        fs::remove_dir_all(&dir).unwrap();
    }
}
