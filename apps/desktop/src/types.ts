// Mirrors of the Rust serde types (camelCase). Keep in sync with
// crates/epikos-sidecar/src/document.rs and crates/epikos-engine/src/lib.rs.

export type WbMode = "asShot" | "custom";
export type DemosaicMode = "auto" | "malvar" | "bilinear" | "xtrans";

export interface WhiteBalance {
  mode: WbMode;
  temperature: number;
  tint: number;
}

export interface DistortionCoeffs {
  enabled: boolean;
  k1: number;
  k2: number;
  k3: number;
  p1: number;
  p2: number;
  cx: number;
  cy: number;
}

export interface ChromaticAberration {
  enabled: boolean;
  red: number;
  blue: number;
}

/** 0–100. Colour NR is on by default for RAW files, luminance off. */
export interface NoiseReduction {
  luminance: number;
  color: number;
}

/** Step 4. Clarity and micro-texture −100…100; skin retouching 0…100. */
export interface Texture {
  clarity: number;
  microTexture: number;
  blemishSmoothing: number;
  specularBalance: number;
  /** −100 (soften) … 100 (deepen) the lines of a face. */
  characterLines: number;
  /** Confine skin retouching and line sculpting to the subject mask. */
  retouchSubjectOnly: boolean;
}

/** Step 2 tone controls, each −100…100. */
export interface Tone {
  contrast: number;
  highlights: number;
  shadows: number;
  whites: number;
  blacks: number;
  vibrance: number;
  saturation: number;
  /** −100…100: removes (+) or adds (−) haze. */
  dehaze: number;
}

export const TONE_KEYS = ["contrast", "highlights", "shadows", "whites", "blacks", "vibrance", "saturation"] as const;

/** A Step 3 region. */
export type MaskTarget = "subject" | "background" | "sky" | "skin" | "eyes" | "hair" | "foreground";

export const MASK_TARGETS: MaskTarget[] = ["subject", "background", "sky", "skin", "eyes", "hair", "foreground"];

/** Step 3 edit inside a mask: exposure in EV (−3…3), the rest −100…100. */
export interface LocalAdjustment {
  mask: MaskTarget;
  exposure: number;
  contrast: number;
  saturation: number;
  warmth: number;
  clarity: number;
  /** Greener (−) or more magenta (+). */
  tint: number;
}

/** Aspect presets of the crop tool. */
export type CropAspect = "free" | "original" | "1:1" | "4:5" | "16:9" | "9:16";

/** A crop of the upright frame, as fractions of its width and height. */
export interface Crop {
  x: number;
  y: number;
  width: number;
  height: number;
  aspect: CropAspect;
}

export const defaultCrop = (): Crop => ({ x: 0, y: 0, width: 1, height: 1, aspect: "free" });

export const cropIsActive = (c: Crop) => c.x > 1e-4 || c.y > 1e-4 || c.width < 1 - 1e-4 || c.height < 1 - 1e-4;

/** Hue in degrees, amount 0…100, saturation and luminance −100…100. */
export interface BackgroundTint {
  hue: number;
  amount: number;
  saturation: number;
  luminance: number;
}

/** −100…100 each. */
export interface HslChannel {
  hue: number;
  saturation: number;
  luminance: number;
}

export const HSL_BANDS = ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"] as const;
export type HslBand = (typeof HSL_BANDS)[number];
export type HslBands = Record<HslBand, HslChannel>;

/** Hue in degrees on the HSV wheel, amount 0…100, luminance −100…100. */
export interface ColorWheel {
  hue: number;
  amount: number;
  luminance: number;
}

export const WHEEL_RANGES = ["shadows", "midtones", "highlights"] as const;
export type WheelRange = (typeof WHEEL_RANGES)[number];

/** Step 5. */
export interface ColorGrade {
  hsl: HslBands;
  wheels: Record<WheelRange, ColorWheel>;
  /** 0…100. */
  skinProtection: number;
  /** Greenery outside the subject: hue − teal … + autumn gold. */
  foliage: HslChannel;
  /** Everything outside the subject. */
  background: BackgroundTint;
}

