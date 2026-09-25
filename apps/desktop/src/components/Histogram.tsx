import { useEffect, useRef } from "react";
import type { Preview } from "../types";

const W = 256;
const H = 96;
const COLORS = ["rgb(230,70,70)", "rgb(80,200,90)", "rgb(80,130,240)"];

/** RGB histogram of the displayed image, square-root scaled so shadows stay visible. */
export function Histogram({ preview }: { preview: Preview | null }) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const ctx = ref.current?.getContext("2d");
    if (!ctx) return;
    ctx.globalCompositeOperation = "source-over";
    ctx.clearRect(0, 0, W, H);
    if (!preview) return;

    // Ignore the extreme bins when scaling so a clipped sky doesn't flatten the rest.
    let peak = 1;
    for (const ch of preview.histogram) {
      for (let i = 1; i < 255; i++) peak = Math.max(peak, ch[i]);
    }
    const scale = (H - 1) / Math.sqrt(peak);

    ctx.globalCompositeOperation = "lighter";
    preview.histogram.forEach((ch, c) => {
      ctx.fillStyle = COLORS[c];
      ctx.globalAlpha = 0.75;
      ctx.beginPath();
      ctx.moveTo(0, H);
      for (let i = 0; i < 256; i++) ctx.lineTo(i, H - Math.min(H, Math.sqrt(ch[i]) * scale));
      ctx.lineTo(W, H);
      ctx.closePath();
      ctx.fill();
    });
    ctx.globalAlpha = 1;
  }, [preview]);

  const clipped = (bin: number) =>
    preview ? preview.histogram.some((ch) => ch[bin] > (preview.width * preview.height) / 500) : false;

  return (
    <div className="histogram">
      <canvas ref={ref} width={W} height={H} aria-label="RGB histogram" />
      <div className="histogram-clip">
        <span className={clipped(0) ? "is-on" : ""} title="Shadows clipped">◀</span>
        <span className={clipped(255) ? "is-on" : ""} title="Highlights clipped">▶</span>
      </div>
    </div>
  );
}
