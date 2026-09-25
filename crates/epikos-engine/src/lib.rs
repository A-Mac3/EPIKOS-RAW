//! Session layer shared by the desktop app and the CLI.
//!
//! Keeps recently opened RAW files decoded in memory, renders fast downsampled previews
//! for interactive editing, runs the Step 3 AI masks, and owns sidecar load/save
//! policy. All methods are
//! synchronous and thread-safe; callers run them off the UI thread.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use epikos_core::{CameraFormat, Error, ImageRgbF32, Result, SensorLayout};
use epikos_decode::{decode_file, embedded_thumbnail, DecodedRaw};
use epikos_masks::{DepthMap, RgbImage};
use epikos_pipeline::{
    bin_mosaic, block_for_size, develop_rgb_with, look_needs_depth, skin_likelihood, temperature_for_gains,
    to_display_srgb, DepthPlane, DisplayImage, LookInputs,
};
use epikos_sidecar::{
    load_json, load_xmp, save_json, save_xmp, sidecar_json_path, sidecar_xmp_path, Adjustments,
    DevelopDocument, SourceRef,
};
use serde::Serialize;

mod dng;
mod export;
mod prompt;
mod psd;
pub use epikos_masks::{DepthMap as Depth, Mask, MaskKind, Masker, ModelStatus};
pub use epikos_pipeline::{styles, OutputSpace, StyleInfo};
pub use export::{ExportFormat, ExportOptions, ExportReport};
pub use prompt::{interpret_look, LookPrompt, PromptMatch};

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
    /// Camera, lens, exposure, capture time and GPS from the RAW's EXIF.
    pub capture: epikos_core::CaptureMetadata,
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

/// Longest side of the image handed to the mask models. IS-Net works at 1024², so more
/// detail wouldn't reach it.
const MASK_INPUT_SIDE: u32 = 1024;

/// Where the mask models are expected and which of them are present.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskModels {
    pub dir: String,
    pub models: Vec<ModelStatus>,
    /// The Step 6 depth model.
    pub depth: DepthModel,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepthModel {
    pub file: String,
    pub available: bool,
}

struct Loaded {
    raw: DecodedRaw,
    /// Binned camera-RGB preview bases, keyed by block size.
    bases: Mutex<Vec<(u32, Arc<ImageRgbF32>)>>,
    /// Most recent mask of each kind, upright, at up to [`MASK_INPUT_SIDE`].
    masks: Mutex<Vec<Arc<Mask>>>,
    /// Depth map and the lens geometry it was estimated for.
    depth: Mutex<Option<(String, Arc<DepthMap>)>>,
}

impl Loaded {
    fn base(&self, max_w: u32, max_h: u32) -> Arc<ImageRgbF32> {
        let block = block_for_size(&self.raw.mosaic, max_w, max_h);
        let mut bases = self.bases.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, img)) = bases.iter().find(|(b, _)| *b == block) {
            return img.clone();
        }
        let img = Arc::new(bin_mosaic(&self.raw.mosaic, block));
        bases.push((block, img.clone()));
        img
    }
}

/// Every lock recovers from poisoning (`PoisonError::into_inner`): the data behind
/// them are caches that stay valid, so one panicking request must not make every
/// later one panic too.
pub struct Engine {
    capacity: usize,
    cache: Mutex<VecDeque<(PathBuf, Arc<Loaded>)>>,
    masker: Masker,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(3)
    }
}

/// `$EPIKOS_MODELS_DIR`, if set.
pub fn env_models_dir() -> Option<PathBuf> {
    std::env::var_os("EPIKOS_MODELS_DIR").map(PathBuf::from)
}

