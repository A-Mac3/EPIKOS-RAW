// Dev-only stand-in for the Rust engine so the UI can be exercised in a plain browser
// (`npm run dev`, then open http://localhost:5173/?mock). It renders a synthetic test
// chart and applies exposure / white balance approximately. Never shipped: main.tsx only
// imports this behind `import.meta.env.DEV`.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Adjustments, DevelopDocument, FileEntry, ImageInfo } from "../types";
import { defaultAdjustments } from "../types";

const FOLDER = "/mock/Sydney shoot";
const FILES: FileEntry[] = ["DSCF0346.RAF", "DSCF0352.RAF", "L1000530.DNG", "L1000583.DNG", "IMG_0421.CR3"].map(
  (name) => ({
    path: `${FOLDER}/${name}`,
    name,
    format: name.endsWith("RAF") ? "Fujifilm RAF" : name.endsWith("DNG") ? "Leica DNG" : "Canon CR3",
    hasEdits: false,
  }),
);
const saved = new Map<string, DevelopDocument>();
const AS_SHOT = { temperature: 5600, tint: 4 };

export function installMockBackend() {
  (globalThis as { isTauri?: boolean }).isTauri = true;
  mockWindows("main");
  mockIPC(async (cmd, args) => {
    const a = args as Record<string, unknown>;
    await new Promise((r) => setTimeout(r, 15)); // pretend IPC latency
    switch (cmd) {
      case "plugin:dialog|open":
        return FOLDER;
      case "plugin:dialog|save":
        return (a.options as { defaultPath?: string } | undefined)?.defaultPath ?? `${FOLDER}/export.tif`;
      case "export_tiff":
        await new Promise((r) => setTimeout(r, 900));
        console.info("[mock] export", a);
        return {
          path: a.dest,
          width: 7728,
          height: 5152,
          colorSpace: a.colorSpace,
          bytes: 231_900_000,
          developMs: 1180,
          writeMs: 850,
        };
      case "list_folder":
        return FILES.map((f) => ({ ...f, hasEdits: f.hasEdits || saved.has(f.path) }));
      case "open_image":
        return info(a.path as string);
      case "render_preview":
        return render(a.adjustments as Adjustments, a.maxWidth as number, a.maxHeight as number, a.path as string);
      case "thumbnail":
        return jpegThumb(a.path as string);
      case "save_document":
        saved.set(a.path as string, a.document as DevelopDocument);
        console.info("[mock] saved", a.path, a.document);
        return { jsonPath: `${a.path}.epikos.json`, xmpPath: null, warning: null };
      default:
        throw new Error(`mock backend: unhandled command ${cmd}`);
    }
  });
}

function info(path: string): ImageInfo {
  const f = FILES.find((x) => x.path === path)!;
  return {
    path,
    name: f.name,
    format: f.format,
    make: f.format.split(" ")[0],
    model: f.format.startsWith("Fuji") ? "X-T5" : f.format.startsWith("Leica") ? "SL2" : "EOS R5",
    width: 7728,
    height: 5152,
    monochrome: false,
    asShot: AS_SHOT,
    document: saved.get(path) ?? {
      version: 1,
      source: { path, sha256: "", format: f.format, make: "", model: "" },
      adjustments: defaultAdjustments(),
    },
    loadedFrom: saved.has(path) ? `${path}.epikos.json` : null,
  };
}

function seed(path: string) {
  let h = 0;
  for (const c of path) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return (h % 360) / 360;
}

/** Linear-light synthetic scene: sky gradient, horizon, colour patches, grey ramp. */
function scene(u: number, v: number, hue: number): [number, number, number] {
  if (v > 0.82) {
    const g = 0.02 + 0.2 * u; // grey ramp
    return [g, g, g];
  }
  if (v > 0.55 && v < 0.75 && u > 0.08 && u < 0.92) {
    const i = Math.floor(((u - 0.08) / 0.84) * 8);
    const h = (hue + i / 8) % 1;
    const [r, g, b] = hsv(h, 0.65, 0.45);
    return [r, g, b];
  }
  if (v < 0.55) {
    const t = v / 0.55;
    return [0.12 + 0.35 * t, 0.2 + 0.3 * t, 0.45 + 0.1 * t]; // sky
  }
  return [0.05, 0.06, 0.04];
}

function hsv(h: number, s: number, v: number): [number, number, number] {
  const f = (n: number) => {
    const k = (n + h * 6) % 6;
    return v - v * s * Math.max(0, Math.min(k, 4 - k, 1));
  };
  return [f(5), f(3), f(1)];
}

function render(adj: Adjustments, maxW: number, maxH: number, path: string): ArrayBuffer {
  const aspect = 3 / 2;
  let w = Math.min(maxW, 1600);
  let h = Math.round(w / aspect);
  if (h > maxH) {
    h = Math.min(maxH, 1067);
    w = Math.round(h * aspect);
  }
  const gain = Math.pow(2, adj.exposure);
  const wb = adj.whiteBalance.mode === "custom" ? adj.whiteBalance : { ...AS_SHOT };
  const t = (wb.temperature - AS_SHOT.temperature) / 4000;
  const tint = (wb.tint - AS_SHOT.tint) / 300;
  const mul = [1 + 0.35 * t + tint, 1 - tint, 1 - 0.35 * t + tint];
  const hue = seed(path);

  const hist = new Uint32Array(3 * 256);
  const out = new ArrayBuffer(8 + hist.byteLength + w * h * 4);
  const view = new DataView(out);
  view.setUint32(0, w, true);
  view.setUint32(4, h, true);
  const px = new Uint8ClampedArray(out, 8 + hist.byteLength);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const lin = scene(x / w, y / h, hue);
      const i = (y * w + x) * 4;
      for (let c = 0; c < 3; c++) {
        const v = Math.max(0, lin[c] * gain * mul[c]);
        const shoulder = v <= 0.8 ? v : 0.8 + 0.2 * (1 - Math.exp(-(v - 0.8) / 0.2));
        const e = shoulder <= 0.0031308 ? 12.92 * shoulder : 1.055 * Math.pow(shoulder, 1 / 2.4) - 0.055;
        const b = Math.round(Math.min(1, e) * 255);
        px[i + c] = b;
        hist[c * 256 + b]++;
      }
      px[i + 3] = 255;
    }
  }
  new Uint32Array(out, 8, 3 * 256).set(hist);
  return out;
}

async function jpegThumb(path: string): Promise<ArrayBuffer> {
  const buf = render(defaultAdjustments(), 240, 160, path);
  const w = new DataView(buf).getUint32(0, true);
  const h = new DataView(buf).getUint32(4, true);
  const canvas = new OffscreenCanvas(w, h);
  canvas.getContext("2d")!.putImageData(new ImageData(new Uint8ClampedArray(buf, 8 + 3 * 256 * 4), w, h), 0, 0);
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality: 0.8 });
  return blob.arrayBuffer();
}
