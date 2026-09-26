import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Crop, ManualShape, Mask, Preview } from "../types";
import { CropBar, CropOverlay } from "./CropTool";
import { ManualMaskTool, type BrushSettings } from "./ManualMaskTool";
import { LightPalette, type LightKind } from "./LightPalette";

/** Zoom limits, as a share of the photo's actual pixels (1 = 100 %). */
const MIN_ZOOM = 0.1;
const MAX_ZOOM = 5;
/** Sharper renders follow zooming once it pauses. */
const ZOOM_RENDER_DELAY_MS = 250;

/** The crop tool's state and callbacks (the image shows the whole upright frame). */
export interface CropTool {
  crop: Crop;
  rotation: number;
  onChange: (c: Crop) => void;
  onRotate: (degrees: number) => void;
  onEnd: () => void;
  onReset: () => void;
  onDone: () => void;
}

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
  /** Split view: the unedited render, and where the divider is (0–1 across). */
  before?: Preview | null;
  split?: number | null;
  onSplit?: (x: number) => void;
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
  /**
   * The part of the upright frame the preview shows (the crop). Positions in and out
   * of the viewer (lights, picks, the mask overlay) are always on the whole frame.
   */
  frame?: Crop | null;
  /** Actual pixel size of what the preview shows, for the zoom percentage. */
  pixelSize?: { w: number; h: number } | null;
  /** When set, the crop tool is open. */
  cropTool?: CropTool | null;
  /** When set, a hand-drawn mask is being edited on the image. */
  manualTool?: {
    shape: ManualShape;
    brush: BrushSettings;
    onChange: (shape: ManualShape) => void;
    onEnd: () => void;
  } | null;
}

