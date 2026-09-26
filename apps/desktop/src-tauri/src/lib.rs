//! Tauri command surface for the EPIKOS RAW desktop app.
//!
//! Every command runs the engine on a blocking worker thread so decoding and
//! developing never stall the UI. Image data crosses the IPC bridge as raw bytes
//! (`tauri::ipc::Response`) rather than JSON.

use std::path::{Path, PathBuf};
use std::sync::Arc;

mod handoff;

use epikos_core::CameraFormat;
use epikos_engine::{
    dev_models_dir, env_models_dir, Engine, ExportOptions, ExportReport, FileEntry, ImageInfo, MaskModels,
    Masker, SaveReport, StyleInfo,
};
use epikos_sidecar::{Adjustments, DevelopDocument, MaskTarget, Tone};
use tauri::ipc::Response;
use tauri::{Manager, State};

type EngineState<'a> = State<'a, Arc<Engine>>;
type CmdResult<T> = Result<T, String>;

/// Upper bound on preview size requested by the UI (keeps IPC payloads bounded).
const MAX_PREVIEW_SIDE: u32 = 4096;

#[tauri::command]
async fn list_folder(engine: EngineState<'_>, dir: String) -> CmdResult<Vec<FileEntry>> {
    let engine = engine.inner().clone();
    blocking(move || engine.list_folder(Path::new(&dir))).await
}

/// A session of individual photos (Open Photo, or files dropped on the window):
/// the supported ones among `paths`, folders expanded, sorted by name.
#[tauri::command]
async fn list_files(engine: EngineState<'_>, paths: Vec<String>) -> CmdResult<Vec<FileEntry>> {
    let engine = engine.inner().clone();
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    blocking(move || engine.list_files(&paths)).await
}

/// Story-arc groups for a session of individual photos.
#[tauri::command]
async fn story_arc_files(engine: EngineState<'_>, paths: Vec<String>) -> CmdResult<epikos_engine::StoryArc> {
    let engine = engine.inner().clone();
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    blocking(move || engine.story_arc_files(&paths)).await
}

/// Extensions the Open Photo dialog offers (RAW, DNG, JPEG, PNG, TIFF).
#[tauri::command]
fn photo_extensions() -> Vec<&'static str> {
    CameraFormat::EXTENSIONS.to_vec()
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
async fn export_image(
    engine: EngineState<'_>,
    path: String,
    adjustments: Adjustments,
    dest: String,
    options: ExportOptions,
) -> CmdResult<ExportReport> {
    let path = raw_path(&path)?;
    let dest = PathBuf::from(dest);
    let engine = engine.inner().clone();
    blocking(move || engine.export(&path, &adjustments, &dest, options)).await
}

#[tauri::command]
async fn mask_models(engine: EngineState<'_>) -> CmdResult<MaskModels> {
    let engine = engine.inner().clone();
    blocking(move || Ok(engine.mask_models())).await
}

/// Step 6 depth map, same binary layout as [`detect_mask`] with one byte per pixel
/// (255 = nearest, 0 = farthest).
#[tauri::command]
async fn detect_depth(engine: EngineState<'_>, path: String, adjustments: Adjustments) -> CmdResult<Response> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    let depth = blocking(move || engine.depth(&path, &adjustments)).await?;

    let mut out = Vec::with_capacity(12 + depth.depth.len());
    out.extend_from_slice(&depth.width.to_le_bytes());
    out.extend_from_slice(&depth.height.to_le_bytes());
    out.extend_from_slice(&(depth.infer_ms.min(u32::MAX as u64) as u32).to_le_bytes());
    out.extend(depth.depth.iter().map(|d| (d.clamp(0.0, 1.0) * 255.0).round() as u8));
    Ok(Response::new(out))
}

/// Natural Language Look Prompting: a description → settings, plus what matched.
#[tauri::command]
async fn interpret_look(
    engine: EngineState<'_>,
    path: String,
    prompt: String,
    adjustments: Adjustments,
) -> CmdResult<epikos_engine::LookPrompt> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    blocking(move || engine.interpret_look(&path, &prompt, &adjustments)).await
}

/// Section 2.1: genre, light and skin reading of one photo.
#[tauri::command]
async fn analyze_image(
    engine: EngineState<'_>,
    path: String,
    adjustments: Adjustments,
) -> CmdResult<epikos_engine::SceneAnalysis> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    blocking(move || engine.analyze(&path, &adjustments)).await
}

/// Section 2.2: split a folder into story-arc groups with hero frames and palettes.
#[tauri::command]
async fn story_arc(engine: EngineState<'_>, dir: String) -> CmdResult<epikos_engine::StoryArc> {
    let engine = engine.inner().clone();
    blocking(move || engine.story_arc(Path::new(&dir))).await
}

