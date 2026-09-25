// The Step 7 curve shape, mirrored from crates/epikos-pipeline/src/look/tone.rs so
// the curve graph draws exactly what the engine applies. Keep the two in step.
import type { ToneCurve } from "./types";

const REGION_AMPLITUDE = 0.12;
const POINT_RANGE = 0.15;
const BINOM = [1, 5, 10, 10, 5, 1];

/** Degree-5 Bernstein basis `i` (1…4), scaled to peak at 1. */
function region(i: number, x: number): number {
  const b = (t: number) => BINOM[i] * t ** i * (1 - t) ** (5 - i);
  return b(x) / b(i / 5);
}

const k = (v: number) => Math.min(1, Math.max(-1, v / 100));

/** Encoded in → encoded out on [0, 1]. */
export function curveShape(c: ToneCurve, x: number): number {
  const crush = POINT_RANGE * Math.max(0, -k(c.black));
  const clip = POINT_RANGE * Math.max(0, k(c.white));
  const t = Math.min(1, Math.max(0, (x - crush) / (1 - crush - clip)));
  let y = t;
  [c.shadows, c.darks, c.lights, c.highlights].forEach((v, i) => {
    y += REGION_AMPLITUDE * k(v) * region(i + 1, t);
  });
  const lo = POINT_RANGE * Math.max(0, k(c.black));
  const hi = 1 - POINT_RANGE * Math.max(0, -k(c.white));
  return lo + Math.min(1, Math.max(0, y)) * (hi - lo);
}

/** `n + 1` samples of the curve, forced monotonic like the engine's table. */
export function curvePoints(c: ToneCurve, n = 64): [number, number][] {
  const pts: [number, number][] = [];
  let prev = -Infinity;
  for (let i = 0; i <= n; i++) {
    const x = i / n;
    prev = Math.max(prev, curveShape(c, x));
    pts.push([x, prev]);
  }
  return pts;
}

export const isIdentity = (c: ToneCurve) => Object.values(c).every((v) => v === 0);