/// Debug builds only: the workspace `models/` folder that `scripts/fetch-models.sh` fills.
pub fn dev_models_dir() -> Option<PathBuf> {
    cfg!(debug_assertions).then(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models"))
}

/// Model search order when the host has no folders of its own.
pub fn default_model_dirs() -> Vec<PathBuf> {
    env_models_dir().into_iter().chain(dev_models_dir()).collect()
}

impl Engine {
    /// `capacity` decoded images are kept in memory (≈ 4 bytes × sensor pixels each).
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            cache: Mutex::new(VecDeque::new()),
            masker: Masker::locate(&default_model_dirs()),
        }
    }

    /// Use `masker` (and its model directory) for AI masks.
    pub fn with_masker(mut self, masker: Masker) -> Self {
        self.masker = masker;
        self
    }

    pub fn mask_models(&self) -> MaskModels {
        MaskModels {
            dir: self.masker.dir().to_string_lossy().into_owned(),
            models: self.masker.status(),
            depth: DepthModel {
                file: self.masker.depth_file().to_string_lossy().into_owned(),
                available: self.masker.depth_available(),
            },
        }
    }

    /// Relative depth (1 = near) of the image as framed by `adjustments`' Steps 1–2,
    /// upright, at up to 1024 px. Estimated once per lens geometry and cached.
    pub fn depth(&self, path: &Path, adjustments: &Adjustments) -> Result<Arc<DepthMap>> {
        let loaded = self.load(path)?;
        self.depth_for(&loaded, adjustments)
    }

    fn depth_for(&self, loaded: &Loaded, adjustments: &Adjustments) -> Result<Arc<DepthMap>> {
        // Only the geometry changes what the model sees in a way that matters.
        let key = format!("{:?}", adjustments.lens);
        if let Some((k, d)) = loaded.depth.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            if *k == key {
                return Ok(d.clone());
            }
        }
        let display = render(loaded, &scene_only(adjustments), MASK_INPUT_SIDE, MASK_INPUT_SIDE)?;
        let rgb = rgba_to_rgb(&display);
        let image = RgbImage {
            width: rgb.width(),
            height: rgb.height(),
            data: rgb.into_raw(),
        };
        let depth = Arc::new(self.masker.depth(&image)?);
        *loaded.depth.lock().unwrap_or_else(PoisonError::into_inner) = Some((key, depth.clone()));
        Ok(depth)
    }

    /// Depth for the look when it needs one and the model is installed. Without it,
    /// fog falls back to a uniform haze, so a failure here is not fatal.
    fn look_depth(&self, loaded: &Loaded, adjustments: &Adjustments) -> Option<Arc<DepthMap>> {
        if !look_needs_depth(adjustments) || !self.masker.depth_available() {
            return None;
        }
        self.depth_for(loaded, adjustments)
            .inspect_err(|e| eprintln!("depth estimation failed: {e}"))
            .ok()
    }

    /// Run the `kind` model on the image as developed by `adjustments` (Steps 1–2
    /// precede masking) and keep the result for later steps.
    ///
    /// The mask is upright and matches the preview's framing, at up to 1024 px.
    pub fn detect_mask(
        &self,
        path: &Path,
        adjustments: &Adjustments,
        kind: MaskKind,
    ) -> Result<Arc<Mask>> {
        let loaded = self.load(path)?;
        let mask = Arc::new(self.segment(&loaded, adjustments, kind)?);
        let mut masks = loaded.masks.lock().unwrap_or_else(PoisonError::into_inner);
        masks.retain(|m| m.kind != kind);
        masks.push(mask.clone());
        Ok(mask)
    }

    fn segment(&self, loaded: &Loaded, adjustments: &Adjustments, kind: MaskKind) -> Result<Mask> {
        let display = render(loaded, &scene_only(adjustments), MASK_INPUT_SIDE, MASK_INPUT_SIDE)?;
        let rgb = rgba_to_rgb(&display);
        let image = RgbImage {
            width: rgb.width(),
            height: rgb.height(),
            data: rgb.into_raw(),
        };
        self.masker.segment(kind, &image)
    }

    /// The last mask of `kind` detected for `path`, if the image is still cached.
    pub fn mask(&self, path: &Path, kind: MaskKind) -> Option<Arc<Mask>> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let (_, loaded) = cache.iter().find(|(p, _)| p == path)?;
        let masks = loaded.masks.lock().unwrap_or_else(PoisonError::into_inner);
        masks.iter().find(|m| m.kind == kind).cloned()
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
            capture: raw.metadata.clone(),
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
        let depth = self.look_depth(&loaded, adjustments);
        render_with(&loaded, adjustments, max_w, max_h, depth.as_deref())
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

    /// Develop at full resolution and write a TIFF, layered PSD or enhanced DNG to
    /// `dest` (Step 8 handoff), with the AI masks when asked for.
    pub fn export(
        &self,
        path: &Path,
        adjustments: &Adjustments,
        dest: &Path,
        options: ExportOptions,
    ) -> Result<ExportReport> {
        let loaded = self.load(path)?;
        let depth = self.look_depth(&loaded, adjustments);
        let aux = self.export_channels(&loaded, adjustments, &options, depth.as_deref())?;
        export::export(&loaded, adjustments, dest, options, depth.as_deref(), aux)
    }

    /// Model outputs for the export's alpha channels (Step 8 handoff): subject and sky
    /// where their models are installed, skin always, depth on request.
    fn export_channels(
        &self,
        loaded: &Loaded,
        adjustments: &Adjustments,
        options: &ExportOptions,
        look_depth: Option<&DepthMap>,
    ) -> Result<Vec<export::AuxPlane>> {
        let mut aux = Vec::new();
        if options.ai_masks {
            let status = self.masker.status();
            for kind in MaskKind::ALL {
                if !status.iter().any(|s| s.kind == kind && s.available) {
                    continue;
                }
                let mask = self.segment(loaded, adjustments, kind)?;
                aux.push(export::AuxPlane {
                    name: kind.label().to_string(),
                    width: mask.width,
                    height: mask.height,
                    data: mask.alpha.iter().map(|&a| a as f32 / 255.0).collect(),
                });
            }
            let (w, h, skin) = self.skin_plane(loaded, adjustments)?;
            aux.push(export::AuxPlane { name: "Skin".into(), width: w, height: h, data: skin });
        }
        if options.depth_channel && self.masker.depth_available() {
            let depth = match look_depth {
                Some(d) => d.clone(),
                None => (*self.depth_for(loaded, adjustments)?).clone(),
            };
            aux.push(export::AuxPlane {
                name: "Depth".into(),
                width: depth.width,
                height: depth.height,
                data: depth.depth,
            });
        }
        Ok(aux)
    }

    /// Turn a written description into settings (PRD Section 4). Lights are placed on
    /// the subject, found as the centre of the detected skin.
    pub fn interpret_look(&self, path: &Path, prompt: &str, adjustments: &Adjustments) -> Result<LookPrompt> {
        let loaded = self.load(path)?;
        let base = loaded.base(256, 256);
        let rgb = develop_rgb_with((*base).clone(), &loaded.raw.profile, &scene_only(adjustments), &LookInputs::default())?;
        let skin = skin_likelihood(&rgb);
        let (mut sx, mut sy, mut sw) = (0.0f32, 0.0f32, 0.0f32);
        for (i, &s) in skin.iter().enumerate() {
            if s > 0.5 {
                sx += s * ((i as u32 % rgb.width) as f32 + 0.5) / rgb.width as f32;
                sy += s * ((i as u32 / rgb.width) as f32 + 0.5) / rgb.height as f32;
                sw += s;
            }
        }
        // Enough skin to be a subject (≥ 0.3 % of the frame), else no hint.
        let subject = (sw > 0.003 * skin.len() as f32).then(|| (sx / sw, sy / sw));
        Ok(interpret_look(prompt, adjustments, subject))
    }

    /// Skin likelihood of the scene (Steps 1–2) at mask resolution.
    fn skin_plane(&self, loaded: &Loaded, adjustments: &Adjustments) -> Result<(u32, u32, Vec<f32>)> {
        let base = loaded.base(MASK_INPUT_SIDE, MASK_INPUT_SIDE);
        let rgb = develop_rgb_with((*base).clone(), &loaded.raw.profile, &scene_only(adjustments), &LookInputs::default())?;
        Ok((rgb.width, rgb.height, skin_likelihood(&rgb)))
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
            let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
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
            masks: Mutex::new(Vec::new()),
            depth: Mutex::new(None),
        });
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.retain(|(p, _)| p != path);
        cache.push_front((path.to_path_buf(), loaded.clone()));
        cache.truncate(self.capacity);
        Ok(loaded)
    }
}