/// Copy the hero's look onto `targets`, calibrated per frame; every sidecar is backed up.
#[tauri::command]
async fn sync_look(
    engine: EngineState<'_>,
    hero: String,
    adjustments: Adjustments,
    targets: Vec<String>,
) -> CmdResult<epikos_engine::SyncReport> {
    let hero = raw_path(&hero)?;
    let targets = targets.iter().map(|t| raw_path(t)).collect::<CmdResult<Vec<_>>>()?;
    let engine = engine.inner().clone();
    blocking(move || engine.sync_look(&hero, &adjustments, &targets)).await
}

/// Restore the sidecars a sync replaced. Returns the frames that were restored.
#[tauri::command]
async fn undo_sync(engine: EngineState<'_>, targets: Vec<String>) -> CmdResult<Vec<String>> {
    let targets = targets.iter().map(|t| raw_path(t)).collect::<CmdResult<Vec<_>>>()?;
    let engine = engine.inner().clone();
    blocking(move || engine.undo_sync(&targets)).await
}

/// The AI Mentor: how and why to edit this photo, and a recommended starting point.
#[tauri::command]
async fn mentor(
    engine: EngineState<'_>,
    path: String,
    adjustments: Adjustments,
) -> CmdResult<epikos_engine::MentorReport> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    blocking(move || engine.mentor(&path, &adjustments)).await
}

/// Live feedback on the current edit (praise and warnings).
#[tauri::command]
async fn critique(
    engine: EngineState<'_>,
    path: String,
    adjustments: Adjustments,
) -> CmdResult<Vec<epikos_engine::Feedback>> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    blocking(move || engine.critique(&path, &adjustments)).await
}

/// Imported 3D LUTs.
#[tauri::command]
async fn list_luts(engine: EngineState<'_>) -> CmdResult<Vec<epikos_engine::LutInfo>> {
    let engine = engine.inner().clone();
    blocking(move || engine.list_luts()).await
}

/// Validate a `.cube` file and copy it into the app's LUT folder.
#[tauri::command]
async fn import_lut(engine: EngineState<'_>, path: String) -> CmdResult<epikos_engine::LutInfo> {
    let path = PathBuf::from(path);
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("cube")) {
        return Err("choose a .cube 3D LUT file".into());
    }
    let engine = engine.inner().clone();
    blocking(move || engine.import_lut(&path)).await
}

/// Delete an imported LUT (the app's copy only).
#[tauri::command]
async fn remove_lut(engine: EngineState<'_>, name: String) -> CmdResult<()> {
    let engine = engine.inner().clone();
    blocking(move || engine.remove_lut(&name)).await
}

/// Photo editors installed on this computer, for "Export & open in…".
#[tauri::command]
async fn handoff_apps() -> CmdResult<Vec<handoff::HandoffApp>> {
    blocking(|| Ok(handoff::installed())).await
}

/// Open an exported TIFF in one of [`handoff_apps`].
#[tauri::command]
async fn open_in_app(app: String, file: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || handoff::open_in(&app, Path::new(&file)))
        .await
        .map_err(|e| format!("worker failed: {e}"))?
}

/// Built-in parametric styles for the preset panel.
#[tauri::command]
async fn list_styles() -> CmdResult<Vec<StyleInfo>> {
    blocking(|| Ok(epikos_engine::styles())).await
}

/// Binary layout (little-endian): `u32 width, u32 height, u32 inference ms,
/// width×height mask bytes` (0 = outside, 255 = inside). Upright, preview framing.
#[tauri::command]
async fn detect_mask(
    engine: EngineState<'_>,
    path: String,
    adjustments: Adjustments,
    kind: MaskTarget,
) -> CmdResult<Response> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    let mask = blocking(move || engine.detect_mask(&path, &adjustments, kind)).await?;
    let alpha = mask.alpha();

    let mut out = Vec::with_capacity(12 + alpha.len());
    out.extend_from_slice(&mask.width.to_le_bytes());
    out.extend_from_slice(&mask.height.to_le_bytes());
    out.extend_from_slice(&(mask.infer_ms.min(u32::MAX as u64) as u32).to_le_bytes());
    out.extend_from_slice(&alpha);
    Ok(Response::new(out))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoTone {
    exposure: f32,
    tone: Tone,
}

