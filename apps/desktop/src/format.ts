import type { CaptureMetadata, Ratio } from "./types";

const value = ([n, d]: Ratio) => (d === 0 ? 0 : n / d);

/** "1/250 s", "0.5 s", "30 s". */
export function shutter(r: Ratio): string {
  const v = value(r);
  if (v <= 0) return "";
  if (v < 0.3) return `1/${Math.round(1 / v)} s`;
  return `${Number(v.toFixed(1))} s`;
}

/** One-line capture summary: "1/250 s · f/2.8 · ISO 400 · 35 mm · XF33mmF1.4". */
export function captureSummary(c: CaptureMetadata): string {
  const parts: string[] = [];
  if (c.exposureTime) parts.push(shutter(c.exposureTime));
  if (c.fNumber) parts.push(`f/${Number(value(c.fNumber).toFixed(1))}`);
  if (c.iso) parts.push(`ISO ${c.iso}`);
  if (c.focalLength) parts.push(`${Number(value(c.focalLength).toFixed(1))} mm`);
  if (c.lensModel) parts.push(c.lensModel);
  return parts.filter(Boolean).join(" · ");
}

export const hasLocation = (c: CaptureMetadata) => !!(c.gps?.latitude && c.gps?.longitude);
