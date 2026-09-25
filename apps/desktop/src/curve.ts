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

/**
 * Monotone cubic through control points (Fritsch–Carlson), mirroring `PointCurve` in
 * tone.rs. Returns null for the identity. Corners (0,0) and (1,1) are implied.
 */
export function pointSpline(points: [number, number][]): ((x: number) => number) | null {
  const clamp = (v: number) => Math.min(1, Math.max(0, v));
  let pts = points.filter(([x, y]) => Number.isFinite(x) && Number.isFinite(y)).map(([x, y]) => [clamp(x), clamp(y)]);
  if (pts.every(([x, y]) => Math.abs(x - y) < 1e-6)) return null;
  pts.sort((a, b) => a[0] - b[0]);
  pts = pts.filter((p, i) => i === 0 || Math.abs(p[0] - pts[i - 1][0]) >= 1e-4);
  if (pts[0][0] > 0) pts.unshift([0, 0]);
  if (pts[pts.length - 1][0] < 1) pts.push([1, 1]);
  const xs = pts.map((p) => p[0]);
  const ys = pts.map((p) => p[1]);
  const n = xs.length;
  const d = xs.slice(0, -1).map((_, k) => (ys[k + 1] - ys[k]) / (xs[k + 1] - xs[k]));
  const ms = new Array<number>(n).fill(0);
  ms[0] = d[0];
  ms[n - 1] = d[n - 2];
  for (let k = 1; k < n - 1; k++) ms[k] = d[k - 1] * d[k] <= 0 ? 0 : 0.5 * (d[k - 1] + d[k]);
  for (let k = 0; k < n - 1; k++) {
    if (d[k] === 0) {
      ms[k] = 0;
      ms[k + 1] = 0;
      continue;
    }
    const a = ms[k] / d[k];
    const b = ms[k + 1] / d[k];
    const h = a * a + b * b;
    if (h > 9) {
      const t = 3 / Math.sqrt(h);
      ms[k] = t * a * d[k];
      ms[k + 1] = t * b * d[k];
    }
  }
  return (xIn: number) => {
    const x = clamp(xIn);
    let k = 0;
    while (k < n - 2 && xs[k + 1] <= x) k++;
    const h = xs[k + 1] - xs[k];
    const t = (x - xs[k]) / h;
    const t2 = t * t;
    const t3 = t2 * t;
    return clamp(
      (2 * t3 - 3 * t2 + 1) * ys[k] +
        (t3 - 2 * t2 + t) * h * ms[k] +
        (-2 * t3 + 3 * t2) * ys[k + 1] +
        (t3 - t2) * h * ms[k + 1],
    );
  };
}

/** `n + 1` samples of the full curve (parametric, forced monotonic, then points). */
export function curvePoints(c: ToneCurve, n = 96): [number, number][] {
  const spline = pointSpline(c.points);
  const pts: [number, number][] = [];
  let prev = -Infinity;
  for (let i = 0; i <= n; i++) {
    const x = i / n;
    prev = Math.max(prev, curveShape(c, x));
    pts.push([x, spline ? spline(prev) : prev]);
  }
  return pts;
}

export const isIdentity = (c: ToneCurve) =>
  [c.shadows, c.darks, c.lights, c.highlights, c.black, c.white].every((v) => v === 0) &&
  c.points.every(([x, y]) => Math.abs(x - y) < 1e-6);

/** Where a baked curve gets its control points (mirrors BAKE_AT in tone.rs). */
const BAKE_AT = [0, 0.25, 0.5, 0.75, 1];

/**
 * The curve as points only: parametric shaping (from an older sidecar) is sampled into
 * control points and zeroed, like `bake_tone_curve` in tone.rs. The curve canvas
 * edits points, so this is what it shows and edits.
 */
export function bakeCurve(c: ToneCurve): ToneCurve {
  if ([c.shadows, c.darks, c.lights, c.highlights, c.black, c.white].every((v) => v === 0)) return c;
  const samples = curvePoints(c, 400);
  const at = (x: number) => samples[Math.round(x * 400)][1];
  return {
    shadows: 0,
    darks: 0,
    lights: 0,
    highlights: 0,
    black: 0,
    white: 0,
    points: BAKE_AT.map((x) => [x, at(x)] as [number, number]),
  };
}
