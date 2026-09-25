//! Step 3: AI Subject & Semantic Masking, run locally on the CPU through ONNX Runtime.
//!
//! | Mask    | Model                                   | Input      |
//! |---------|-----------------------------------------|------------|
//! | Subject | IS-Net (DIS), `isnet-general-use.onnx`  | 1024×1024  |
//! | Sky     | U²-Net sky segmentation, `skyseg.onnx`  | 320×320    |
//!
//! Models are downloaded by `scripts/fetch-models.sh` (they are not committed) and loaded
//! lazily on first use, then kept for the life of the [`Masker`]. Inputs are
//! display-referred sRGB images; masks come back at the input's size, upright, as
//! 8-bit coverage (255 = fully inside the mask).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use epikos_core::{Error, Result};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use rayon::prelude::*;
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
    sessions: [Mutex<Option<Session>>; 2],
}

impl Masker {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            sessions: [Mutex::new(None), Mutex::new(None)],
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
        let path = self.dir.join(kind.model_file());
        if !path.is_file() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "{} model not found at {} (run scripts/fetch-models.sh)",
                    kind.label(),
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
                .commit_from_file(&path)
                .map_err(ml)
        };
        // One retry: a model in a cloud-synced folder can transiently read short.
        commit().or_else(|_| commit())
    }
}

fn ml(e: impl std::fmt::Display) -> Error {
    Error::Decode(format!("mask model: {e}"))
}

/// Resize to the model's square input and normalise into planar NCHW floats.
fn to_nchw(image: &RgbImage, spec: &ModelSpec) -> Vec<f32> {
    let side = spec.side;
    let n = (side * side) as usize;
    let mut out = vec![0.0f32; 3 * n];
    for c in 0..3 {
        let plane: Vec<f32> = image.data[c..]
            .iter()
            .step_by(3)
            .map(|&v| v as f32 / 255.0)
            .collect();
        let resized = resize_plane(&plane, image.width, image.height, side, side);
        for (o, v) in out[c * n..(c + 1) * n].iter_mut().zip(resized) {
            *o = (v - spec.mean[c]) / spec.std[c];
        }
    }
    out
}

/// Separable resize: box-averages when shrinking (no aliasing), bilinear when enlarging.
fn resize_plane(src: &[f32], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<f32> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    // Horizontal pass: sh rows of dw.
    let mut tmp = vec![0.0f32; dw * sh];
    tmp.par_chunks_mut(dw).enumerate().for_each(|(y, row)| {
        let s = &src[y * sw..(y + 1) * sw];
        for (x, o) in row.iter_mut().enumerate() {
            *o = sample_1d(|i| s[i], sw, dw, x);
        }
    });
    // Vertical pass.
    let mut out = vec![0.0f32; dw * dh];
    out.par_chunks_mut(dw).enumerate().for_each(|(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            *o = sample_1d(|i| tmp[i * dw + x], sh, dh, y);
        }
    });
    out
}

/// Output sample `i` of a length-`n` signal resampled to length `m`.
fn sample_1d(at: impl Fn(usize) -> f32, n: usize, m: usize, i: usize) -> f32 {
    let scale = n as f32 / m as f32;
    if scale > 1.0 {
        // Area average over [i·scale, (i+1)·scale).
        let (a, b) = (i as f32 * scale, (i + 1) as f32 * scale);
        let (first, last) = (a.floor() as usize, (b.ceil() as usize).min(n));
        let (mut sum, mut weight) = (0.0, 0.0);
        for j in first..last {
            let w = (b.min(j as f32 + 1.0) - a.max(j as f32)).max(0.0);
            sum += w * at(j);
            weight += w;
        }
        sum / weight.max(1e-6)
    } else {
        // Pixel-centre aligned bilinear.
        let x = ((i as f32 + 0.5) * scale - 0.5).clamp(0.0, (n - 1) as f32);
        let x0 = x.floor() as usize;
        let x1 = (x0 + 1).min(n - 1);
        let t = x - x0 as f32;
        at(x0) * (1.0 - t) + at(x1) * t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_preserves_flat_fields_and_means() {
        let src = vec![0.25f32; 37 * 23];
        for (w, h) in [(10, 7), (80, 51), (37, 23)] {
            let out = resize_plane(&src, 37, 23, w, h);
            assert_eq!(out.len(), (w * h) as usize);
            assert!(out.iter().all(|v| (v - 0.25).abs() < 1e-5));
        }
        // Shrinking a checkerboard averages it instead of aliasing.
        let checker: Vec<f32> = (0..64 * 64).map(|i| ((i % 64 + i / 64) % 2) as f32).collect();
        let out = resize_plane(&checker, 64, 64, 16, 16);
        assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-5));
    }

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
