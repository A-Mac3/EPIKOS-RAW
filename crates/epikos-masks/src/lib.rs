//! Step 3: AI Subject & Semantic Masking, run locally on the CPU through ONNX Runtime.
//!
//! | Mask    | Model                                   | Input      |
//! |---------|-----------------------------------------|------------|
//! | Subject | IS-Net (DIS), `isnet-general-use.onnx`  | 1024×1024  |
//! | Sky     | U²-Net sky segmentation, `skyseg.onnx`  | 320×320    |
//! | Depth   | Depth Anything V2 Small, `depth-anything-v2-small.onnx` | 518 long side, ×14 |
//!
//! Models are downloaded by `scripts/fetch-models.sh` (they are not committed) and loaded
//! lazily on first use, then kept for the life of the [`Masker`]. Inputs are
//! display-referred sRGB images; masks come back at the input's size, upright, as
//! 8-bit coverage (255 = fully inside the mask).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use epikos_core::{resize_plane, Error, Result};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskKind {
    Subject,
    Sky,
}

impl MaskKind {
    pub const ALL: [MaskKind; 2] = [MaskKind::Subject, MaskKind::Sky];

    pub fn label(self) -> &'static str {
        match self {
            MaskKind::Subject => "Subject",
            MaskKind::Sky => "Sky",
        }
    }

    /// Model file name inside the models directory.
    pub fn model_file(self) -> &'static str {
        self.spec().file
    }

    fn spec(self) -> &'static ModelSpec {
        match self {
            MaskKind::Subject => &ISNET,
            MaskKind::Sky => &SKYSEG,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// How a model wants its input prepared and its output read.
struct ModelSpec {
    file: &'static str,
    side: u32,
    mean: [f32; 3],
    std: [f32; 3],
}

/// rembg's preprocessing for IS-Net: pixels / 255 − 0.5.
const ISNET: ModelSpec = ModelSpec {
    file: "isnet-general-use.onnx",
    side: 1024,
    mean: [0.5, 0.5, 0.5],
    std: [1.0, 1.0, 1.0],
};

/// ImageNet normalisation, as in the reference skyseg pipeline.
const SKYSEG: ModelSpec = ModelSpec {
    file: "skyseg.onnx",
    side: 320,
    mean: [0.485, 0.456, 0.406],
    std: [0.229, 0.224, 0.225],
};

/// Depth Anything V2 Small (Apache-2.0), ImageNet normalisation. `side` is the long
/// side; the input keeps the image's aspect ratio in multiples of the 14-px patch.
const DEPTH: ModelSpec = ModelSpec {
    file: "depth-anything-v2-small.onnx",
    side: 518,
    mean: [0.485, 0.456, 0.406],
    std: [0.229, 0.224, 0.225],
};

/// Session slot of the depth model, after the two masks.
const DEPTH_SLOT: usize = 2;

/// Relative scene depth, row-major: 1 = nearest, 0 = farthest (the sky).
#[derive(Debug, Clone)]
pub struct DepthMap {
    pub width: u32,
    pub height: u32,
    pub depth: Vec<f32>,
    /// Inference time, excluding the one-off model load.
    pub infer_ms: u64,
}

/// 8-bit RGB image, row-major, 3 bytes per pixel.
#[derive(Debug, Clone)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Soft mask, row-major, one byte per pixel (0 = outside, 255 = inside).
#[derive(Debug, Clone)]
pub struct Mask {
    pub kind: MaskKind,
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
    /// Inference time, excluding the one-off model load.
    pub infer_ms: u64,
}

impl Mask {
    /// Fraction of the frame covered (mean alpha, 0–1).
    pub fn coverage(&self) -> f32 {
        if self.alpha.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.alpha.iter().map(|&a| a as u64).sum();
        sum as f32 / (255.0 * self.alpha.len() as f32)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub kind: MaskKind,
    pub file: String,
    pub available: bool,
}

/// Owns the ONNX sessions. Thread-safe; each model runs one inference at a time.
pub struct Masker {
    dir: PathBuf,
    sessions: [Mutex<Option<Session>>; 3],
}

impl Masker {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            sessions: [Mutex::new(None), Mutex::new(None), Mutex::new(None)],
        }
    }

    /// The first candidate directory holding any model, else the first candidate (where
    /// models should be put), else `./models`.
    pub fn locate(candidates: &[PathBuf]) -> Self {
        let has_models = |d: &Path| MaskKind::ALL.iter().any(|k| d.join(k.model_file()).is_file());
        let dir = candidates
            .iter()
            .find(|d| has_models(d))
            .or(candidates.first())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("models"));
        Self::new(dir)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn status(&self) -> Vec<ModelStatus> {
        MaskKind::ALL
            .iter()
            .map(|&kind| ModelStatus {
                kind,
                file: self.dir.join(kind.model_file()).to_string_lossy().into_owned(),
                available: self.dir.join(kind.model_file()).is_file(),
            })
            .collect()
    }

    pub fn depth_file(&self) -> PathBuf {
        self.dir.join(DEPTH.file)
    }

    pub fn depth_available(&self) -> bool {
        self.depth_file().is_file()
    }

    /// Estimate relative depth for `image`, returned at the image's size.
    pub fn depth(&self, image: &RgbImage) -> Result<DepthMap> {
        if image.width == 0 || image.height == 0 {
            return Err(Error::InvalidImage {
                reason: "cannot estimate depth of an empty image".into(),
            });
        }
        let (w, h) = patch_size(image.width, image.height, DEPTH.side);
        let input = to_nchw_sized(image, &DEPTH, w, h);

        let mut slot = self.sessions[DEPTH_SLOT].lock().unwrap();
        if slot.is_none() {
            *slot = Some(self.load_file(&self.depth_file(), "Depth")?);
        }
        let session = slot.as_mut().expect("session loaded above");
        let t = Instant::now();
        let tensor =
            Tensor::from_array(([1usize, 3, h as usize, w as usize], input)).map_err(ml)?;
        let outputs = session.run(ort::inputs![tensor]).map_err(ml)?;
        // `predicted_depth`, [1, h, w]: relative inverse depth (larger = nearer).
        let (shape, disparity) = outputs[0].try_extract_tensor::<f32>().map_err(ml)?;
        let n = (w * h) as usize;
        if shape.iter().rev().take(2).product::<i64>() as usize != n || disparity.len() < n {
            return Err(Error::Decode(format!(
                "{}: unexpected output shape {shape:?}",
                DEPTH.file
            )));
        }
        let disparity = disparity[..n].to_vec();
        drop(outputs);
        let infer_ms = t.elapsed().as_millis() as u64;
        drop(slot);

        let depth = normalise_depth(&disparity);
        Ok(DepthMap {
            width: image.width,
            height: image.height,
            depth: resize_plane(&depth, w, h, image.width, image.height),
            infer_ms,
        })
    }

    /// Segment `image` and return a mask of the same size.
    pub fn segment(&self, kind: MaskKind, image: &RgbImage) -> Result<Mask> {
        let spec = kind.spec();
        if image.width == 0 || image.height == 0 {
            return Err(Error::InvalidImage {
                reason: "cannot mask an empty image".into(),
            });
        }
        let input = to_nchw(image, spec);

        let mut slot = self.sessions[kind.index()].lock().unwrap();
        if slot.is_none() {
            *slot = Some(self.load(kind)?);
        }
        let session = slot.as_mut().expect("session loaded above");

        let t = Instant::now();
        let side = spec.side as usize;
        let tensor = Tensor::from_array(([1usize, 3, side, side], input)).map_err(ml)?;
        let outputs = session.run(ort::inputs![tensor]).map_err(ml)?;
        // Both models put the finest (full-resolution) prediction first, already through
        // a sigmoid. Unlike rembg we don't min-max stretch it: that would turn a
        // near-zero "no subject" / "no sky" map into a full-strength mask.
        let (shape, prob) = outputs[0].try_extract_tensor::<f32>().map_err(ml)?;
        let n = shape.iter().rev().take(2).product::<i64>() as usize;
        if n != side * side || prob.len() < n {
            return Err(Error::Decode(format!(
                "{}: unexpected output shape {shape:?}",
                spec.file
            )));
        }
        let prob = prob[..n].to_vec();
        drop(outputs);
        let infer_ms = t.elapsed().as_millis() as u64;
        drop(slot);

        let full = resize_plane(&prob, spec.side, spec.side, image.width, image.height);
        Ok(Mask {
            kind,
            width: image.width,
            height: image.height,
            alpha: full
                .iter()
                .map(|&p| (p.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect(),
            infer_ms,
        })
    }

    fn load(&self, kind: MaskKind) -> Result<Session> {
        self.load_file(&self.dir.join(kind.model_file()), kind.label())
    }

    fn load_file(&self, path: &Path, label: &str) -> Result<Session> {
        if !path.is_file() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "{label} model not found at {} (run scripts/fetch-models.sh)",
                    path.display()
                ),
            )));
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let commit = || {
            Session::builder()
                .map_err(ml)?
                .with_optimization_level(GraphOptimizationLevel::Level3)
                .map_err(ml)?
                .with_intra_threads(threads)
                .map_err(ml)?
                .commit_from_file(path)
                .map_err(ml)
        };
        // One retry: a model in a cloud-synced folder can transiently read short.
        commit().or_else(|_| commit())
    }
}

