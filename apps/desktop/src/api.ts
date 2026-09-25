import { invoke, isTauri } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  Adjustments,
  DevelopDocument,
  FileEntry,
  ImageInfo,
  Preview,
  SaveReport,
} from "./types";

export const inTauri = isTauri();

export async function pickFolder(): Promise<string | null> {
  const dir = await open({ directory: true, multiple: false, title: "Open folder" });
  return typeof dir === "string" ? dir : null;
}

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