/** Step 6. Strengths 0…100, warmth −100 (cool) … 100 (gold). */
export interface Atmosphere {
  glow: number;
  glowSize: number;
  glowWarmth: number;
  /** Only the subject glows (subject mask). */
  glowSubjectOnly: boolean;
  fog: number;
  /** Depth where fog begins, 0 (camera) … 100 (far background). */
  fogStart: number;
  fogWarmth: number;
  shafts: number;
  shaftLength: number;
  shaftWarmth: number;
  shaftAuto: boolean;
  /** Light position, 0–1 across and down the upright frame. */
  shaftX: number;
  shaftY: number;
  /** 3D Light Sculptor: virtual lights placed in the scene's depth. */
  lights: VirtualLight[];
}

/** A light placed in 3D: x, y 0–1 on the frame; depth 0 (camera) … 1 (far). */
export interface VirtualLight {
  x: number;
  y: number;
  depth: number;
  intensity: number;
  reach: number;
  /** −100 cool … 100 warm. */
  warmth: number;
  halo: number;
}

/** What a click on the image does in Step 6: place the shafts' light, or a 3D light. */
export type PickTarget = "shafts" | "light" | null;

export function defaultLight(x = 0.5, y = 0.4): VirtualLight {
  return { x, y, depth: 0.5, intensity: 50, reach: 50, warmth: 40, halo: 40 };
}

/**
 * Step 7 parametric curve, every value −100…100 (all zero = identity). `black` > 0
 * lifts the floor (matte), < 0 crushes; `white` > 0 clips earlier, < 0 fades.
 */
export interface ToneCurve {
  shadows: number;
  darks: number;
  lights: number;
  highlights: number;
  black: number;
  white: number;
  /** Free-form point curve after the parametric one: [input, output] in 0…1. */
  points: [number, number][];
}

export const CURVE_CHANNELS = ["rgb", "red", "green", "blue"] as const;
export type CurveChannel = (typeof CURVE_CHANNELS)[number];
export type Curves = Record<CurveChannel, ToneCurve> & {
  /** Standard contrast S-curve under the master curve. */
  sCurve: SCurve;
};

/** Amount 0…100; pivot 0…100 is the tone it turns around (50 = middle). */
export interface SCurve {
  enabled: boolean;
  amount: number;
  pivot: number;
}

/** An imported `.cube` LUT by name, blended at `amount` 0…100; empty name = none. */
export interface LutRef {
  name: string;
  amount: number;
}

export interface LutInfo {
  name: string;
  title: string | null;
  size: number;
}

/** AI Mentor: what it sees, why it matters and what to do. */
export interface Insight {
  topic: string;
  observation: string;
  why: string;
  how: string;
}

export interface MentorReport {
  summary: string;
  insights: Insight[];
  recommended: Adjustments;
  changes: string[];
  /** A composition crop and straighten, offered apart from the starting point. */
  crop: { crop: Crop; rotation: number; reason: string } | null;
  /** The look the starting point aims for ("Editorial" or a learned style's name). */
  target: string;
  analysisMs: number;
}

export interface Feedback {
  level: "praise" | "warning" | "tip";
  text: string;
  /** The correction (warnings and tips): slider targets in words and the settings. */
  fix: { label: string; adjustments: Adjustments } | null;
}

/** Hue in degrees on the HSV wheel, saturation 0…100, balance −100…100. */
export interface SplitToning {
  highlightHue: number;
  highlightSaturation: number;
  shadowHue: number;
  shadowSaturation: number;
  balance: number;
}

/** Step 8. Grain 0…100; vignette −100 (darken) … 100 (lighten). */
export interface Finishing {
  grain: number;
  grainSize: number;
  grainRoughness: number;
  vignette: number;
  vignetteMidpoint: number;
  vignetteFeather: number;
  /** −100 follows the frame … 100 circle. */
  vignetteRoundness: number;
}

/** Parametric style layered on the manual Step 4–6 settings. `id` empty = none. */
export interface StyleRef {
  id: string;
  amount: number;
  skinProtection: number;
  /** Style Fusion Matrix: up to four styles by weight; replaces `id` when set. */
  blend: StyleWeight[];
}

export interface StyleWeight {
  id: string;
  weight: number;
}

/** Natural Language Look Prompting result. */
export interface LookPrompt {
  adjustments: Adjustments;
  matched: { phrase: string; effect: string; strength: number }[];
  unknown: string[];
}

export const STYLE_CATEGORIES = ["Film Simulations", "Portraits & Skin", "Landscape & Nature", "Cinematic"] as const;

