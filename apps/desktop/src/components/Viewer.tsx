import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Preview } from "../types";

interface Props {
  preview: Preview | null;
  busy: boolean;
  error: string | null;
  loading: boolean;
  showingBefore: boolean;
  /** Reports the drawable area in device pixels so previews match the screen. */
  onResize: (width: number, height: number) => void;
}

export function Viewer({ preview, busy, error, loading, showingBefore, onResize }: Props) {
  const frame = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [cssSize, setCssSize] = useState({ w: 0, h: 0 });

  useLayoutEffect(() => {
    const el = frame.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setCssSize({ w: width, h: height });
      const dpr = window.devicePixelRatio || 1;
      onResize(Math.floor(width * dpr), Math.floor(height * dpr));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [onResize]);

  useEffect(() => {
    const c = canvas.current;
    if (!c || !preview) return;
    if (c.width !== preview.width || c.height !== preview.height) {
      c.width = preview.width;
      c.height = preview.height;
    }
    c.getContext("2d")!.putImageData(new ImageData(preview.rgba, preview.width, preview.height), 0, 0);
  }, [preview]);

  // Fit the whole image to the frame. The engine renders close to the frame's device
  // resolution, so any scaling here is small.
  let style: React.CSSProperties = { display: "none" };
  if (preview && cssSize.w > 0) {
    const scale = Math.min(cssSize.w / preview.width, cssSize.h / preview.height);
    style = { width: preview.width * scale, height: preview.height * scale };
  }

  return (
    <div className="viewer" ref={frame}>
      <canvas ref={canvas} style={style} />
      {showingBefore && preview && <div className="viewer-tag">Before</div>}
      {(loading || (busy && !preview)) && !error && <div className="viewer-status">Developing…</div>}
      {error && <div className="viewer-status is-error">{error}</div>}
    </div>
  );
}
