import { invoke, isTauri } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  Adjustments,
  DevelopDocument,
  ExportOptions,
  ExportReport,
  FileEntry,
  HandoffApp,
  ImageInfo,
  Mask,
  MaskKind,
  MaskModels,
  Preview,
  SaveReport,
  StyleInfo,
} from "./types";

export const inTauri = isTauri();

export async function pickFolder(): Promise<string | null> {
  const dir = await open({ directory: true, multiple: false, title: "Open folder" });
  return typeof dir === "string" ? dir : null;
}

/** Native save dialog for a TIFF export; `null` if cancelled. */
export async function pickExportPath(defaultPath: string): Promise<string | null> {
  const dest = await save({
    title: "Export 16-bit TIFF",
    defaultPath,
    filters: [{ name: "TIFF image", extensions: ["tif", "tiff"] }],
  });
  return dest ?? null;
}

export const exportTiff = (
  path: string,
  adjustments: Adjustments,
  dest: string,
  options: ExportOptions,
) => invoke<ExportReport>("export_tiff", { path, adjustments, dest, options });

export const listFolder = (dir: string) => invoke<FileEntry[]>("list_folder", { dir });

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
export async function detectMask(path: string, adjustments: Adjustments, kind: MaskKind): Promise<Mask> {
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
