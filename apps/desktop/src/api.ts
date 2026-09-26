import { invoke, isTauri } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  Adjustments,
  DevelopDocument,
  ExportFormat,
  ExportOptions,
  ExportReport,
  FileEntry,
  HandoffApp,
  ImageInfo,
  LookPrompt,
  AutoTone,
  AutoUpright,
  Feedback,
  LutInfo,
  Mask,
  MaskModels,
  MentorReport,
  MaskTarget,
  Preview,
  SaveReport,
  SceneAnalysis,
  StoryArc,
  StyleInfo,
  SyncReport,
} from "./types";

export const inTauri = isTauri();

export async function pickFolder(): Promise<string | null> {
  const dir = await open({ directory: true, multiple: false, title: "Open folder" });
  return typeof dir === "string" ? dir : null;
}

/** Native file picker for one photo (RAW, DNG, JPEG, PNG, TIFF); `null` if cancelled. */
export async function pickPhoto(): Promise<string | null> {
  const exts = await invoke<string[]>("photo_extensions");
  const file = await open({
    directory: false,
    multiple: false,
    title: "Open photo",
    // Both cases: some pickers match extensions case-sensitively (IMG_0001.CR3).
    filters: [{ name: "Photos", extensions: [...exts, ...exts.map((e) => e.toUpperCase())] }],
  });
  return typeof file === "string" ? file : null;
}

/** Files dragged over the window: `over` while they hover (with the paths on entry),
 * `drop` with the paths, `leave` when the drag ends elsewhere. */
export type FileDrag = { type: "over"; paths?: string[] } | { type: "drop"; paths: string[] } | { type: "leave" };

/** Tauri 2's native drag-and-drop of files onto the window. */
export function onFileDrag(handler: (e: FileDrag) => void): Promise<UnlistenFn> {
  return getCurrentWebview().onDragDropEvent(({ payload: p }) => {
    if (p.type === "enter") handler({ type: "over", paths: p.paths });
    else if (p.type === "over") handler({ type: "over" });
    else if (p.type === "drop") handler({ type: "drop", paths: p.paths });
    else handler({ type: "leave" });
  });
}

const SAVE_FILTER: Record<ExportFormat, { title: string; name: string; extensions: string[] }> = {
  tiff: { title: "Export 16-bit TIFF", name: "TIFF image", extensions: ["tif", "tiff"] },
  psd: { title: "Export layered PSD", name: "Photoshop document", extensions: ["psd"] },
  dng: { title: "Export enhanced DNG", name: "DNG raw", extensions: ["dng"] },
  jpeg: { title: "Export JPEG", name: "JPEG image", extensions: ["jpg", "jpeg"] },
  png: { title: "Export 16-bit PNG", name: "PNG image", extensions: ["png"] },
};

/** Native save dialog for an export; `null` if cancelled. */
export async function pickExportPath(defaultPath: string, format: ExportFormat): Promise<string | null> {
  const f = SAVE_FILTER[format];
  const dest = await save({ title: f.title, defaultPath, filters: [{ name: f.name, extensions: f.extensions }] });
  return dest ?? null;
}

export const exportImage = (
  path: string,
  adjustments: Adjustments,
  dest: string,
  options: ExportOptions,
) => invoke<ExportReport>("export_image", { path, adjustments, dest, options });

export const listFolder = (dir: string) => invoke<FileEntry[]>("list_folder", { dir });

/** A session of individual photos: the supported ones among `paths` (folders expanded). */
export const listFiles = (paths: string[]) => invoke<FileEntry[]>("list_files", { paths });

/** Story-arc groups for a session of individual photos. */
export const storyArcFiles = (paths: string[]) => invoke<StoryArc>("story_arc_files", { paths });

export const openImage = (path: string) => invoke<ImageInfo>("open_image", { path });

export const saveDocument = (path: string, document: DevelopDocument) =>
  invoke<SaveReport>("save_document", { path, document });

/** Layout: u32 width, u32 height, 3×256 u32 histogram, then RGBA bytes (little-endian). */
export async function renderPreview(
  path: string,
  adjustments: Adjustments,
  maxWidth: number,
  maxHeight: number,
): Promise<Preview> {
  const buf = await invoke<ArrayBuffer>("render_preview", {
    path,
    adjustments,
    maxWidth: Math.round(maxWidth),
    maxHeight: Math.round(maxHeight),
  });
  const view = new DataView(buf);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  const bins = 256;
  const histogram = [0, 1, 2].map((ch) => {
    const out = new Uint32Array(bins);
    for (let i = 0; i < bins; i++) out[i] = view.getUint32(8 + (ch * bins + i) * 4, true);
    return out;
  }) as Preview["histogram"];
  const offset = 8 + 3 * bins * 4;
  const rgba = new Uint8ClampedArray(buf, offset, width * height * 4);
  return { width, height, rgba, histogram };
}