fn render(loaded: &Loaded, adjustments: &Adjustments, max_w: u32, max_h: u32) -> Result<DisplayImage> {
    render_with(loaded, adjustments, max_w, max_h, None)
}

fn render_with(
    loaded: &Loaded,
    adjustments: &Adjustments,
    max_w: u32,
    max_h: u32,
    depth: Option<&DepthMap>,
) -> Result<DisplayImage> {
    let base = loaded.base(max_w, max_h);
    let inputs = look_inputs(depth);
    let developed = develop_rgb_with((*base).clone(), &loaded.raw.profile, adjustments, &inputs)?;
    Ok(to_display_srgb(&developed))
}

pub(crate) fn look_inputs(depth: Option<&DepthMap>) -> LookInputs<'_> {
    LookInputs {
        depth: depth.map(|d| DepthPlane {
            width: d.width,
            height: d.height,
            data: &d.depth,
        }),
    }
}

/// Steps 1–2 only. Models describe the scene, not the look: a black-and-white or
/// golden style would only make sky, subject and depth harder to read.
pub(crate) fn scene_only(adjustments: &Adjustments) -> Adjustments {
    Adjustments {
        texture: Default::default(),
        color: Default::default(),
        atmosphere: Default::default(),
        style: Default::default(),
        ..adjustments.clone()
    }
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
                metadata: Default::default(),
            },
            bases: Mutex::new(Vec::new()),
            masks: Mutex::new(Vec::new()),
            depth: Mutex::new(None),
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
        assert_eq!(loaded.bases.lock().unwrap_or_else(PoisonError::into_inner).len(), 1);
    }

    #[test]
    fn noise_reduction_reaches_the_preview() {
        // Noisy flat field: colour NR must reduce pixel-to-pixel colour variation.
        let mut loaded = synthetic_loaded(128, 128);
        let mut seed = 1u32;
        for v in loaded.raw.mosaic.data.iter_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *v += 0.05 * ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5);
        }
        let spread = |d: &DisplayImage| {
            let diffs: Vec<f32> = d.rgba.chunks(4).map(|p| p[0] as f32 - p[1] as f32).collect();
            let mean = diffs.iter().sum::<f32>() / diffs.len() as f32;
            diffs.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / diffs.len() as f32
        };
        let mut off = Adjustments::default();
        off.noise_reduction.color = 0.0;
        let mut on = Adjustments::default();
        on.noise_reduction.color = 100.0;
        let (a, b) = (
            render(&loaded, &off, 64, 64).unwrap(),
            render(&loaded, &on, 64, 64).unwrap(),
        );
        assert!(spread(&b) < 0.5 * spread(&a), "{} vs {}", spread(&b), spread(&a));
    }

    #[test]
    fn missing_mask_models_fail_cleanly() {
        let dir = temp_dir("no-models");
        let engine = Engine::default().with_masker(Masker::new(&dir));
        assert!(engine.mask_models().models.iter().all(|m| !m.available));
        let raw = dir.join("x.ARW");
        assert!(engine
            .detect_mask(&raw, &Adjustments::default(), MaskKind::Sky)
            .is_err());
        fs::remove_dir_all(&dir).unwrap();
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

    fn prophoto() -> ExportOptions {
        ExportOptions {
            color_space: OutputSpace::ProPhoto,
            ..ExportOptions::default()
        }
    }

    #[test]
    fn export_writes_a_16_bit_tiff_with_icc_and_no_partial_file() {
        use tiff::decoder::Decoder;
        use tiff::tags::Tag;

        let dir = temp_dir("export");
        let dest = dir.join("out.tif");
        let loaded = synthetic_loaded(64, 48);
        let report =
            export::export(&loaded, &Adjustments::default(), &dest, prophoto(), None, Vec::new()).unwrap();
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
    fn export_writes_named_alpha_channels_that_decode_intact() {
        use tiff::decoder::Decoder;
        use tiff::tags::Tag;

        let dir = temp_dir("export-alpha");
        let dest = dir.join("masks.tif");
        let loaded = synthetic_loaded(64, 48);
        // Low-resolution planes, as the models produce: subject on the left half.
        let subject: Vec<f32> = (0..32 * 24).map(|i| if i % 32 < 16 { 1.0 } else { 0.0 }).collect();
        let aux = vec![
            export::AuxPlane { name: "Subject".into(), width: 32, height: 24, data: subject },
            export::AuxPlane { name: "Skin".into(), width: 32, height: 24, data: vec![0.25; 32 * 24] },
        ];
        let report = export::export(
            &loaded,
            &Adjustments::default(),
            &dest,
            ExportOptions::default(),
            None,
            aux,
        )
        .unwrap();
        assert_eq!(report.alpha_channels, ["Subject", "Skin"]);

        let mut dec = Decoder::new(fs::File::open(&dest).unwrap()).unwrap();
        assert_eq!(dec.get_tag_u32_vec(Tag::ExtraSamples).unwrap(), [0, 0]);
        assert_eq!(dec.get_tag_u32(Tag::SamplesPerPixel).unwrap(), 5);
        let resources = dec.get_tag_u8_vec(Tag::Unknown(34377)).unwrap();
        assert!(resources.windows(8).any(|w| w == b"\x07Subject"), "no Pascal name");
        let utf16: Vec<u8> = "Skin".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert!(resources.windows(utf16.len()).any(|w| w == utf16), "no Unicode name");

        // The `tiff` decoder drops non-alpha extra channels, so read the strips as
        // Photoshop does: inflate, then undo horizontal differencing over 5 samples.
        let offsets = dec.get_tag_u64_vec(Tag::StripOffsets).unwrap();
        let counts = dec.get_tag_u64_vec(Tag::StripByteCounts).unwrap();
        let file = fs::read(&dest).unwrap();
        let mut bytes = Vec::new();
        for (&o, &n) in offsets.iter().zip(&counts) {
            use std::io::Read;
            flate2::read::ZlibDecoder::new(&file[o as usize..(o + n) as usize])
                .read_to_end(&mut bytes)
                .unwrap();
        }
        let mut px: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
        assert_eq!(px.len(), 64 * 48 * 5);
        for row in px.as_chunks_mut::<{ 64 * 5 }>().0 {
            for i in 5..row.len() {
                row[i] = row[i].wrapping_add(row[i - 5]);
            }
        }
        let at = |x: usize, y: usize, c: usize| px[(y * 64 + x) * 5 + c];
        // Grey synthetic image: RGB neutral; masks survive the predictor and ZIP.
        assert_eq!(at(10, 20, 0), at(10, 20, 1));
        assert!(at(5, 20, 3) > 60_000, "subject inside: {}", at(5, 20, 3));
        assert!(at(58, 20, 3) < 5_000, "subject outside: {}", at(58, 20, 3));
        assert!(at(30, 30, 4).abs_diff(16_384) < 700, "skin: {}", at(30, 30, 4));
        fs::remove_dir_all(&dir).unwrap();
    }

    fn subject_plane() -> Vec<export::AuxPlane> {
        let subject: Vec<f32> = (0..32 * 24).map(|i| if i % 32 < 16 { 1.0 } else { 0.0 }).collect();
        vec![export::AuxPlane { name: "Subject".into(), width: 32, height: 24, data: subject }]
    }

    #[test]
    fn psd_export_has_the_image_layer_and_a_masked_group() {
        let dir = temp_dir("export-psd");
        let dest = dir.join("layers.psd");
        let loaded = synthetic_loaded(64, 48);
        let options = ExportOptions { format: ExportFormat::Psd, ..ExportOptions::default() };
        let report =
            export::export(&loaded, &Adjustments::default(), &dest, options, None, subject_plane()).unwrap();
        assert_eq!(report.alpha_channels, ["Subject"]);

        let d = fs::read(&dest).unwrap();
        assert_eq!(&d[..4], b"8BPS");
        assert_eq!(u16::from_be_bytes([d[22], d[23]]), 16, "16-bit");
        let find = |needle: &[u8]| d.windows(needle.len()).position(|w| w == needle);
        let lr16 = find(b"8BIMLr16").expect("16-bit layer block");
        let count = i16::from_be_bytes([d[lr16 + 12], d[lr16 + 13]]);
        assert_eq!(count, 3, "image layer + group end marker + group");
        assert!(find(b"8BIMlsct").is_some() && find(b"8BIMpass").is_some());
        let name: Vec<u8> = "Subject".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert!(find(&name).is_some(), "Unicode group name");
        // Composite: raw 16-bit RGB at the end.
        assert_eq!(u16::from_be_bytes([d[d.len() - 64 * 48 * 6 - 2], d[d.len() - 64 * 48 * 6 - 1]]), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    fn fuji_metadata() -> epikos_core::CaptureMetadata {
        use epikos_core::{CaptureMetadata, GpsInfo};
        CaptureMetadata {
            make: "FUJIFILM".into(),
            model: "X-H2".into(),
            exposure_time: Some((1, 640)),
            f_number: Some((4, 1)),
            iso: Some(5000),
            lens_model: Some("XF50-140mmF2.8 R LM OIS WR".into()),
            date_time_original: Some("2025:12:14 16:20:31".into()),
            offset_time_original: Some("+11:00".into()),
            artist: Some("Aaron Machiri".into()),
            gps: Some(GpsInfo {
                latitude_ref: Some("S".into()),
                latitude: Some([(33, 1), (52, 1), (1234, 100)]),
                longitude_ref: Some("E".into()),
                longitude: Some([(151, 1), (12, 1), (3456, 100)]),
                ..GpsInfo::default()
            }),
            ..CaptureMetadata::default()
        }
    }

    /// Read one SHORT/LONG tag from a little-endian TIFF-structured block's IFD at `ifd`.
    fn tiff_tag(block: &[u8], ifd: usize, tag: u16) -> Option<u32> {
        let u16_at = |o: usize| u16::from_le_bytes([block[o], block[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes(block[o..o + 4].try_into().unwrap());
        (0..u16_at(ifd) as usize).map(|i| ifd + 2 + 12 * i).find(|&e| u16_at(e) == tag).map(|e| {
            if u16_at(e + 2) == 3 { u16_at(e + 8) as u32 } else { u32_at(e + 8) }
        })
    }

    #[test]
    fn exif_block_is_a_valid_tiff_structure() {
        let block = export::exif_block(&fuji_metadata(), OutputSpace::Srgb, None).unwrap();
        assert_eq!(&block[..4], b"II*\0");
        let ifd0 = u32::from_le_bytes(block[4..8].try_into().unwrap()) as usize;
        assert_eq!(tiff_tag(&block, ifd0, 274), Some(1), "orientation");
        let exif = tiff_tag(&block, ifd0, 34665).expect("EXIF sub-IFD pointer") as usize;
        assert_eq!(tiff_tag(&block, exif, 34855), Some(5000), "ISO in the EXIF IFD");
        assert!(tiff_tag(&block, ifd0, 34853).is_none(), "no GPS unless allowed");
        assert!(block.windows(8).any(|w| w == b"FUJIFILM"));
    }

    #[test]
    fn psd_export_carries_exif_and_xmp_metadata() {
        let dir = temp_dir("export-psd-meta");
        let mut loaded = synthetic_loaded(32, 24);
        loaded.raw.metadata = fuji_metadata();
        for include_location in [true, false] {
            let dest = dir.join(format!("meta-{include_location}.psd"));
            let options = ExportOptions { format: ExportFormat::Psd, include_location, ..ExportOptions::default() };
            let report = export::export(&loaded, &Adjustments::default(), &dest, options, None, Vec::new()).unwrap();
            assert!(report.wrote_exif);
            assert_eq!(report.wrote_location, include_location);
            let d = fs::read(&dest).unwrap();
            let find = |needle: &[u8]| d.windows(needle.len()).any(|w| w == needle);
            // Resources 1058 (EXIF) and 1060 (XMP), each with the "8BIM" signature.
            assert!(find(&[b'8', b'B', b'I', b'M', 0x04, 0x22]), "EXIF resource");
            assert!(find(&[b'8', b'B', b'I', b'M', 0x04, 0x24]), "XMP resource");
            assert!(find(b"exif:DateTimeOriginal=\"2025-12-14T16:20:31+11:00\""));
            assert!(find(b"<rdf:li>5000</rdf:li>"), "ISO in XMP");
            assert!(find(b"aux:Lens=\"XF50-140mmF2.8 R LM OIS WR\""));
            assert!(find(b"<dc:creator><rdf:Seq><rdf:li>Aaron Machiri</rdf:li>"));
            assert_eq!(find(b"exif:GPSLatitude=\"33,52.205667S\""), include_location);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dng_export_reopens_as_linear_rgb() {
        let dir = temp_dir("export-dng");
        let dest = dir.join("enhanced.dng");
        let loaded = synthetic_loaded(64, 48);
        let options = ExportOptions { format: ExportFormat::Dng, ..ExportOptions::default() };
        let report =
            export::export(&loaded, &Adjustments::default(), &dest, options, None, subject_plane()).unwrap();
        assert_eq!(report.format, ExportFormat::Dng);
        let raw = epikos_decode::decode_file(&dest).unwrap();
        assert_eq!((raw.mosaic.width, raw.mosaic.height), (64, 48));
        assert_eq!(raw.mosaic.samples_per_pixel, 3);
        let d = fs::read(&dest).unwrap();
        assert!(d.windows(7).any(|w| w == b"Subject"), "semantic mask name");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_refuses_a_mismatched_extension() {
        let dir = temp_dir("export-ext");
        let loaded = synthetic_loaded(16, 16);
        let options = ExportOptions { format: ExportFormat::Psd, ..ExportOptions::default() };
        let err = export::export(&loaded, &Adjustments::default(), &dir.join("x.tif"), options, None, Vec::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains(".psd"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_resizes_to_the_long_edge() {
        let dir = temp_dir("export-size");
        let dest = dir.join("small.tif");
        let loaded = synthetic_loaded(64, 48);
        let options = ExportOptions { long_edge: Some(32), ..ExportOptions::default() };
        let report =
            export::export(&loaded, &Adjustments::default(), &dest, options, None, Vec::new()).unwrap();
        assert_eq!((report.width, report.height), (32, 24));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_copies_exif_and_gps_only_when_allowed() {
        use epikos_core::{CaptureMetadata, GpsInfo};
        use tiff::decoder::Decoder;
        use tiff::tags::Tag;

        let dir = temp_dir("export-exif");
        let mut loaded = synthetic_loaded(16, 16);
        loaded.raw.metadata = CaptureMetadata {
            make: "FUJIFILM".into(),
            model: "X-T5".into(),
            exposure_time: Some((1, 250)),
            f_number: Some((28, 10)),
            iso: Some(3200),
            lens_model: Some("XF33mmF1.4 R LM WR".into()),
            date_time_original: Some("2026:05:14 18:42:07".into()),
            gps: Some(GpsInfo {
                latitude_ref: Some("S".into()),
                latitude: Some([(33, 1), (52, 1), (1234, 100)]),
                longitude_ref: Some("E".into()),
                longitude: Some([(151, 1), (12, 1), (3456, 100)]),
                ..GpsInfo::default()
            }),
            ..CaptureMetadata::default()
        };

        for include_location in [true, false] {
            let dest = dir.join(format!("gps-{include_location}.tif"));
            let options = ExportOptions {
                include_location,
                ..ExportOptions::default()
            };
            let report =
                export::export(&loaded, &Adjustments::default(), &dest, options, None, Vec::new()).unwrap();
            assert_eq!(report.wrote_location, include_location);

            let mut dec = Decoder::new(fs::File::open(&dest).unwrap()).unwrap();
            assert_eq!(dec.get_tag_ascii_string(Tag::Make).unwrap(), "FUJIFILM");
            assert_eq!(dec.get_tag_u32(Tag::Orientation).unwrap(), 1);
        assert_eq!(dec.get_tag_u32(Tag::ResolutionUnit).unwrap(), 2);
        assert_eq!(dec.get_tag_u32_vec(Tag::XResolution).unwrap(), [300, 1]);
            assert_eq!(
                dec.find_tag(Tag::GpsDirectory).unwrap().is_some(),
                include_location
            );

            let ptr = dec
                .get_tag(Tag::ExifDirectory)
                .unwrap()
                .into_ifd_pointer()
                .unwrap();
            let exif = dec.read_directory(ptr).unwrap();
            let mut tags = dec.read_directory_tags(&exif);
            assert_eq!(
                tags.get_tag_ascii_string(Tag::Unknown(42036)).unwrap(),
                "XF33mmF1.4 R LM WR"
            );
            assert_eq!(
                tags.get_tag_ascii_string(Tag::Unknown(36867)).unwrap(),
                "2026:05:14 18:42:07"
            );
            assert_eq!(tags.get_tag_u32(Tag::Unknown(34855)).unwrap(), 3200);
        }
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
        assert!(
            export::export(&loaded, &adj, &dir.join("x.jpg"), ExportOptions::default(), None, Vec::new())
                .is_err()
        );
        assert!(export::export(&loaded, &adj, &raw, ExportOptions::default(), None, Vec::new()).is_err());
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