export interface StyleInfo {
  id: string;
  name: string;
  world: string;
  /** Library tab (one of STYLE_CATEGORIES; "Legacy" for unlisted styles). */
  category: string;
  /** Offered in the library (earlier styles still resolve but aren't listed). */
  listed: boolean;
  description: string;
  skinProtection: number;
  swatch: [string, string];
}

export interface Adjustments {
  whiteBalance: WhiteBalance;
  highlightRecovery: boolean;
  demosaic: DemosaicMode;
  lens: {
    /** Apply the camera's (DNG) or Lensfun lens profile. */
    profile: boolean;
    distortion: DistortionCoeffs;
    chromaticAberration: ChromaticAberration;
    /** Straighten, degrees (+ = counter-clockwise). */
    rotation: number;
    /** Vertical perspective −100…100. */
    vertical: number;
  };
  exposure: number;
  tone: Tone;
  noiseReduction: NoiseReduction;
  local: LocalAdjustment[];
  texture: Texture;
  color: ColorGrade;
  atmosphere: Atmosphere;
  curves: Curves;
  splitToning: SplitToning;
  finishing: Finishing;
  style: StyleRef;
  lut: LutRef;
  /** Step 1 crop of the upright (straightened) frame. */
  crop: Crop;
  /** Step 3 hand-drawn masks (brush, linear and radial gradients) with their edits. */
  manual: ManualAdjustment[];
}

/** One brush stroke: points on the upright frame (0–1); size is a share of its width. */
export interface BrushStroke {
  points: [number, number][];
  size: number;
  feather: number;
  flow: number;
  erase: boolean;
}

/** A hand-drawn mask; positions are fractions of the upright (uncropped) frame. */
export type ManualShape =
  | { kind: "linear"; x0: number; y0: number; x1: number; y1: number }
  | { kind: "radial"; cx: number; cy: number; rx: number; ry: number; angle: number; feather: number; invert: boolean }
  | { kind: "brush"; strokes: BrushStroke[] };

export interface ManualAdjustment {
  shape: ManualShape;
  exposure: number;
  contrast: number;
  saturation: number;
  /** Temperature: cooler (−) or warmer (+). */
  warmth: number;
  tint: number;
  clarity: number;
  dehaze: number;
}

export function newManual(kind: ManualShape["kind"]): ManualAdjustment {
  const shape: ManualShape =
    kind === "linear"
      ? { kind, x0: 0.5, y0: 0.05, x1: 0.5, y1: 0.45 }
      : kind === "radial"
        ? { kind, cx: 0.5, cy: 0.5, rx: 0.22, ry: 0.28, angle: 0, feather: 50, invert: false }
        : { kind, strokes: [] };
  return { shape, exposure: 0, contrast: 0, saturation: 0, warmth: 0, tint: 0, clarity: 0, dehaze: 0 };
}

export interface SourceRef {
  path: string;
  sha256: string;
  format: string;
  make: string;
  model: string;
}

export interface DevelopDocument {
  version: number;
  source: SourceRef;
  adjustments: Adjustments;
}

export interface TemperatureTint {
  temperature: number;
  tint: number;
}

/** EXIF rationals are [numerator, denominator]. */
export type Ratio = [number, number];

export interface CaptureMetadata {
  make: string;
  model: string;
  dateTimeOriginal: string | null;
  exposureTime: Ratio | null;
  fNumber: Ratio | null;
  focalLength: Ratio | null;
  exposureBias: Ratio | null;
  iso: number | null;
  lensMake: string | null;
  lensModel: string | null;
  gps: { latitude: Ratio[] | null; longitude: Ratio[] | null } | null;
}

export type ExportFormat = "tiff" | "psd" | "dng" | "jpeg" | "png";

export interface ExportOptions {
  format: ExportFormat;
  /** TIFF and PSD; a DNG is always linear Rec.2020. */
  colorSpace: OutputSpace;
  includeLocation: boolean;
  /** Subject, sky and skin masks as named alpha channels (Photoshop). */
  aiMasks: boolean;
  /** Depth map as an alpha channel. */
  depthChannel: boolean;
  /** Long edge in pixels; null = full size. */
  longEdge: number | null;
}

/** A photo editor the export can be handed to. */
export interface HandoffApp {
  name: string;
  path: string;
}

