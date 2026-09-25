//! Tauri command surface for the EPIKOS RAW desktop app.
//!
//! Every command runs the engine on a blocking worker thread so decoding and
//! developing never stall the UI. Image data crosses the IPC bridge as raw bytes
//! (`tauri::ipc::Response`) rather than JSON.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use epikos_core::CameraFormat;
use epikos_engine::{Engine, ExportReport, FileEntry, ImageInfo, OutputSpace, SaveReport};
use epikos_sidecar::{Adjustments, DevelopDocument};
use tauri::ipc::Response;
use tauri::State;

type EngineState<'a> = State<'a, Arc<Engine>>;
type CmdResult<T> = Result<T, String>;

/// Upper bound on preview size requested by the UI (keeps IPC payloads bounded).
const MAX_PREVIEW_SIDE: u32 = 4096;

#[tauri::command]
async fn list_folder(engine: EngineState<'_>, dir: String) -> CmdResult<Vec<FileEntry>> {
    let engine = engine.inner().clone();
    blocking(move || engine.list_folder(Path::new(&dir))).await
}

#[tauri::command]
async fn open_image(engine: EngineState<'_>, path: String) -> CmdResult<ImageInfo> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    blocking(move || engine.open(&path)).await
}

/// Binary layout (little-endian): `u32 width, u32 height, 3×256 u32 histogram (R, G, B),
/// width×height×4 RGBA bytes`.
#[tauri::command]
async fn render_preview(
    engine: EngineState<'_>,
    path: String,
    adjustments: Adjustments,
    max_width: u32,
    max_height: u32,
) -> CmdResult<Response> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    let (w, h) = (
        max_width.clamp(1, MAX_PREVIEW_SIDE),
        max_height.clamp(1, MAX_PREVIEW_SIDE),
    );
    let image = blocking(move || engine.render_preview(&path, &adjustments, w, h)).await?;

    let mut out = Vec::with_capacity(8 + 3 * 256 * 4 + image.rgba.len());
    out.extend_from_slice(&image.width.to_le_bytes());
    out.extend_from_slice(&image.height.to_le_bytes());
    for channel in &image.histogram {
        for bin in channel {
            out.extend_from_slice(&bin.to_le_bytes());
        }
    }
    out.extend_from_slice(&image.rgba);
    Ok(Response::new(out))
}

/// JPEG bytes.
#[tauri::command]
async fn thumbnail(engine: EngineState<'_>, path: String, max_side: u32) -> CmdResult<Response> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    let jpeg = blocking(move || engine.thumbnail_jpeg(&path, max_side.clamp(32, 1024))).await?;
    Ok(Response::new(jpeg))
}

#[tauri::command]
async fn save_document(
    engine: EngineState<'_>,
    path: String,
    document: DevelopDocument,
) -> CmdResult<SaveReport> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    blocking(move || engine.save(&path, &document)).await
}

/// Full-resolution 16-bit TIFF. `dest` comes from the native save dialog.
#[tauri::command]
async fn export_tiff(
    engine: EngineState<'_>,
    path: String,
    adjustments: Adjustments,
    dest: String,
    color_space: OutputSpace,
) -> CmdResult<ExportReport> {
    let path = raw_path(&path)?;
    let dest = PathBuf::from(dest);
    let engine = engine.inner().clone();
    blocking(move || engine.export_tiff(&path, &adjustments, &dest, color_space)).await
}

/// Only existing RAW/DNG files may be opened, and sidecars are only written next to them.
fn raw_path(path: &str) -> CmdResult<PathBuf> {
    let p = PathBuf::from(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    if CameraFormat::from_extension(ext) == CameraFormat::Unknown {
        return Err(format!("not a supported RAW/DNG file: {path}"));
    }
    if !p.is_file() {
        return Err(format!("file not found: {path}"));
    }
    Ok(p)
}

async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> epikos_core::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("worker failed: {e}"))?
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(Engine::default()))
        .invoke_handler(tauri::generate_handler![
            list_folder,
            open_image,
            render_preview,
            thumbnail,
            save_document,
            export_tiff
        ])
        .run(tauri::generate_context!())
        .expect("error while running EPIKOS RAW");
}