export function Viewer({
  preview,
  busy,
  error,
  loading,
  showingBefore,
  before = null,
  split = null,
  onSplit,
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
  frame: view = null,
  pixelSize = null,
  cropTool = null,
  manualTool = null,
}: Props) {
  const imageRef = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const overlayCanvas = useRef<HTMLCanvasElement>(null);
  const beforeCanvas = useRef<HTMLCanvasElement>(null);
  const [cssSize, setCssSize] = useState({ w: 0, h: 0 });
  /** `null` = fit the frame; else a share of the actual pixels. */
  const [zoom, setZoom] = useState<number | null>(null);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [spaceHeld, setSpaceHeld] = useState(false);
  const [panning, setPanning] = useState(false);
  const panStart = useRef<{ x: number; y: number; pan: { x: number; y: number } } | null>(null);
  const dpr = window.devicePixelRatio || 1;

  useLayoutEffect(() => {
    const el = frame.current;
    if (!el) return;
    // Measure now, so the first preview doesn't wait for the observer's first callback
    // (which a hidden window may delay).
    const r = el.getBoundingClientRect();
    if (r.width > 0) setCssSize({ w: r.width, h: r.height });
    const ro = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setCssSize({ w: width, h: height });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Whole frame <-> what the preview shows (the crop).
  const toView = (x: number, y: number) =>
    view ? { x: (x - view.x) / view.width, y: (y - view.y) / view.height } : { x, y };
  const toFrame = (u: number, v: number) =>
    view ? { x: view.x + u * view.width, y: view.y + v * view.height } : { x: u, y: v };
  const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

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
    const c = beforeCanvas.current;
    if (!c || !before) return;
    if (c.width !== before.width || c.height !== before.height) {
      c.width = before.width;
      c.height = before.height;
    }
    c.getContext("2d")!.putImageData(new ImageData(before.rgba, before.width, before.height), 0, 0);
  }, [before]);
  const splitting = split !== null && before !== null && !showingBefore;

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

  // Fit the whole image to the frame, or zoom (a share of the actual pixels). The
  // engine renders close to the displayed device resolution, so scaling here is small.
  const px = pixelSize ?? (preview ? { w: preview.width, h: preview.height } : null);
  const aspect = preview ? preview.height / preview.width : px ? px.h / px.w : 1;
  const fitW = preview && cssSize.w > 0 ? Math.min(cssSize.w, cssSize.h / aspect) : 0;
  const fitZoom = px && fitW > 0 ? (fitW * dpr) / px.w : 1;
  const zooming = zoom !== null && !cropTool;
  const shownW = zooming && px ? (px.w * zoom) / dpr : fitW;
  const shownH = shownW * aspect;
  const maxPan = { x: Math.max(0, (shownW - cssSize.w) / 2), y: Math.max(0, (shownH - cssSize.h) / 2) };
  const shownPan = zooming
    ? { x: Math.min(maxPan.x, Math.max(-maxPan.x, pan.x)), y: Math.min(maxPan.y, Math.max(-maxPan.y, pan.y)) }
    : { x: 0, y: 0 };
  const style: React.CSSProperties =
    preview && cssSize.w > 0
      ? {
          width: shownW,
          height: shownH,
          flex: "none",
          transform: shownPan.x || shownPan.y ? `translate(${shownPan.x}px, ${shownPan.y}px)` : undefined,
        }
      : { display: "none" };
  const effectiveZoom = zooming ? zoom : fitZoom;

  // Ask for a render as sharp as what's shown: the frame, or the zoomed image.
  useEffect(() => {
    if (cssSize.w <= 0) return;
    const w = Math.floor(Math.max(cssSize.w, zooming ? shownW : 0) * dpr);
    const h = Math.floor(Math.max(cssSize.h, zooming ? shownH : 0) * dpr);
    if (!zooming) {
      onResize(Math.floor(cssSize.w * dpr), Math.floor(cssSize.h * dpr));
      return;
    }
    const t = window.setTimeout(() => onResize(w, h), ZOOM_RENDER_DELAY_MS);
    return () => window.clearTimeout(t);
  }, [cssSize.w, cssSize.h, zooming, shownW, shownH, dpr, onResize]);

  /** Zoom by `factor`, keeping the point under (cx, cy) in place. */
  const zoomAt = useCallback(
    (factor: number, cx?: number, cy?: number) => {
      if (!px || cropTool) return;
      const b = frame.current?.getBoundingClientRect();
      if (!b) return;
      const current = zoom ?? fitZoom;
      let next = Math.min(MAX_ZOOM, Math.max(Math.min(MIN_ZOOM, fitZoom), current * factor));
      if (Math.abs(next / fitZoom - 1) < 0.03) next = fitZoom;
      const centre = { x: b.left + b.width / 2, y: b.top + b.height / 2 };
      const at = { x: cx ?? centre.x, y: cy ?? centre.y };
      const imageCentre = { x: centre.x + shownPan.x, y: centre.y + shownPan.y };
      const k = next / current;
      setPan({
        x: at.x - (at.x - imageCentre.x) * k - centre.x,
        y: at.y - (at.y - imageCentre.y) * k - centre.y,
      });
      setZoom(next === fitZoom ? null : next);
    },
    [px, cropTool, zoom, fitZoom, shownPan.x, shownPan.y],
  );

  // Scroll (or pinch, which arrives as ctrl+wheel) to zoom. Native listener: React's
  // wheel events are passive and can't stop the page scrolling.
  useEffect(() => {
    const el = frame.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (cropTool || !preview) return;
      // Scrolling over a light changes its depth instead.
      if ((e.target as Element | null)?.closest(".viewer-light, .light-palette")) return;
      e.preventDefault();
      zoomAt(Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.002)), e.clientX, e.clientY);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [zoomAt, cropTool, preview]);

  // Space held: drag to pan.
  useEffect(() => {
    const typing = (t: EventTarget | null) =>
      t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName));
    const down = (e: KeyboardEvent) => {
      if (e.code !== "Space" || typing(e.target)) return;
      e.preventDefault();
      setSpaceHeld(true);
    };
    const up = (e: KeyboardEvent) => {
      if (e.code === "Space") setSpaceHeld(false);
    };
    const blur = () => setSpaceHeld(false);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", blur);
    };
  }, []);

  // A new photo opens fitted.
  const photoKey = pixelSize ? `${pixelSize.w}x${pixelSize.h}` : "";
  useEffect(() => {
    setZoom(null);
    setPan({ x: 0, y: 0 });
  }, [photoKey]);

  // Space always pans; a plain drag pans a zoomed image unless a tool uses the drag.
  const canPan = !cropTool && preview !== null && (spaceHeld || (zooming && !onPick && !manualTool));
  const panHandlers = {
    onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => {
      if (!(canPan && e.button === 0) && e.button !== 1) return;
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      panStart.current = { x: e.clientX, y: e.clientY, pan: shownPan };
      setPanning(true);
    },
    onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => {
      const s = panStart.current;
      if (!s || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
      setPan({ x: s.pan.x + e.clientX - s.x, y: s.pan.y + e.clientY - s.y });
    },
    onPointerUp: () => {
      panStart.current = null;
      setPanning(false);
    },
  };
  // A click that ended a pan isn't a pick.
  const moved = useRef(false);

  return (
    <div
      className={`viewer${canPan ? " can-pan" : ""}${panning ? " is-panning" : ""}${zooming ? " is-zoomed" : ""}`}
      ref={frame}
      {...panHandlers}
      onPointerDownCapture={() => {
        moved.current = false;
      }}
      onPointerMoveCapture={() => {
        if (panStart.current) moved.current = true;
      }}
    >
      <div
        ref={imageRef}
        className={`viewer-image${onPick && !canPan ? " is-picking" : ""}${cropTool ? " is-cropping" : ""}`}
        style={style}
        onClick={(e) => {
          if (!onPick || canPan || moved.current) return;
          const box = e.currentTarget.getBoundingClientRect();
          const p = toFrame(clamp01((e.clientX - box.left) / box.width), clamp01((e.clientY - box.top) / box.height));
          onPick(p.x, p.y);
        }}
      >
        {/* Split view: the unedited render under the edit, which is clipped to the right. */}
        <canvas ref={beforeCanvas} className="viewer-before" style={{ display: splitting ? "block" : "none" }} />
        <canvas ref={canvas} style={splitting ? { clipPath: `inset(0 0 0 ${split! * 100}%)` } : undefined} />
        {splitting && (
          <div
            className="viewer-split"
            style={{ left: `${split! * 100}%` }}
            role="slider"
            aria-label="Before / after divider"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(split! * 100)}
            tabIndex={0}
            onPointerDown={(e) => {
              e.stopPropagation();
              e.currentTarget.setPointerCapture(e.pointerId);
            }}
            onPointerMove={(e) => {
              if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
              const b = imageRef.current!.getBoundingClientRect();
              onSplit?.(Math.min(1, Math.max(0, (e.clientX - b.left) / b.width)));
            }}
            onClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              if (e.key === "ArrowLeft") onSplit?.(Math.max(0, split! - 0.02));
              if (e.key === "ArrowRight") onSplit?.(Math.min(1, split! + 0.02));
            }}
          >
            <span className="viewer-split-knob" aria-hidden>
              <svg viewBox="0 0 20 12" width="20" height="12">
                <path d="M7 2L3 6l4 4M13 2l4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </span>
            <span className="viewer-split-label is-before">Before</span>
            <span className="viewer-split-label is-after">After</span>
          </div>
        )}
        {/* Always mounted so hiding it (e.g. while showing Before) keeps the drawing. The
            overlay covers the whole frame; the clip shows the cropped part. */}
        <div className="viewer-overlay-clip" style={{ display: overlay && !showingBefore && !cropTool ? "block" : "none" }}>
          <canvas
            ref={overlayCanvas}
            className="viewer-overlay"
            style={
              view
                ? {
                    left: `${(-view.x / view.width) * 100}%`,
                    top: `${(-view.y / view.height) * 100}%`,
                    width: `${100 / view.width}%`,
                    height: `${100 / view.height}%`,
                  }
                : undefined
            }
          />
        </div>
        {manualTool && preview && !cropTool && !showingBefore && (
          <div className={`mask-tool-layer${spaceHeld ? " is-passive" : ""}`}>
            <ManualMaskTool
              shape={manualTool.shape}
              width={shownW}
              height={shownH}
              view={view}
              brush={manualTool.brush}
              onChange={manualTool.onChange}
              onEnd={manualTool.onEnd}
            />
          </div>
        )}
        {cropTool && preview && (
          <>
            <div className="crop-clip" aria-hidden>
              <div
                className="crop-shade"
                style={{
                  left: `${cropTool.crop.x * 100}%`,
                  top: `${cropTool.crop.y * 100}%`,
                  width: `${cropTool.crop.width * 100}%`,
                  height: `${cropTool.crop.height * 100}%`,
                }}
              />
            </div>
            <CropOverlay
              crop={cropTool.crop}
              frameAspect={preview.width / preview.height}
              rotation={cropTool.rotation}
              onChange={cropTool.onChange}
              onRotate={cropTool.onRotate}
              onEnd={cropTool.onEnd}
            />
          </>
        )}
        {!showingBefore &&
          !cropTool &&
          lights.map((l, i) => {
            const size = 26 - 14 * l.depth;
            const selected = i === selectedLight;
            // Reach as the engine models it: 5–65 % of the frame's long side.
            const box = imageRef.current?.getBoundingClientRect();
            const reachPx = box ? (0.05 + 0.6 * (l.reach / 100)) * Math.max(box.width, box.height) : 0;
            const at = toView(l.x, l.y);
            return (
              <div key={i} className="viewer-light-anchor" style={{ left: `${at.x * 100}%`, top: `${at.y * 100}%` }}>
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
                    const p = toFrame(clamp01((e.clientX - b.left) / b.width), clamp01((e.clientY - b.top) / b.height));
                    onMoveLight?.(i, p.x, p.y);
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
        {marker && !showingBefore && !cropTool && (
          <span
            className="viewer-marker"
            style={{ left: `${toView(marker.x, marker.y).x * 100}%`, top: `${toView(marker.x, marker.y).y * 100}%` }}
            title="Light source"
            aria-hidden
          />
        )}
      </div>
      {onPick && <div className="viewer-tag">Click the image to place the light</div>}
      {preview && onDropLight && !showingBefore && !cropTool && (
        <LightPalette
          onPrepare={onLightPrepare}
          onDrop={(kind, cx, cy) => {
            const b = imageRef.current?.getBoundingClientRect();
            if (!b || cx < b.left || cx > b.right || cy < b.top || cy > b.bottom) return;
            const p = toFrame((cx - b.left) / b.width, (cy - b.top) / b.height);
            onDropLight(kind, p.x, p.y);
          }}
        />
      )}
      {cropTool && preview && (
        <CropBar
          crop={cropTool.crop}
          frameAspect={preview.width / preview.height}
          rotation={cropTool.rotation}
          onChange={cropTool.onChange}
          onRotate={cropTool.onRotate}
          onEnd={cropTool.onEnd}
          onReset={cropTool.onReset}
          onDone={cropTool.onDone}
        />
      )}
      {preview && !cropTool && (
        <div className="viewer-zoom" onPointerDown={(e) => e.stopPropagation()}>
          <button type="button" className={zooming ? "" : "is-active"} onClick={() => (setZoom(null), setPan({ x: 0, y: 0 }))}>
            Fit
          </button>
          <button type="button" className={zooming && Math.abs(effectiveZoom - 1) < 1e-3 ? "is-active" : ""} onClick={() => zoomAt(1 / effectiveZoom)}>
            100%
          </button>
          <button type="button" aria-label="Zoom out" onClick={() => zoomAt(1 / 1.25)}>
            −
          </button>
          <output title="Scroll or pinch to zoom; hold Space and drag to pan">{Math.round(effectiveZoom * 100)}%</output>
          <button type="button" aria-label="Zoom in" onClick={() => zoomAt(1.25)}>
            +
          </button>
        </div>
      )}
      {showingBefore && preview && <div className="viewer-tag">Before</div>}
      {(loading || (busy && !preview)) && !error && <div className="viewer-status">Developing…</div>}
      {error && <div className="viewer-status is-error">{error}</div>}
    </div>
  );
}
