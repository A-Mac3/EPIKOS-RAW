import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Mask, Preview } from "../types";

/** Lightroom-style red mask overlay. */
const OVERLAY_RGB = [255, 48, 64];
const OVERLAY_OPACITY = 0.55;

interface Props {
  preview: Preview | null;
  busy: boolean;
  error: string | null;
  loading: boolean;
  showingBefore: boolean;
  /** Mask drawn over the image, stretched to its frame. */
  overlay: Mask | null;
  /** Reports the drawable area in device pixels so previews match the screen. */
  onResize: (width: number, height: number) => void;
}

export function Viewer({ preview, busy, error, loading, showingBefore, overlay, onResize }: Props) {
  const frame = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const overlayCanvas = useRef<HTMLCanvasElement>(null);
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

  useEffect(() => {
    const c = overlayCanvas.current;
    if (!c || !overlay) return;
    c.width = overlay.width;
    c.height = overlay.height;
    const rgba = new Uint8ClampedArray(overlay.width * overlay.height * 4);
    for (let i = 0; i < overlay.alpha.length; i++) {
      rgba[i * 4] = OVERLAY_RGB[0];
      rgba[i * 4 + 1] = OVERLAY_RGB[1];
      rgba[i * 4 + 2] = OVERLAY_RGB[2];
      rgba[i * 4 + 3] = overlay.alpha[i] * OVERLAY_OPACITY;
    }
    c.getContext("2d")!.putImageData(new ImageData(rgba, overlay.width, overlay.height), 0, 0);
  }, [overlay]);

  // Fit the whole image to the frame. The engine renders close to the frame's device
  // resolution, so any scaling here is small.
  let style: React.CSSProperties = { display: "none" };
  if (preview && cssSize.w > 0) {
    const scale = Math.min(cssSize.w / preview.width, cssSize.h / preview.height);
    style = { width: preview.width * scale, height: preview.height * scale };
  }

  return (
    <div className="viewer" ref={frame}>
      <div className="viewer-image" style={style}>
        <canvas ref={canvas} />
        {/* Always mounted so hiding it (e.g. while showing Before) keeps the drawing. */}
        <canvas
          ref={overlayCanvas}
          className="viewer-overlay"
          style={{ display: overlay && !showingBefore ? "block" : "none" }}
        />
      </div>
      {showingBefore && preview && <div className="viewer-tag">Before</div>}
      {(loading || (busy && !preview)) && !error && <div className="viewer-status">Developing…</div>}
      {error && <div className="viewer-status is-error">{error}</div>}
    </div>
  );
}
