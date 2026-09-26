import { useRef } from "react";
import type { Crop, CropAspect } from "../types";

/** Width / height of each preset; `original` is the frame's own. */
export const ASPECTS: { value: CropAspect; label: string }[] = [
  { value: "free", label: "Free" },
  { value: "original", label: "Original" },
  { value: "1:1", label: "1:1" },
  { value: "4:5", label: "4:5" },
  { value: "16:9", label: "16:9" },
  { value: "9:16", label: "9:16" },
];

const MIN_SIZE = 0.05;
const MAX_ANGLE = 45;

/** Pixel width / height of `aspect` on a frame of pixel aspect `frame` (w / h). */
export function aspectRatio(aspect: CropAspect, frame: number): number | null {
  if (aspect === "free") return null;
  if (aspect === "original") return frame;
  const [w, h] = aspect.split(":").map(Number);
  return w / h;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** The largest rectangle of `ratio` (pixel w / h) inside `c`, centred on it. */
export function fitAspect(c: Crop, ratio: number | null, frame: number): Crop {
  if (ratio === null) return c;
  // Normalised width / height for this pixel ratio.
  const k = ratio / frame;
  let width = c.width;
  let height = width / k;
  if (height > c.height) {
    height = c.height;
    width = height * k;
  }
  if (width > 1) [width, height] = [1, 1 / k];
  if (height > 1) [width, height] = [k, 1];
  const cx = c.x + c.width / 2;
  const cy = c.y + c.height / 2;
  return {
    ...c,
    width,
    height,
    x: clamp(cx - width / 2, 0, 1 - width),
    y: clamp(cy - height / 2, 0, 1 - height),
  };
}

type Handle = "move" | "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";
const HANDLES: Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

/** Resize `c0` by dragging `h` by (dx, dy) (fractions of the frame). */
function drag(c0: Crop, h: Handle, dx: number, dy: number, ratio: number | null, frame: number): Crop {
  if (h === "move") {
    return { ...c0, x: clamp(c0.x + dx, 0, 1 - c0.width), y: clamp(c0.y + dy, 0, 1 - c0.height) };
  }
  let [l, t, r, b] = [c0.x, c0.y, c0.x + c0.width, c0.y + c0.height];
  if (h.includes("w")) l = clamp(l + dx, 0, r - MIN_SIZE);
  if (h.includes("e")) r = clamp(r + dx, l + MIN_SIZE, 1);
  if (h.includes("n")) t = clamp(t + dy, 0, b - MIN_SIZE);
  if (h.includes("s")) b = clamp(b + dy, t + MIN_SIZE, 1);
  if (ratio === null) return { ...c0, x: l, y: t, width: r - l, height: b - t };

  // Locked aspect: the dragged side sets the size, anchored at the opposite side (or
  // centred across it, for an edge), and shrinks to stay inside the frame.
  const k = ratio / frame;
  const horizontal = h === "e" || h === "w";
  const vertical = h === "n" || h === "s";
  let width = r - l;
  let height = b - t;
  if (vertical) width = height * k;
  else if (horizontal) height = width / k;
  else if (width / height > k) height = width / k;
  else width = height * k;
  const ax = h.includes("w") ? c0.x + c0.width : h.includes("e") ? c0.x : c0.x + c0.width / 2;
  const ay = h.includes("n") ? c0.y + c0.height : h.includes("s") ? c0.y : c0.y + c0.height / 2;
  // Room from the anchor towards where the rectangle grows.
  const roomX = h.includes("w") ? ax : h.includes("e") ? 1 - ax : 2 * Math.min(ax, 1 - ax);
  const roomY = h.includes("n") ? ay : h.includes("s") ? 1 - ay : 2 * Math.min(ay, 1 - ay);
  const scale = Math.min(1, roomX / width, roomY / height);
  width *= scale;
  height *= scale;
  const x = h.includes("w") ? ax - width : h.includes("e") ? ax : ax - width / 2;
  const y = h.includes("n") ? ay - height : h.includes("s") ? ay : ay - height / 2;
  return { ...c0, x, y, width, height };
}

interface Props {
  crop: Crop;
  /** Pixel width / height of the (uncropped, straightened) frame shown. */
  frameAspect: number;
  rotation: number;
  onChange: (c: Crop) => void;
  onRotate: (degrees: number) => void;
  /** End of a drag: one undo step. */
  onEnd: () => void;
}

/**
 * Crop overlay on the whole upright frame: drag inside to move, handles to resize
 * (locked to the aspect preset), the knob above to straighten. Rule-of-thirds grid.
 */
export function CropOverlay({ crop, frameAspect, rotation, onChange, onRotate, onEnd }: Props) {
  const box = useRef<HTMLDivElement>(null);
  const start = useRef<{ handle: Handle; x: number; y: number; crop: Crop; angle: number; rotation: number } | null>(
    null,
  );
  const ratio = aspectRatio(crop.aspect, frameAspect);

  const begin = (handle: Handle) => (e: React.PointerEvent) => {
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    start.current = { handle, x: e.clientX, y: e.clientY, crop, angle: 0, rotation };
  };
  const move = (e: React.PointerEvent) => {
    const s = start.current;
    const b = box.current?.parentElement?.getBoundingClientRect();
    if (!s || !b || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
    onChange(drag(s.crop, s.handle, (e.clientX - s.x) / b.width, (e.clientY - s.y) / b.height, ratio, frameAspect));
  };
  const end = () => {
    if (start.current) onEnd();
    start.current = null;
  };

  // Straighten knob: the angle swept around the crop's centre.
  const centre = () => {
    const b = box.current!.getBoundingClientRect();
    return { x: b.left + b.width / 2, y: b.top + b.height / 2 };
  };
  const angleAt = (e: React.PointerEvent) => {
    const c = centre();
    return (Math.atan2(e.clientY - c.y, e.clientX - c.x) * 180) / Math.PI;
  };

  return (
    <div
      ref={box}
      className="crop-box"
      style={{
        left: `${crop.x * 100}%`,
        top: `${crop.y * 100}%`,
        width: `${crop.width * 100}%`,
        height: `${crop.height * 100}%`,
      }}
      onPointerDown={begin("move")}
      onPointerMove={move}
      onPointerUp={end}
      onPointerCancel={end}
      onClick={(e) => e.stopPropagation()}
    >
      <span className="crop-grid" aria-hidden />
      {HANDLES.map((h) => (
        <span
          key={h}
          className={`crop-handle is-${h}`}
          onPointerDown={begin(h)}
          onPointerMove={move}
          onPointerUp={end}
          onPointerCancel={end}
          aria-hidden
        />
      ))}
      <span
        className="crop-rotate"
        title={`Straighten: drag around the crop (${rotation >= 0 ? "+" : ""}${rotation.toFixed(1)}°)`}
        onPointerDown={(e) => {
          e.stopPropagation();
          e.currentTarget.setPointerCapture(e.pointerId);
          start.current = { handle: "move", x: 0, y: 0, crop, angle: angleAt(e), rotation };
        }}
        onPointerMove={(e) => {
          const s = start.current;
          if (!s || !e.currentTarget.hasPointerCapture(e.pointerId)) return;
          let d = angleAt(e) - s.angle;
          if (d > 180) d -= 360;
          if (d < -180) d += 360;
          // Dragging clockwise turns the picture clockwise; + is counter-clockwise.
          onRotate(Math.round(clamp(s.rotation - d, -MAX_ANGLE, MAX_ANGLE) * 20) / 20);
        }}
        onPointerUp={end}
        onPointerCancel={end}
      />
    </div>
  );
}

/** Aspect presets, straighten angle and reset / done, under the image while cropping. */
export function CropBar({
  crop,
  frameAspect,
  rotation,
  onChange,
  onRotate,
  onEnd,
  onReset,
  onDone,
}: Props & { onReset: () => void; onDone: () => void }) {
  return (
    <div className="crop-bar" onPointerDown={(e) => e.stopPropagation()}>
      <div className="crop-aspects" role="radiogroup" aria-label="Aspect ratio">
        {ASPECTS.map((a) => (
          <button
            key={a.value}
            type="button"
            role="radio"
            aria-checked={crop.aspect === a.value}
            className={`crop-aspect${crop.aspect === a.value ? " is-active" : ""}`}
            onClick={() => {
              onChange(fitAspect({ ...crop, aspect: a.value }, aspectRatio(a.value, frameAspect), frameAspect));
              onEnd();
            }}
          >
            {a.label}
          </button>
        ))}
      </div>
      <label className="crop-angle">
        <span>Angle</span>
        <input
          type="range"
          min={-MAX_ANGLE}
          max={MAX_ANGLE}
          step={0.05}
          value={rotation}
          onChange={(e) => onRotate(Number(e.currentTarget.value))}
          onPointerUp={onEnd}
          onKeyUp={onEnd}
          aria-label="Straighten angle"
        />
        <output>{`${rotation >= 0 ? "+" : ""}${rotation.toFixed(1)}°`}</output>
      </label>
      <button type="button" className="btn" onClick={onReset}>
        Reset
      </button>
      <button type="button" className="btn primary" onClick={onDone}>
        Done
      </button>
    </div>
  );
}
