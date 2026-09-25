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
}

/** Upright soft mask in preview framing: one byte per pixel, 255 = inside. */
export interface Mask {
  kind: MaskKind;
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
  };
}
