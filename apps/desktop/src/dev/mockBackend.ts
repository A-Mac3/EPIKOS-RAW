// Dev-only stand-in for the Rust engine so the UI can be exercised in a plain browser
// (`npm run dev`, then open http://localhost:5173/?mock). It renders a synthetic test
// chart and applies exposure / white balance approximately. Never shipped: main.tsx only
// imports this behind `import.meta.env.DEV`.
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type {
  Adjustments,
  DevelopDocument,
  FileEntry,
  ImageInfo,
  MaskTarget,
  SceneAnalysis,
  StoryArc,
  StyleInfo,
  SyncReport,
} from "../types";
import { defaultAdjustments } from "../types";

const FOLDER = "/mock/Sydney shoot";
const FILES: FileEntry[] = [
  "DSCF0346.RAF",
  "DSCF0352.RAF",
  "L1000530.DNG",
  "L1000583.DNG",
  "IMG_0421.CR3",
  "IMG_0433.JPG",
].map((name) => ({
  path: `${FOLDER}/${name}`,
  name,
  format: name.endsWith("RAF")
    ? "Fujifilm RAF"
    : name.endsWith("DNG")
      ? "Leica DNG"
      : name.endsWith("JPG")
        ? "JPEG"
        : "Canon CR3",
  hasEdits: false,
}));
const saved = new Map<string, DevelopDocument>();
const presync = new Map<string, DevelopDocument | null>();
const AS_SHOT = { temperature: 5600, tint: 4 };

// Copy of the engine's built-in styles (crates/epikos-pipeline/src/look/style.rs).
const STYLES: StyleInfo[] = [
  {
    id: "dark-melanin-glow",
    name: "Dark Melanin Glow",
    world: "High-Fashion & Melanin Precision",
    description: "Rich chocolate and bronze undertones with a clean, luminous glow.",
    skinProtection: 70,
    swatch: ["#3b2116", "#c98a4b"],
  },
  {
    id: "silver-charcoal",
    name: "Silver & Charcoal",
    world: "Character & High-Contrast Portraiture",
    description: "Black and white with deep blacks and crisp midtone texture.",
    skinProtection: 60,
    swatch: ["#111214", "#d9dadc"],
  },
  {
    id: "volumetric-golden-hour",
    name: "Volumetric Golden Hour",
    world: "Atmospheric & Environmental Landscapes",
    description: "Warm low-sun light with soft bloom and a hazy, lifted atmosphere.",
    skinProtection: 75,
    swatch: ["#5a3a1c", "#f2b45a"],
  },
  {
    id: "teal-orange",
    name: "Teal & Orange",
    world: "Cinematic & Film Emulation",
    description: "Blockbuster grade: teal shadows against warm highlights.",
    skinProtection: 80,
    swatch: ["#0f4c55", "#e08a3c"],
  },
  {
    id: "moody-earthy",
    name: "Moody & Earthy",
    world: "Atmospheric & Environmental Landscapes",
    description: "Muted foliage, slate blues, earth-tone shadows.",
    skinProtection: 70,
    swatch: ["#2d2a22", "#6b7a5e"],
  },
  {
    id: "high-key-editorial",
    name: "High-Key Editorial",
    world: "High-Fashion & Melanin Precision",
    description: "Luminous skin, pastel shadows, gentle contrast.",
    skinProtection: 60,
    swatch: ["#d9d2e6", "#f7efe9"],
  },
];