/// Aspect-preserving input size with the long side ≈ `long`, both sides multiples of 14
/// (the ViT patch).
fn patch_size(width: u32, height: u32, long: u32) -> (u32, u32) {
    let scale = long as f32 / width.max(height) as f32;
    let snap = |v: u32| (((v as f32 * scale) / 14.0).round().max(1.0) as u32) * 14;
    (snap(width), snap(height))
}

/// Disparity → 0–1 depth (1 = near) using robust percentiles, so a few extreme pixels
/// don't flatten the range.
fn normalise_depth(disparity: &[f32]) -> Vec<f32> {
    let mut sorted: Vec<f32> = disparity.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return vec![0.0; disparity.len()];
    }
    sorted.sort_unstable_by(f32::total_cmp);
    let pct = |p: f32| sorted[((sorted.len() - 1) as f32 * p) as usize];
    let (lo, hi) = (pct(0.02), pct(0.98));
    let range = (hi - lo).max(1e-6);
    disparity
        .iter()
        .map(|&d| if d.is_finite() { ((d - lo) / range).clamp(0.0, 1.0) } else { 0.0 })
        .collect()
}

fn ml(e: impl std::fmt::Display) -> Error {
    Error::Decode(format!("mask model: {e}"))
}

/// Resize to the model's square input and normalise into planar NCHW floats.
fn to_nchw(image: &RgbImage, spec: &ModelSpec) -> Vec<f32> {
    to_nchw_sized(image, spec, spec.side, spec.side)
}

