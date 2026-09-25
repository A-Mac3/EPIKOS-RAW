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
}

/** Step 6. Strengths 0…100, warmth −100 (cool) … 100 (gold). */
export interface Atmosphere {
  glow: number;
  glowSize: number;
  glowWarmth: number;
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
}

export const CURVE_CHANNELS = ["rgb", "red", "green", "blue"] as const;
export type CurveChannel = (typeof CURVE_CHANNELS)[number];
export type Curves = Record<CurveChannel, ToneCurve>;

/** Hue in degrees on the HSV wheel, saturation 0…100, balance −100…100. */
export interface SplitToning {
  highlightHue: number;
  highlightSaturation: number;
  shadowHue: number;
  shadowSaturation: number;
  balance: number;
}

/** Parametric style layered on the manual Step 4–6 settings. `id` empty = none. */
export interface StyleRef {
  id: string;
  amount: number;
  skinProtection: number;
}

export interface StyleInfo {
  id: string;
  name: string;
  world: string;
  description: string;
  skinProtection: number;
  swatch: [string, string];
}

export interface Adjustments {
  whiteBalance: WhiteBalance;
  highlightRecovery: boolean;
  demosaic: DemosaicMode;
  lens: {
    distortion: DistortionCoeffs;
    chromaticAberration: ChromaticAberration;
  };
  exposure: number;
  noiseReduction: NoiseReduction;
  texture: Texture;
  color: ColorGrade;
  atmosphere: Atmosphere;
  curves: Curves;
  splitToning: SplitToning;
  style: StyleRef;
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

export interface ExportOptions {
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

export type MaskKind = "subject" | "sky";

export interface ModelStatus {
  kind: MaskKind;
  file: string;
  available: boolean;
}

export interface MaskModels {
  dir: string;
  models: ModelStatus[];
  /** Step 6 depth model. */
  depth: { file: string; available: boolean };
}

/**
 * Upright soft mask in preview framing: one byte per pixel, 255 = inside. The depth
 * map uses the same shape, 255 = nearest.
 */
export interface Mask {
  kind: MaskKind | "depth";
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
      distortion: { enabled: false, k1: 0, k2: 0, k3: 0, p1: 0, p2: 0, cx: 0, cy: 0 },
      chromaticAberration: { enabled: false, red: 0, blue: 0 },
    },
    exposure: 0,
    noiseReduction: { luminance: 0, color: 25 },
    texture: { clarity: 0, microTexture: 0, blemishSmoothing: 0, specularBalance: 0 },
    color: defaultColorGrade(),
    atmosphere: defaultAtmosphere(),
    curves: {
      rgb: defaultToneCurve(),
      red: defaultToneCurve(),
      green: defaultToneCurve(),
      blue: defaultToneCurve(),
    },
    splitToning: { highlightHue: 40, highlightSaturation: 0, shadowHue: 215, shadowSaturation: 0, balance: 0 },
    style: { id: "", amount: 100, skinProtection: 0 },
  };
}

export function defaultToneCurve(): ToneCurve {
  return { shadows: 0, darks: 0, lights: 0, highlights: 0, black: 0, white: 0 };
}

export function defaultAtmosphere(): Atmosphere {
  return {
    glow: 0,
    glowSize: 50,
    glowWarmth: 0,
    fog: 0,
    fogStart: 30,
    fogWarmth: 0,
    shafts: 0,
    shaftLength: 60,
    shaftWarmth: 40,
    shaftAuto: true,
    shaftX: 0.5,
    shaftY: 0.15,
  };
}

export function defaultColorGrade(): ColorGrade {
  const zero = () => ({ hue: 0, saturation: 0, luminance: 0 });
  const wheel = () => ({ hue: 0, amount: 0, luminance: 0 });
  return {
    hsl: Object.fromEntries(HSL_BANDS.map((b) => [b, zero()])) as HslBands,
    wheels: { shadows: wheel(), midtones: wheel(), highlights: wheel() },
    skinProtection: 0,
  };
}
