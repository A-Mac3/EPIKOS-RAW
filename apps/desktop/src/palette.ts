import type { Swatch } from "./types";

type Lab = [number, number, number];

const toLinear = (c: number) => {
  const v = c / 255;
  return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
};

const toByte = (v: number) => {
  const c = Math.min(1, Math.max(0, v));
  const e = c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
  return Math.round(e * 255);
};

/** Display sRGB (0–255) → Oklab. */
function oklab(r8: number, g8: number, b8: number): Lab {
  const [r, g, b] = [toLinear(r8), toLinear(g8), toLinear(b8)];
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

function hex([L, a, b]: Lab): string {
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  const rgb = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
  return `#${rgb.map((v) => toByte(v).toString(16).padStart(2, "0")).join("")}`;
}

const dist = (x: Lab, y: Lab) => (x[0] - y[0]) ** 2 + (x[1] - y[1]) ** 2 + (x[2] - y[2]) ** 2;

/**
 * The five dominant colours of a rendered preview: k-means in Oklab over a pixel
 * sample, seeded the same way as the engine's as-shot palette so the two compare.
 */
export function dominantColors(rgba: Uint8ClampedArray, k = 5): Swatch[] {
  const n = rgba.length / 4;
  const step = Math.max(1, Math.floor(n / 4000));
  const samples: Lab[] = [];
  for (let i = 0; i < n; i += step) samples.push(oklab(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]));
  if (samples.length === 0) return [];
  k = Math.min(k, samples.length);
  const byL = samples.slice().sort((x, y) => x[0] - y[0]);
  const centers: Lab[] = Array.from({ length: k }, (_, j) => byL[Math.floor(((j * 2 + 1) * byL.length) / (2 * k))]);
  const assign = new Uint8Array(samples.length);
  for (let iter = 0; iter < 12; iter++) {
    samples.forEach((s, i) => {
      let best = 0;
      for (let j = 1; j < k; j++) if (dist(s, centers[j]) < dist(s, centers[best])) best = j;
      assign[i] = best;
    });
    for (let j = 0; j < k; j++) {
      const sum: Lab = [0, 0, 0];
      let count = 0;
      samples.forEach((s, i) => {
        if (assign[i] !== j) return;
        sum[0] += s[0];
        sum[1] += s[1];
        sum[2] += s[2];
        count++;
      });
      if (count > 0) centers[j] = [sum[0] / count, sum[1] / count, sum[2] / count];
    }
  }
  const counts = new Array<number>(k).fill(0);
  assign.forEach((a) => counts[a]++);
  return centers
    .map((c, j) => ({ hex: hex(c), weight: counts[j] / samples.length }))
    .filter((s) => s.weight > 0)
    .sort((x, y) => y.weight - x.weight);
}

/** CIE L*a*b* (D65) of a display sRGB hex colour, for swatch tooltips. */
export function hexToLab(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map(toLinear);
  const x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
  const y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  const z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
  const f = (t: number) => (t > 216 / 24389 ? Math.cbrt(t) : (24389 / 27 * t + 16) / 116);
  const [fx, fy, fz] = [f(x), f(y), f(z)];
  return [116 * fy - 16, 500 * (fx - fy), 200 * (fy - fz)];
}