export interface ImageInfo {
  path: string;
  name: string;
  format: string;
  make: string;
  model: string;
  width: number;
  height: number;
  monochrome: boolean;
  asShot: TemperatureTint | null;
  document: DevelopDocument;
  capture: CaptureMetadata;
  loadedFrom: string | null;
  /** Source of the lens profile ("Camera (DNG)", "Lensfun: …"), if any. */
  lensProfile: string | null;
  /** JPEG or PNG: no enhanced-DNG export. */
  bitmap: boolean;
}

export interface FileEntry {
  path: string;
  name: string;
  format: string;
  hasEdits: boolean;
}

export interface SaveReport {
  jsonPath: string;
  xmpPath: string | null;
  warning: string | null;
}

export type OutputSpace = "srgb" | "displayP3" | "proPhoto";

export interface ExportReport {
  path: string;
  format: ExportFormat;
  width: number;
  height: number;
  colorSpace: string;
  bytes: number;
  wroteExif: boolean;
  wroteLocation: boolean;
  alphaChannels: string[];
  developMs: number;
  writeMs: number;
}

/** A model-backed mask kind (the file-level status). */
export type ModelKind = "subject" | "sky";

export interface ModelStatus {
  kind: ModelKind;
  file: string;
  available: boolean;
}

export interface TargetStatus {
  target: MaskTarget;
  available: boolean;
  source: string;
}

export interface MaskModels {
  dir: string;
  models: ModelStatus[];
  /** Step 6 depth model. */
  depth: { file: string; available: boolean };
  /** Face parsing (eye and hair masks). */
  face: { file: string; available: boolean };
  /** Every Step 3 mask and whether the installed models can make it. */
  targets: TargetStatus[];
  /** Lenses in the built-in Lensfun database. */
  lensDatabase: number;
}

export interface AutoTone {
  exposure: number;
  tone: Tone;
}

export interface AutoUpright {
  rotation: number;
  vertical: number;
}

/**
 * Upright soft mask in preview framing: one byte per pixel, 255 = inside. The depth
 * map uses the same shape, 255 = nearest.
 */
export interface Mask {
  kind: MaskTarget | "depth";
  width: number;
  height: number;
  alpha: Uint8Array<ArrayBuffer>;
  /** Mean coverage of the frame, 0–1. */
  coverage: number;
  inferMs: number;
}

export interface Preview {
  width: number;
  height: number;
  rgba: Uint8ClampedArray<ArrayBuffer>;
  /** R, G, B histograms, 256 bins each. */
  histogram: [Uint32Array, Uint32Array, Uint32Array];
}

/** Matches `Adjustments::default()` in Rust. */
export function defaultAdjustments(): Adjustments {
  return {
    whiteBalance: { mode: "asShot", temperature: 6500, tint: 0 },
    highlightRecovery: true,
    demosaic: "auto",
    lens: {
      profile: true,
      distortion: { enabled: false, k1: 0, k2: 0, k3: 0, p1: 0, p2: 0, cx: 0, cy: 0 },
      chromaticAberration: { enabled: false, red: 0, blue: 0 },
      rotation: 0,
      vertical: 0,
    },
    exposure: 0,
    tone: defaultTone(),
    noiseReduction: { luminance: 0, color: 25 },
    local: [],
    texture: {
      clarity: 0,
      microTexture: 0,
      blemishSmoothing: 0,
      specularBalance: 0,
      characterLines: 0,
      retouchSubjectOnly: false,
    },
    color: defaultColorGrade(),
    atmosphere: defaultAtmosphere(),
    curves: {
      rgb: defaultToneCurve(),
      red: defaultToneCurve(),
      green: defaultToneCurve(),
      blue: defaultToneCurve(),
      sCurve: { enabled: false, amount: 50, pivot: 50 },
    },
    splitToning: { highlightHue: 40, highlightSaturation: 0, shadowHue: 215, shadowSaturation: 0, balance: 0 },
    finishing: {
      grain: 0,
      grainSize: 25,
      grainRoughness: 50,
      vignette: 0,
      vignetteMidpoint: 50,
      vignetteFeather: 50,
      vignetteRoundness: 0,
    },
    style: { id: "", amount: 100, skinProtection: 0, blend: [] },
    lut: { name: "", amount: 100 },
    crop: defaultCrop(),
    manual: [],
  };
}

export function defaultTone(): Tone {
  return { contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0, vibrance: 0, saturation: 0, dehaze: 0 };
}