fn to_nchw_sized(image: &RgbImage, spec: &ModelSpec, w: u32, h: u32) -> Vec<f32> {
    let n = (w * h) as usize;
    let mut out = vec![0.0f32; 3 * n];
    for c in 0..3 {
        let plane: Vec<f32> = image.data[c..]
            .iter()
            .step_by(3)
            .map(|&v| v as f32 / 255.0)
            .collect();
        let resized = resize_plane(&plane, image.width, image.height, w, h);
        for (o, v) in out[c * n..(c + 1) * n].iter_mut().zip(resized) {
            *o = (v - spec.mean[c]) / spec.std[c];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nchw_layout_and_normalisation() {
        let image = RgbImage {
            width: 2,
            height: 2,
            data: [255, 0, 51].repeat(4),
        };
        let x = to_nchw(&image, &SKYSEG);
        let n = 320 * 320;
        assert_eq!(x.len(), 3 * n);
        let expect = |c: usize, v: f32| (v - SKYSEG.mean[c]) / SKYSEG.std[c];
        assert!((x[0] - expect(0, 1.0)).abs() < 1e-5);
        assert!((x[n + 17] - expect(1, 0.0)).abs() < 1e-5);
        assert!((x[2 * n + n - 1] - expect(2, 0.2)).abs() < 1e-5);
    }

    #[test]
    fn missing_models_are_reported_not_panicked() {
        let masker = Masker::new(std::env::temp_dir().join("epikos-no-models-here"));
        assert!(masker.status().iter().all(|s| !s.available));
        let img = RgbImage {
            width: 4,
            height: 4,
            data: vec![128; 48],
        };
        let err = masker.segment(MaskKind::Sky, &img).unwrap_err().to_string();
        assert!(err.contains("fetch-models"), "{err}");
    }

    #[test]
    fn locate_prefers_a_directory_with_models() {
        let empty = std::env::temp_dir().join(format!("epikos-models-empty-{}", std::process::id()));
        let full = std::env::temp_dir().join(format!("epikos-models-full-{}", std::process::id()));
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join(MaskKind::Sky.model_file()), b"").unwrap();
        assert_eq!(Masker::locate(&[empty.clone(), full.clone()]).dir(), full);
        assert_eq!(Masker::locate(std::slice::from_ref(&empty)).dir(), empty);
        std::fs::remove_dir_all(&full).unwrap();
    }

    #[test]
    fn depth_input_keeps_aspect_in_patch_multiples() {
        assert_eq!(patch_size(6000, 4000, 518), (518, 350));
        assert_eq!(patch_size(4000, 6000, 518), (350, 518));
        let (w, h) = patch_size(1000, 10, 518);
        assert!(w % 14 == 0 && h == 14);
    }

    #[test]
    fn depth_normalisation_is_robust_and_bounded() {
        let mut d: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        d[0] = -1e9; // outliers
        d[999] = 1e9;
        d[500] = f32::NAN;
        let n = normalise_depth(&d);
        assert!(n.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!((n[250] - 0.24).abs() < 0.02, "{}", n[250]);
        assert_eq!(n[500], 0.0);
    }

    /// Runs the real depth model when it's been fetched (skipped otherwise).
    #[test]
    fn real_depth_model_puts_the_ground_nearer_than_the_sky() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
        let masker = Masker::new(&dir);
        if !masker.depth_available() {
            eprintln!("skipping: depth model not fetched");
            return;
        }
        // Sky over a ground plane whose texture gets finer towards the horizon.
        let (w, h) = (384u32, 256u32);
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                if y < h / 2 {
                    data.extend([120, 170, 230]);
                } else {
                    let d = (y - h / 2 + 4) as f32;
                    let stripe = ((x as f32 - w as f32 / 2.0) / d * 8.0).floor() as i32 % 2 == 0;
                    let v = if stripe { 90 } else { 60 };
                    data.extend([v, v + 30, v / 2]);
                }
            }
        }
        let img = RgbImage { width: w, height: h, data };
        let depth = masker.depth(&img).unwrap();
        assert_eq!((depth.width, depth.height), (w, h));
        let at = |x: u32, y: u32| depth.depth[(y * w + x) as usize];
        eprintln!("sky {} horizon {} foreground {} ({} ms)", at(192, 20), at(192, 140), at(192, 250), depth.infer_ms);
        assert!(at(192, 250) > at(192, 20) + 0.3, "foreground not nearer than sky");
    }

    /// Runs the real models when they've been fetched (skipped otherwise).
    #[test]
    fn real_models_run_and_find_sky_at_the_top() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
        let masker = Masker::new(&dir);
        if !masker.status().iter().all(|s| s.available) {
            eprintln!("skipping: models not fetched");
            return;
        }
        // Blue sky gradient over dark green ground.
        let (w, h) = (256u32, 192u32);
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for _ in 0..w {
                if y < h / 2 {
                    let t = y as f32 / (h / 2) as f32;
                    data.extend([(90.0 + 80.0 * t) as u8, (150.0 + 60.0 * t) as u8, 235]);
                } else {
                    data.extend([40, 70, 30]);
                }
            }
        }
        let img = RgbImage { width: w, height: h, data };
        let sky = masker.segment(MaskKind::Sky, &img).unwrap();
        assert_eq!((sky.width, sky.height), (w, h));
        let mean = |rows: std::ops::Range<u32>| {
            let v: Vec<u32> = rows
                .flat_map(|y| (0..w).map(move |x| (y * w + x) as usize))
                .map(|i| sky.alpha[i] as u32)
                .collect();
            v.iter().sum::<u32>() as f32 / v.len() as f32
        };
        let (top, bottom) = (mean(0..h / 4), mean(3 * h / 4..h));
        assert!(top > 128.0 && bottom < 64.0, "sky top {top}, bottom {bottom}");

        let subject = masker.segment(MaskKind::Subject, &img).unwrap();
        assert_eq!(subject.alpha.len(), (w * h) as usize);
    }
}