/// Step 2 Auto: exposure and tone suggestions for the photo as set up in Step 1.
#[tauri::command]
async fn auto_tone(engine: EngineState<'_>, path: String, adjustments: Adjustments) -> CmdResult<AutoTone> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    let (exposure, tone) = blocking(move || engine.auto_tone(&path, &adjustments)).await?;
    Ok(AutoTone { exposure, tone })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AutoUpright {
    rotation: f32,
    vertical: f32,
}

/// Step 1 auto-geometry: straighten angle and vertical perspective.
#[tauri::command]
async fn auto_upright(engine: EngineState<'_>, path: String, adjustments: Adjustments) -> CmdResult<AutoUpright> {
    let path = raw_path(&path)?;
    let engine = engine.inner().clone();
    let (rotation, vertical) = blocking(move || engine.auto_upright(&path, &adjustments)).await?;
    Ok(AutoUpright { rotation, vertical })
}

/// Only existing RAW, DNG, JPEG, PNG and TIFF files may be opened, and sidecars are only
/// written next to them.
fn raw_path(path: &str) -> CmdResult<PathBuf> {
    let p = PathBuf::from(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    if CameraFormat::from_extension(ext) == CameraFormat::Unknown {
        return Err(format!("not a supported RAW, DNG, JPEG, PNG or TIFF file: {path}"));
    }
    if !p.is_file() {
        return Err(format!("file not found: {path}"));
    }
    Ok(p)
}

/// Mask model search order: `$EPIKOS_MODELS_DIR`, the app data folder, models bundled
/// as resources, then (debug builds) the workspace `models/` folder.
fn model_dirs<R: tauri::Runtime>(paths: &tauri::path::PathResolver<R>) -> Vec<PathBuf> {
    env_models_dir()
        .into_iter()
        .chain(paths.app_data_dir().ok().map(|d| d.join("models")))
        .chain(paths.resource_dir().ok().map(|d| d.join("models")))
        .chain(dev_models_dir())
        .collect()
}

/// Run engine work off the UI thread. A panic in `f` becomes an error message for
/// the UI (and a crash-log entry, see [`crash_log`]) instead of taking the app down.
async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> epikos_core::Result<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)))
        .await
        .map_err(|e| format!("worker failed: {e}"))?
        .map_err(|payload| {
            format!(
                "Internal error: {}. The app kept running; details are in {}.",
                panic_message(&*payload),
                crash_log().display()
            )
        })?
        .map_err(|e| e.to_string())
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// `~/Library/Logs/EPIKOS RAW/crash.log` on macOS, the temp folder elsewhere.
fn crash_log() -> PathBuf {
    let dir = match std::env::var_os("HOME") {
        Some(home) if cfg!(target_os = "macos") => Path::new(&home).join("Library/Logs/EPIKOS RAW"),
        _ => std::env::temp_dir().join("epikos-raw"),
    };
    dir.join("crash.log")
}

/// Record every panic (message, location, backtrace) in [`crash_log`] before the
/// default hook prints it, so a crash in a dev or release build can be diagnosed.
fn install_panic_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let path = crash_log();
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let when = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
            let _ = writeln!(
                f,
                "--- panic at unix time {when} on thread '{thread}' ---\n{info}\n{}\n",
                std::backtrace::Backtrace::force_capture()
            );
        }
        default(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_log();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let masker = Masker::locate(&model_dirs(app.path()));
            let mut engine = Engine::default().with_masker(masker);
            // Imported LUTs live in the app's data folder (where the CLI looks too),
            // unless $EPIKOS_LUTS_DIR says otherwise.
            if std::env::var_os("EPIKOS_LUTS_DIR").is_none() {
                if let Ok(dir) = app.path().app_data_dir() {
                    engine = engine.with_lut_dir(dir.join("luts"));
                }
            }
            app.manage(Arc::new(engine));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_folder,
            list_files,
            story_arc_files,
            photo_extensions,
            open_image,
            render_preview,
            thumbnail,
            save_document,
            export_image,
            mask_models,
            detect_mask,
            detect_depth,
            list_styles,
            handoff_apps,
            open_in_app,
            interpret_look,
            analyze_image,
            story_arc,
            sync_look,
            undo_sync,
            auto_tone,
            auto_upright,
            mentor,
            critique,
            list_luts,
            import_lut,
            remove_lut
        ])
        .run(tauri::generate_context!())
        .expect("error while running EPIKOS RAW");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_command_returns_an_error_instead_of_crashing() {
        let result: CmdResult<()> = tauri::async_runtime::block_on(blocking(|| panic!("boom in the engine")));
        let err = result.unwrap_err();
        assert!(err.contains("boom in the engine") && err.contains("crash.log"), "{err}");
        // The runtime still works afterwards.
        assert_eq!(tauri::async_runtime::block_on(blocking(|| Ok(7))), Ok(7));
    }
}
