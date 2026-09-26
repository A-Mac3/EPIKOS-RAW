import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Preview } from "../types";
import { smoothPath } from "../spline";

const HEIGHT = 96;
/** Points per curve after smoothing: enough for detail, few enough for clean splines. */
const POINTS = 128;
const CHANNELS = [
  { rgb: [255, 84, 84], name: "Red" },
  { rgb: [96, 214, 112], name: "Green" },
  { rgb: [92, 146, 255], name: "Blue" },
] as const;

/** Luminance histogram of the displayed image (every other pixel; plenty for 256 bins). */
function lumaHistogram(p: Preview): Uint32Array {
  const out = new Uint32Array(256);
  const px = p.rgba;
  for (let i = 0; i < px.length; i += 8) {
    out[Math.round(0.2126 * px[i] + 0.7152 * px[i + 1] + 0.0722 * px[i + 2])]++;
  }
  return out;
}

/** 256 bins → `POINTS` values, Gaussian-smoothed, square-root scaled to 0–1. The end
 * bins are left out of the scale so a clipped sky doesn't flatten the rest. */
function curve(bins: ArrayLike<number>, peak: number): number[] {
  const n = 256;
  const k = [0.06, 0.24, 0.4, 0.24, 0.06];
  const smooth = Array.from({ length: n }, (_, i) =>
    k.reduce((s, w, j) => s + w * bins[Math.min(n - 1, Math.max(0, i + j - 2))], 0),
  );
  const step = n / POINTS;
  return Array.from({ length: POINTS }, (_, i) => {
    let s = 0;
    for (let j = Math.floor(i * step); j < Math.floor((i + 1) * step); j++) s += smooth[j];
    return Math.min(1, Math.sqrt(s / step) / peak);
  });
}

/**
 * RGB and luminance histogram of the displayed image: anti-aliased cubic splines at
 * the screen's full pixel density, soft gradient fills blended in screen mode, and
 * vector outlines. Square-root scaled so shadows stay visible.
 */
export function Histogram({ preview }: { preview: Preview | null }) {
  const box = useRef<HTMLDivElement>(null);
  const ref = useRef<HTMLCanvasElement>(null);
  const [width, setWidth] = useState(256);

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(Math.max(64, Math.round(e.contentRect.width))));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const data = useMemo(() => {
    if (!preview) return null;
    const luma = lumaHistogram(preview);
    let peak = 1;
    for (const ch of [...preview.histogram, luma]) {
      for (let i = 2; i < 254; i++) peak = Math.max(peak, ch[i]);
    }
    const p = Math.sqrt(peak);
    return { rgb: preview.histogram.map((h) => curve(h, p)), luma: curve(luma, p) };
  }, [preview]);

  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    c.width = Math.round(width * dpr);
    c.height = Math.round(HEIGHT * dpr);
    c.style.width = `${width}px`;
    c.style.height = `${HEIGHT}px`;
    const ctx = c.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, HEIGHT);

    // Quarter guides.
    ctx.strokeStyle = "rgba(255,255,255,0.06)";
    ctx.lineWidth = 1;
    for (const t of [0.25, 0.5, 0.75]) {
      const x = Math.round(t * width) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, HEIGHT);
      ctx.stroke();
    }
    if (!data) return;

    const top = 4;
    const pts = (v: number[]) =>
      v.map((y, i) => [(i / (v.length - 1)) * width, HEIGHT - y * (HEIGHT - top)] as [number, number]);
    const area = (v: number[]) => {
      const p = new Path2D(smoothPath(pts(v)));
      p.lineTo(width, HEIGHT);
      p.lineTo(0, HEIGHT);
      p.closePath();
      return p;
    };
    const fill = (rgb: readonly number[], a0: number, a1: number) => {
      const g = ctx.createLinearGradient(0, top, 0, HEIGHT);
      g.addColorStop(0, `rgba(${rgb.join(",")},${a0})`);
      g.addColorStop(1, `rgba(${rgb.join(",")},${a1})`);
      return g;
    };

    // Luminance underneath, then the channels blended in screen mode so overlaps
    // mix towards white as on a scope.
    ctx.globalCompositeOperation = "source-over";
    ctx.fillStyle = fill([220, 220, 228], 0.2, 0.04);
    ctx.fill(area(data.luma));
    ctx.globalCompositeOperation = "screen";
    data.rgb.forEach((v, i) => {
      ctx.fillStyle = fill(CHANNELS[i].rgb, 0.5, 0.06);
      ctx.fill(area(v));
    });
    ctx.globalCompositeOperation = "source-over";
    ctx.lineJoin = "round";
    data.rgb.forEach((v, i) => {
      ctx.strokeStyle = `rgba(${CHANNELS[i].rgb.join(",")},0.85)`;
      ctx.lineWidth = 1;
      ctx.stroke(new Path2D(smoothPath(pts(v))));
    });
    ctx.strokeStyle = "rgba(240,240,245,0.9)";
    ctx.lineWidth = 1.25;
    ctx.stroke(new Path2D(smoothPath(pts(data.luma))));
  }, [data, width]);

  const clipped = (bin: number) =>
    preview ? preview.histogram.some((ch) => ch[bin] > (preview.width * preview.height) / 500) : false;

  return (
    <div className="histogram" ref={box}>
      <canvas ref={ref} aria-label="RGB and luminance histogram" />
      <div className="histogram-clip">
        <span className={clipped(0) ? "is-on" : ""} title="Shadows clipped">
          ◀
        </span>
        <span className={clipped(255) ? "is-on" : ""} title="Highlights clipped">
          ▶
        </span>
      </div>
    </div>
  );
}