export function defaultLocal(mask: MaskTarget): LocalAdjustment {
  return { mask, exposure: 0, contrast: 0, saturation: 0, warmth: 0, clarity: 0, tint: 0 };
}

export function defaultToneCurve(): ToneCurve {
  return { shadows: 0, darks: 0, lights: 0, highlights: 0, black: 0, white: 0, points: [] };
}

export function defaultAtmosphere(): Atmosphere {
  return {
    glow: 0,
    glowSize: 50,
    glowWarmth: 0,
    glowSubjectOnly: false,
    fog: 0,
    fogStart: 30,
    fogWarmth: 0,
    shafts: 0,
    shaftLength: 60,
    shaftWarmth: 40,
    shaftAuto: true,
    shaftX: 0.5,
    shaftY: 0.15,
    lights: [],
  };
}

export function defaultColorGrade(): ColorGrade {
  const zero = () => ({ hue: 0, saturation: 0, luminance: 0 });
  const wheel = () => ({ hue: 0, amount: 0, luminance: 0 });
  return {
    hsl: Object.fromEntries(HSL_BANDS.map((b) => [b, zero()])) as HslBands,
    wheels: { shadows: wheel(), midtones: wheel(), highlights: wheel() },
    skinProtection: 0,
    foliage: zero(),
    background: { hue: 210, amount: 0, saturation: 0, luminance: 0 },
  };
}

/** Section 2.1: rule-based scene reading of one photo. */
export interface SceneAnalysis {
  genres: { id: string; label: string; score: number; evidence: string }[];
  lighting: {
    /** The camera's white-balance estimate (RAW only). */
    colorTemperature: number | null;
    /** Measured from the pixels. */
    ambientTemperature: number;
    ambientLabel: string;
    dynamicRangeEv: number;
    highlightsClipped: number;
    shadowsCrushed: number;
    key: string;
    hardness: number;
    hardnessLabel: string;
    direction: string | null;
    backlit: boolean;
    haze: number;
    hazeLabel: string;
    snow: number;
    timeOfDay: string | null;
  };
  skin: {
    coverage: number;
    toneDepth: number;
    toneLabel: string;
    undertone: string;
    shine: number;
    shineLabel: string;
    texture: number;
    textureLabel: string;
  } | null;
  composition: {
    subjectCoverage: number | null;
    skyCoverage: number | null;
    depthRange: number | null;
    lineStrength: number;
  };
  /** Five dominant colours of the photo as shot (before any edit). */
  palette: Swatch[];
  /** Luminance of the photo as shot: 64-bin display histogram (shares), mean, median. */
  luminance: { histogram: number[]; mean: number; median: number };
  limits: string[];
  analysisMs: number;
}

export interface Swatch {
  hex: string;
  weight: number;
}

/** Section 2.2: a folder split into groups shot under similar conditions. */
export interface StoryArc {
  groups: ShotGroup[];
  frames: FrameSummary[];
  analysisMs: number;
}

export interface ShotGroup {
  id: number;
  label: string;
  frames: string[];
  hero: string;
  palette: Swatch[];
  /** Why this group starts a new one (empty for the first). */
  splitReason: string;
}

export interface FrameSummary {
  path: string;
  name: string;
  captured: string | null;
  sceneEv: number | null;
  lightness: number;
  cast: [number, number];
  gps: [number, number] | null;
  skin: number;
  group: number;
  error: string | null;
}

export interface SyncReport {
  frames: {
    path: string;
    exposureDelta: number;
    whiteBalanceShifted: boolean;
    textureScale: number;
    skinProtection: number;
  }[];
  skipped: { path: string; reason: string }[];
}

/** A look measured from a reference photo (AI Style Learning), kept across sessions. */
export interface LearnedStyle {
  id: string;
  name: string;
  source: string;
  createdAt: number;
  signature: {
    tone: { black: number; white: number; median: number; contrast: number };
    skin: { l: number; hue: number; chroma: number; richness: number; relativeL: number; specular: number } | null;
    bands: { hue: number; chroma: number; share: number }[];
    foliage: { hue: number; chroma: number; share: number } | null;
    backgroundChroma: number | null;
  };
  palette: Swatch[];
}

/** A saved look (no framing, white balance or exposure). */
export interface Preset {
  id: string;
  name: string;
  createdAt: number;
  adjustments: Adjustments;
}