/** JPEG thumbnail as an object URL (caller owns revocation). */
export async function thumbnailUrl(path: string, maxSide: number): Promise<string> {
  const buf = await invoke<ArrayBuffer>("thumbnail", { path, maxSide });
  return URL.createObjectURL(new Blob([buf], { type: "image/jpeg" }));
}

export const maskModels = () => invoke<MaskModels>("mask_models");

/** Layout: u32 width, u32 height, u32 inference ms, then one mask byte per pixel. */
export async function detectMask(path: string, adjustments: Adjustments, kind: MaskTarget): Promise<Mask> {
  return parseMask(await invoke<ArrayBuffer>("detect_mask", { path, adjustments, kind }), kind);
}

/** Step 6 depth map in the mask layout (255 = nearest). */
export async function detectDepth(path: string, adjustments: Adjustments): Promise<Mask> {
  return parseMask(await invoke<ArrayBuffer>("detect_depth", { path, adjustments }), "depth");
}

function parseMask(buf: ArrayBuffer, kind: Mask["kind"]): Mask {
  const view = new DataView(buf);
  const width = view.getUint32(0, true);
  const height = view.getUint32(4, true);
  const inferMs = view.getUint32(8, true);
  const alpha = new Uint8Array(buf, 12, width * height);
  let sum = 0;
  for (let i = 0; i < alpha.length; i++) sum += alpha[i];
  return { kind, width, height, alpha, coverage: alpha.length ? sum / (255 * alpha.length) : 0, inferMs };
}

export const listStyles = () => invoke<StyleInfo[]>("list_styles");

/** Photo editors installed on this computer (macOS). */
export const handoffApps = () => invoke<HandoffApp[]>("handoff_apps");

export const openInApp = (app: string, file: string) => invoke<void>("open_in_app", { app, file });

/** Natural Language Look Prompting: description → settings (engine-side, offline). */
export const interpretLook = (path: string, prompt: string, adjustments: Adjustments) =>
  invoke<LookPrompt>("interpret_look", { path, prompt, adjustments });

/** Section 2.1: genre, light and skin reading of one photo. */
export const analyzeImage = (path: string, adjustments: Adjustments) =>
  invoke<SceneAnalysis>("analyze_image", { path, adjustments });

/** Section 2.2: story-arc groups, hero frames and palettes for a folder. */
export const storyArc = (dir: string) => invoke<StoryArc>("story_arc", { dir });

/** Copy the hero's look onto `targets`, calibrated per frame (sidecars are backed up). */
export const syncLook = (hero: string, adjustments: Adjustments, targets: string[]) =>
  invoke<SyncReport>("sync_look", { hero, adjustments, targets });

/** Restore the sidecars the last sync replaced; returns the restored paths. */
export const undoSync = (targets: string[]) => invoke<string[]>("undo_sync", { targets });

/** Step 2 Auto: exposure and tone suggestions. */
export const autoTone = (path: string, adjustments: Adjustments) =>
  invoke<AutoTone>("auto_tone", { path, adjustments });

/** Step 1 auto-geometry: straighten angle and vertical perspective. */
export const autoUpright = (path: string, adjustments: Adjustments) =>
  invoke<AutoUpright>("auto_upright", { path, adjustments });

/** AI Mentor: insights and a recommended starting point for the photo. */
export const mentor = (path: string, adjustments: Adjustments) =>
  invoke<MentorReport>("mentor", { path, adjustments });

/** Live feedback on the current edit. */
export const critique = (path: string, adjustments: Adjustments) =>
  invoke<Feedback[]>("critique", { path, adjustments });

export const listLuts = () => invoke<LutInfo[]>("list_luts");

/** Pick a `.cube` file and import it; `null` if cancelled. */
export async function importLut(): Promise<LutInfo | null> {
  const file = await open({
    directory: false,
    multiple: false,
    title: "Import 3D LUT",
    filters: [{ name: "3D LUT", extensions: ["cube", "CUBE"] }],
  });
  if (typeof file !== "string") return null;
  return invoke<LutInfo>("import_lut", { path: file });
}

export const removeLut = (name: string) => invoke<void>("remove_lut", { name });
