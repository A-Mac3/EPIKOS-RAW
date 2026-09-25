import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Mask, Preview } from "../types";
import { LightPalette, type LightKind } from "./LightPalette";

/** Lightroom-style red mask overlay; depth in cyan (brighter = nearer). */
const OVERLAY_RGB = [255, 48, 64];
const DEPTH_RGB = [40, 200, 255];
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
  /** Light-source marker, 0–1 across and down. */
  marker?: { x: number; y: number } | null;
  /** When set, a click on the image reports its 0–1 position instead. */
  onPick?: ((x: number, y: number) => void) | null;
  /** 3D virtual lights: draggable markers, smaller the farther away. */
  lights?: { x: number; y: number; depth: number; reach: number }[];
  selectedLight?: number | null;
  onSelectLight?: (i: number) => void;
  onMoveLight?: (i: number, x: number, y: number) => void;
  onMoveLightEnd?: () => void;
  /** Scroll over a light: move it nearer (< 0) or farther (> 0). */
  onLightDepth?: (i: number, delta: number) => void;
  /** A light dragged from the palette was dropped at (x, y), 0–1 on the photo. */
  onDropLight?: (kind: LightKind, x: number, y: number) => void;
  /** The light palette may be used soon: fetch what a drop needs (the depth map). */
  onLightPrepare?: () => void;
}

export function Viewer({
  preview,
  busy,
  error,
  loading,
  showingBefore,
  overlay,
  onResize,
  marker,
  onPick,
  lights = [],
  selectedLight = null,
  onSelectLight,
  onMoveLight,
  onMoveLightEnd,
  onLightDepth,
  onDropLight,
  onLightPrepare,
}: Props) {
  const imageRef = useRef<HTMLDivElement>(null);
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
    const color = overlay.kind === "depth" ? DEPTH_RGB : OVERLAY_RGB;
    for (let i = 0; i < overlay.alpha.length; i++) {
      rgba[i * 4] = color[0];
      rgba[i * 4 + 1] = color[1];
      rgba[i * 4 + 2] = color[2];
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
      <div
        ref={imageRef}
        className={`viewer-image${onPick ? " is-picking" : ""}`}
        style={style}
        onClick={(e) => {
          if (!onPick) return;
          const box = e.currentTarget.getBoundingClientRect();
          const clamp = (v: number) => Math.min(1, Math.max(0, v));
          onPick(clamp((e.clientX - box.left) / box.width), clamp((e.clientY - box.top) / box.height));
        }}
      >
        <canvas ref={canvas} />
        {/* Always mounted so hiding it (e.g. while showing Before) keeps the drawing. */}
        <canvas
          ref={overlayCanvas}
          className="viewer-overlay"
          style={{ display: overlay && !showingBefore ? "block" : "none" }}
        />
        {!showingBefore &&
          lights.map((l, i) => {
            const size = 26 - 14 * l.depth;
            const selected = i === selectedLight;
            // Reach as the engine models it: 5–65 % of the frame's long side.
            const box = imageRef.current?.getBoundingClientRect();
            const reachPx = box ? (0.05 + 0.6 * (l.reach / 100)) * Math.max(box.width, box.height) : 0;
            return (
              <div key={i} className="viewer-light-anchor" style={{ left: `${l.x * 100}%`, top: `${l.y * 100}%` }}>
                {selected && <span className="viewer-light-reach" style={{ width: reachPx * 2, height: reachPx * 2 }} />}
                <span
                  className={`viewer-light${selected ? " is-selected" : ""}`}
                  style={{ width: size, height: size }}
                  title={`Light ${i + 1}: drag to move, scroll to change depth`}
                  onPointerDown={(e) => {
                    e.stopPropagation();
                    e.currentTarget.setPointerCapture(e.pointerId);
                    onSelectLight?.(i);
                  }}
                  onPointerMove={(e) => {
                    if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
                    const b = imageRef.current!.getBoundingClientRect();
                    const clamp = (v: number) => Math.min(1, Math.max(0, v));
                    onMoveLight?.(i, clamp((e.clientX - b.left) / b.width), clamp((e.clientY - b.top) / b.height));
                  }}
                  onPointerUp={() => onMoveLightEnd?.()}
                  onClick={(e) => e.stopPropagation()}
                  onWheel={(e) => {
                    e.preventDefault();
                    onSelectLight?.(i);
                    onLightDepth?.(i, Math.sign(e.deltaY) * 0.02);
                  }}
                />
                {selected && (
                  <span className="viewer-light-label">
                    Light {i + 1} · depth {Math.round(l.depth * 100)}%
                    <small>scroll: nearer / farther · ⌫ remove</small>
                  </span>
                )}
              </div>
            );
          })}
        {marker && !showingBefore && (
          <span
            className="viewer-marker"
            style={{ left: `${marker.x * 100}%`, top: `${marker.y * 100}%` }}
            title="Light source"
            aria-hidden
          />
        )}
      </div>
      {onPick && <div className="viewer-tag">Click the image to place the light</div>}
      {preview && onDropLight && !showingBefore && (
        <LightPalette
          onPrepare={onLightPrepare}
          onDrop={(kind, cx, cy) => {
            const b = imageRef.current?.getBoundingClientRect();
            if (!b || cx < b.left || cx > b.right || cy < b.top || cy > b.bottom) return;
            onDropLight(kind, (cx - b.left) / b.width, (cy - b.top) / b.height);
          }}
        />
      )}
      {showingBefore && preview && <div className="viewer-tag">Before</div>}
      {(loading || (busy && !preview)) && !error && <div className="viewer-status">Developing…</div>}
      {error && <div className="viewer-status is-error">{error}</div>}
    </div>
  );
}