export function installMockBackend() {
  (globalThis as { isTauri?: boolean }).isTauri = true;
  mockWindows("main");
  mockIPC(async (cmd, args) => {
    const a = args as Record<string, unknown>;
    await new Promise((r) => setTimeout(r, 15)); // pretend IPC latency
    switch (cmd) {
      case "plugin:dialog|open":
        // Open Photo asks for a file, Open Folder for a directory.
        return (a.options as { directory?: boolean } | undefined)?.directory === false
          ? `${FOLDER}/L1000583.DNG`
          : FOLDER;
      case "photo_extensions":
        return ["arw", "srf", "sr2", "cr3", "cr2", "crw", "nef", "nrw", "raf", "dng", "jpg", "jpeg", "png", "tif", "tiff"];
      case "list_files":
        return filesFor(a.paths as string[]);
      case "story_arc_files":
        return storyFor(filesFor(a.paths as string[]).map((f) => f.path));
      case "plugin:dialog|save":
        return (a.options as { defaultPath?: string } | undefined)?.defaultPath ?? `${FOLDER}/export.tif`;
      case "export_image":
        if ((a.options as { format: string }).format === "dng" && String(a.path).endsWith(".JPG")) {
          throw new Error("a JPEG or PNG has no sensor data for an enhanced DNG; export a TIFF or PSD");
        }
        await new Promise((r) => setTimeout(r, 900));
        console.info("[mock] export", a);
        return {
          path: a.dest,
          format: (a.options as { format: string }).format,
          width: 7728,
          height: 5152,
          colorSpace: (a.options as { colorSpace: string }).colorSpace,
          bytes: 231_900_000,
          wroteExif: true,
          wroteLocation: (a.options as { includeLocation: boolean }).includeLocation,
          alphaChannels: (a.options as { aiMasks: boolean }).aiMasks ? ["Subject", "Sky", "Skin"] : [],
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
      case "handoff_apps":
        return [
          { name: "Adobe Photoshop 2026", path: "/Applications/Adobe Photoshop 2026/Adobe Photoshop 2026.app" },
          { name: "Adobe Lightroom Classic", path: "/Applications/Adobe Lightroom Classic/Adobe Lightroom Classic.app" },
        ];
      case "open_in_app":
        console.info("[mock] open", a.file, "in", a.app);
        return null;
      case "interpret_look": {
        // Stand-in for the engine's interpreter: recognises a few words only.
        const adj = structuredClone(a.adjustments as Adjustments);
        const words = String(a.prompt).toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
        const matched: { phrase: string; effect: string; strength: number }[] = [];
        if (words.includes("foggy")) {
          adj.atmosphere.fog += 45;
          matched.push({ phrase: "foggy", effect: "distance fog", strength: 1 });
        }
        if (words.includes("film")) {
          adj.finishing.grain += 30;
          matched.push({ phrase: "film", effect: "film: grain, soft blacks", strength: 1 });
        }
        const known = new Set(["foggy", "film", "a", "an", "the", "with", "make", "this", "look", "like"]);
        return { adjustments: adj, matched, unknown: words.filter((w) => !known.has(w)) };
      }
      case "analyze_image":
        await new Promise((r) => setTimeout(r, 400));
        return analysis(a.path as string);
      case "story_arc":
        return story();
      case "sync_look": {
        const targets = a.targets as string[];
        const report: SyncReport = { frames: [], skipped: [] };
        targets.forEach((path, i) => {
          if (saved.has(path)) presync.set(path, saved.get(path)!);
          else presync.set(path, null);
          saved.set(path, { version: 1, adjustments: structuredClone(a.adjustments as Adjustments) } as DevelopDocument);
          report.frames.push({
            path,
            exposureDelta: [0.35, -0.6, 0.1][i % 3],
            whiteBalanceShifted: i % 2 === 0,
            textureScale: [1, 1.22, 0.84][i % 3],
            skinProtection: 70,
          });
        });
        return report;
      }
      case "undo_sync": {
        const restored: string[] = [];
        for (const path of a.targets as string[]) {
          if (!presync.has(path)) continue;
          const before = presync.get(path);
          if (before) saved.set(path, before);
          else saved.delete(path);
          presync.delete(path);
          restored.push(path);
        }
        return restored;
      }
      case "list_styles":
        return STYLES;
      case "mask_models":
        return {
          dir: "/mock/models",
          models: [
            { kind: "subject", file: "/mock/models/isnet-general-use.onnx", available: true },
            { kind: "sky", file: "/mock/models/skyseg.onnx", available: true },
          ],
          depth: { file: "/mock/models/depth-anything-v2-small.onnx", available: true },
          face: { file: "/mock/models/face-parsing-resnet18.onnx", available: true },
          targets: (["subject", "background", "sky", "skin", "eyes", "hair", "foreground"] as MaskTarget[]).map(
            (target) => ({ target, available: true, source: "mock" }),
          ),
          lensDatabase: 1569,
        };
      case "detect_mask":
        await new Promise((r) => setTimeout(r, 300));
        return mask(a.kind as MaskTarget);
      case "auto_tone":
        await new Promise((r) => setTimeout(r, 200));
        return {
          exposure: 1.24,
          tone: { contrast: 0, highlights: -66, shadows: 0, whites: 0, blacks: -13, vibrance: 10, saturation: 0 },
        };
      case "auto_upright":
        await new Promise((r) => setTimeout(r, 200));
        return { rotation: 0.69, vertical: 19 };
      case "detect_depth":
        await new Promise((r) => setTimeout(r, 300));
        return depthMap();
      case "save_document":
        saved.set(a.path as string, a.document as DevelopDocument);
        console.info("[mock] saved", a.path, a.document);
        return { jsonPath: `${a.path}.epikos.json`, xmpPath: null, warning: null };
      default:
        throw new Error(`mock backend: unhandled command ${cmd}`);
    }
  }, { shouldMockEvents: true });
  // Dev only: simulate files dropped on the window, e.g.
  // __epikosDrop(["/mock/Sydney shoot/L1000583.DNG", "/tmp/notes.txt"]).
  (globalThis as { __epikosDrop?: (paths: string[]) => Promise<void> }).__epikosDrop = async (paths) => {
    await emit("tauri://drag-enter", { paths, position: { x: 400, y: 300 } });
    await new Promise((r) => setTimeout(r, 400));
    await emit("tauri://drag-drop", { paths, position: { x: 400, y: 300 } });
  };
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
    capture: {
      make: f.format.split(" ")[0].toUpperCase(),
      model: "X-T5",
      dateTimeOriginal: "2026:06:21 04:13:51",
      exposureTime: [1, 45],
      fNumber: [4, 1],
      focalLength: [302, 10],
      exposureBias: [0, 1],
      iso: 8000,
      lensMake: "FUJIFILM",
      lensModel: "XF16-55mmF2.8 R LM WR",
      gps: f.name.startsWith("L") ? { latitude: [[1, 1], [2, 1], [3, 1]], longitude: [[4, 1], [5, 1], [6, 1]] } : null,
    },
    document: saved.get(path) ?? {
      version: 1,
      source: { path, sha256: "", format: f.format, make: "", model: "" },
      adjustments: defaultAdjustments(),
    },
    loadedFrom: saved.has(path) ? `${path}.epikos.json` : null,
    lensProfile: f.name.endsWith("JPG")
      ? null
      : f.format.startsWith("Leica")
        ? "Camera (DNG)"
        : f.format.startsWith("Fuji")
          ? "Lensfun: Fujifilm XF16-55mmF2.8 R LM WR"
          : null,
    bitmap: f.name.endsWith("JPG"),
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
  // Rough stand-ins for the styles: enough to see the UI react, not the real looks.
  const k = adj.style?.id ? (adj.style.amount ?? 100) / 100 : 0;
  const mono = adj.style?.id === "silver-charcoal" ? k : 0;
  const warm =
    adj.style?.id === "volumetric-golden-hour" ? [1 + 0.25 * k, 1 + 0.05 * k, 1 - 0.3 * k]
    : adj.style?.id === "dark-melanin-glow" ? [1 + 0.08 * k, 1, 1 - 0.1 * k]
    : [1, 1, 1];

  const hist = new Uint32Array(3 * 256);
  const out = new ArrayBuffer(8 + hist.byteLength + w * h * 4);
  const view = new DataView(out);
  view.setUint32(0, w, true);
  view.setUint32(4, h, true);
  const px = new Uint8ClampedArray(out, 8 + hist.byteLength);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const raw = scene(x / w, y / h, hue);
      const grey = 0.2627 * raw[0] + 0.678 * raw[1] + 0.0593 * raw[2];
      const lin = raw.map((v, c) => (v + (grey - v) * mono) * warm[c]);
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

/** Sky = the synthetic sky gradient; subject = the colour-patch row. */
function mask(kind: MaskTarget): ArrayBuffer {
  const w = 1024;
  const h = Math.round(w / 1.5);
  const out = new ArrayBuffer(12 + w * h);
  const view = new DataView(out);
  view.setUint32(0, w, true);
  view.setUint32(4, h, true);
  view.setUint32(8, kind === "sky" ? 380 : 900, true);
  const alpha = new Uint8Array(out, 12);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const [u, v] = [x / w, y / h];
      const subject = v > 0.55 && v < 0.75 && u > 0.08 && u < 0.92;
      const inside =
        kind === "sky"
          ? v < 0.55
          : kind === "background"
            ? !subject
            : kind === "foreground"
              ? v > 0.7
              : kind === "eyes"
                ? v > 0.6 && v < 0.62 && ((u > 0.45 && u < 0.47) || (u > 0.52 && u < 0.54))
                : kind === "hair"
                  ? v > 0.55 && v < 0.58 && u > 0.44 && u < 0.56
                  : kind === "skin"
                    ? v > 0.58 && v < 0.68 && u > 0.43 && u < 0.57
                    : subject;
      alpha[y * w + x] = inside ? 255 : 0;
    }
  }
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

/** Sky far (0), ground getting nearer towards the bottom, patches in between. */
function depthMap(): ArrayBuffer {
  const w = 1024;
  const h = Math.round(w / 1.5);
  const out = new ArrayBuffer(12 + w * h);
  const view = new DataView(out);
  view.setUint32(0, w, true);
  view.setUint32(4, h, true);
  view.setUint32(8, 150, true);
  const d = new Uint8Array(out, 12);
  for (let y = 0; y < h; y++) {
    const v = y / h;
    const near = v < 0.55 ? 0 : v > 0.55 && v < 0.75 ? 0.6 : Math.min(1, (v - 0.55) / 0.45);
    d.fill(Math.round(near * 255), y * w, (y + 1) * w);
  }
  return out;
}

// Shapes and values from `epikos analyze` / `epikos story` on the real test photos.
function analysis(path: string): SceneAnalysis {
  const portrait = /L1000583|IMG_0421/.test(path);
  const bird = /DSCF/.test(path);
  return {
    genres: portrait
      ? [
          { id: "environmental-portrait", label: "Environmental Portrait", score: 0.95, evidence: "skin 7.2% in a wider scene, depth range 0.94" },
          { id: "dark-melanin-fashion", label: "Dark Melanin Fashion", score: 0.95, evidence: "portrait with skin tone depth ~8/10" },
          { id: "close-up-portrait", label: "Close-up Portrait", score: 0.93, evidence: "skin 7.2%, subject 29.3%" },
        ]
      : bird
        ? [{ id: "wildlife", label: "Wildlife", score: 0.46, evidence: "no people, 140 mm, isolated subject 1.1%, 15.4% foliage / earth colours" }]
        : [{ id: "environmental-portrait", label: "Environmental Portrait", score: 0.72, evidence: "skin 0.7% in a wider scene, depth range 0.95" }],
    lighting: {
      colorTemperature: bird ? 4239 : 6156,
      dynamicRangeEv: 7.5,
      highlightsClipped: 0.4,
      shadowsCrushed: 0.1,
      key: "mid-key",
      hardness: portrait ? 0.89 : 0.58,
      hardnessLabel: portrait ? "hard, direct" : "medium",
      direction: "from the upper right",
      backlit: portrait,
      haze: 0.08,
      hazeLabel: "clear",
      snow: 0,
      timeOfDay: "daytime",
    },
    skin: bird
      ? null
      : {
          coverage: portrait ? 7.2 : 0.7,
          toneDepth: portrait ? 8 : 8.4,
          toneLabel: "deep",
          undertone: "neutral",
          shine: 0.68,
          shineLabel: portrait ? "glossy" : "too small to judge",
          texture: 0.2,
          textureLabel: portrait ? "smooth" : "too small to judge",
        },
    composition: { subjectCoverage: portrait ? 29.3 : bird ? 1.1 : 4.7, skyCoverage: bird ? 1.6 : 12, depthRange: 0.9, lineStrength: 0.1 },
    palette: [
      { hex: "#67675d", weight: 0.24 },
      { hex: "#373b39", weight: 0.21 },
      { hex: "#8e7a6a", weight: 0.2 },
      { hex: "#1f1a18", weight: 0.19 },
      { hex: "#c9c3b8", weight: 0.16 },
    ],
    limits: [],
    analysisMs: 3100,
  };
}

/** The mock's photos among `paths` (a folder path selects all of them). */
function filesFor(paths: string[]): FileEntry[] {
  if (paths.includes(FOLDER)) return FILES.map((f) => ({ ...f, hasEdits: f.hasEdits || saved.has(f.path) }));
  return FILES.filter((f) => paths.includes(f.path)).map((f) => ({ ...f, hasEdits: f.hasEdits || saved.has(f.path) }));
}

/** The folder's story arc, restricted to `paths`. */
function storyFor(paths: string[]): StoryArc {
  const all = story();
  const groups = all.groups
    .map((g) => ({ ...g, frames: g.frames.filter((p) => paths.includes(p)) }))
    .filter((g) => g.frames.length > 0)
    .map((g, id) => ({
      ...g,
      id,
      label: g.label.replace(/\d+ photos?$/, `${g.frames.length} photo${g.frames.length === 1 ? "" : "s"}`),
      hero: g.frames.includes(g.hero) ? g.hero : g.frames[0],
    }));
  return { ...all, groups, frames: all.frames.filter((f) => paths.includes(f.path)) };
}

function story(): StoryArc {
  const groups = [
    { names: ["DSCF0346.RAF", "DSCF0352.RAF"], label: "2025-12-14 15:41–15:52 · 2 photos", reason: "", palette: ["#5a5555", "#97818c", "#2e2f30", "#5a874d", "#beceb0"] },
    { names: ["L1000530.DNG"], label: "2026-08-17 08:18 · 1 photo", reason: "245 days without shooting", palette: ["#4f5157", "#95a6b6", "#648db0", "#2e3032", "#cac9c6"] },
    { names: ["L1000583.DNG", "IMG_0421.CR3"], label: "2026-09-20 12:04–12:09 · 2 photos", reason: "34 days without shooting", palette: ["#67675d", "#373b39", "#8e7a6a", "#1f1a18", "#c9c3b8"] },
  ];
  return {
    groups: groups.map((g, id) => ({
      id,
      label: g.label,
      frames: g.names.map((n) => `${FOLDER}/${n}`),
      hero: `${FOLDER}/${g.names[0]}`,
      palette: g.palette.map((hex, i) => ({ hex, weight: [0.29, 0.2, 0.19, 0.17, 0.15][i] })),
      splitReason: g.reason,
    })),
    frames: FILES.map((f) => ({
      path: f.path,
      name: f.name,
      captured: null,
      sceneEv: 10,
      lightness: 0.5,
      cast: [0, 0],
      gps: null,
      skin: 0,
      group: groups.findIndex((g) => g.names.includes(f.name)),
      error: null,
    })),
    analysisMs: 113,
  };
}
