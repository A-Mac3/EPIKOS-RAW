import { useRef, useState, type PointerEvent } from "react";
import type { BrushStroke, Crop, ManualShape } from "../types";

/** Brush settings for new strokes: size (share of the frame width), feather, flow. */
export interface BrushSettings {
  size: number;
  feather: number;
  flow: number;
  erase: boolean;
}

interface Props {
  shape: ManualShape;
  /** The displayed image's size in CSS px. */
  width: number;
  height: number;
  /** The part of the upright frame on screen (the crop), or null for all of it. */
  view: Crop | null;
  brush: BrushSettings;
  /** Live change during a drag or stroke. */
  onChange: (shape: ManualShape) => void;
  /** End of a drag or stroke: one undo step. */
  onEnd: () => void;
}

const OVERLAY = "rgb(255 48 64)";
type Drag = { kind: "p0" | "p1" | "line" | "center" | "rx" | "ry"; start: [number, number]; shape: ManualShape } | null;

/**
 * On-image handles for a hand-drawn mask: a graduated (linear) filter with its start
 * and end lines, a radial ellipse with centre, size / rotation handles and its
 * feather, or a brush that paints (or erases) strokes. Positions are kept on the whole
 * upright frame and mapped through the crop for display.
 */
export function ManualMaskTool({ shape, width, height, view, brush, onChange, onEnd }: Props) {
  const svg = useRef<SVGSVGElement>(null);
  const drag = useRef<Drag>(null);
  const stroke = useRef<BrushStroke | null>(null);
  const [cursor, setCursor] = useState<[number, number] | null>(null);

  const v = view ?? { x: 0, y: 0, width: 1, height: 1 };
  // Frame fraction ↔ overlay px.
  const px = (x: number, y: number): [number, number] => [((x - v.x) / v.width) * width, ((y - v.y) / v.height) * height];
  const frame = (e: PointerEvent): [number, number] => {
    const b = svg.current!.getBoundingClientRect();
    return [v.x + ((e.clientX - b.left) / b.width) * v.width, v.y + ((e.clientY - b.top) / b.height) * v.height];
  };
  // Frame widths / heights in overlay px.
  const fw = width / v.width;
  const fh = height / v.height;

  const begin = (kind: NonNullable<Drag>["kind"]) => (e: PointerEvent) => {
    e.stopPropagation();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    drag.current = { kind, start: frame(e), shape };
  };
  const move = (e: PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const [x, y] = frame(e);
    const [dx, dy] = [x - d.start[0], y - d.start[1]];
    const s = d.shape;
    if (s.kind === "linear") {
      if (d.kind === "p0") onChange({ ...s, x0: x, y0: y });
      else if (d.kind === "p1") onChange({ ...s, x1: x, y1: y });
      else if (d.kind === "line") onChange({ ...s, x0: s.x0 + dx, y0: s.y0 + dy, x1: s.x1 + dx, y1: s.y1 + dy });
    } else if (s.kind === "radial") {
      if (d.kind === "center") onChange({ ...s, cx: s.cx + dx, cy: s.cy + dy });
      else {
        // In px so the ellipse keeps its shape on any aspect ratio.
        const [ox, oy] = [(x - s.cx) * fw, (y - s.cy) * fh];
        const dist = Math.hypot(ox, oy);
        if (d.kind === "rx") {
          onChange({ ...s, rx: Math.max(4, dist) / fw, angle: (Math.atan2(oy, ox) * 180) / Math.PI });
        } else {
          onChange({ ...s, ry: Math.max(4, dist) / fh });
        }
      }
    }
  };
  const end = () => {
    if (drag.current) onEnd();
    drag.current = null;
  };

  // Brush painting.
  const paintDown = (e: PointerEvent<SVGSVGElement>) => {
    if (shape.kind !== "brush" || e.button !== 0) return;
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    stroke.current = { points: [frame(e)], size: brush.size, feather: brush.feather, flow: brush.flow, erase: brush.erase };
    onChange({ ...shape, strokes: [...shape.strokes, stroke.current] });
  };
  const paintMove = (e: PointerEvent<SVGSVGElement>) => {
    if (shape.kind !== "brush") return;
    const p = frame(e);
    setCursor(p);
    const s = stroke.current;
    if (!s) return;
    const last = s.points[s.points.length - 1];
    // Enough points for a smooth path without thousands per stroke.
    if (Math.hypot((p[0] - last[0]) * fw, (p[1] - last[1]) * fh) < 2) return;
    stroke.current = { ...s, points: [...s.points, p] };
    onChange({ ...shape, strokes: [...shape.strokes.slice(0, -1), stroke.current] });
  };
  const paintUp = () => {
    if (stroke.current) onEnd();
    stroke.current = null;
  };

  const handle = (x: number, y: number, kind: NonNullable<Drag>["kind"], title: string, big = false) => {
    const [cx, cy] = px(x, y);
    return (
      <circle
        cx={cx}
        cy={cy}
        r={big ? 7 : 5.5}
        className={`mask-handle${big ? " is-center" : ""}`}
        onPointerDown={begin(kind)}
        onPointerMove={move}
        onPointerUp={end}
        onPointerCancel={end}
      >
        <title>{title}</title>
      </circle>
    );
  };

  let body: React.ReactNode = null;
  if (shape.kind === "linear") {
    const [ax, ay] = px(shape.x0, shape.y0);
    const [bx, by] = px(shape.x1, shape.y1);
    // Long lines through each end, perpendicular to the gradient.
    const [dx, dy] = [bx - ax, by - ay];
    const len = Math.hypot(dx, dy) || 1;
    const [nx, ny] = [(-dy / len) * 4000, (dx / len) * 4000];
    body = (
      <>
        <defs>
          <linearGradient id="mask-linear" gradientUnits="userSpaceOnUse" x1={ax} y1={ay} x2={bx} y2={by}>
            <stop offset="0" stopColor={OVERLAY} stopOpacity="0.38" />
            <stop offset="1" stopColor={OVERLAY} stopOpacity="0" />
          </linearGradient>
        </defs>
        <rect x={0} y={0} width={width} height={height} fill="url(#mask-linear)" pointerEvents="none" />
        <line x1={ax - nx} y1={ay - ny} x2={ax + nx} y2={ay + ny} className="mask-line is-full" />
        <line x1={bx - nx} y1={by - ny} x2={bx + nx} y2={by + ny} className="mask-line" />
        <line
          x1={ax}
          y1={ay}
          x2={bx}
          y2={by}
          className="mask-axis"
          onPointerDown={begin("line")}
          onPointerMove={move}
          onPointerUp={end}
        />
        {handle(shape.x0, shape.y0, "p0", "Full effect: drag to move")}
        {handle(shape.x1, shape.y1, "p1", "No effect: drag to set the transition")}
      </>
    );
  } else if (shape.kind === "radial") {
    const [cx, cy] = px(shape.cx, shape.cy);
    const [rxp, ryp] = [shape.rx * fw, shape.ry * fh];
    const inner = Math.max(0.01, 1 - shape.feather / 100);
    const a = (shape.angle * Math.PI) / 180;
    const rxHandle = [shape.cx + (Math.cos(a) * rxp) / fw, shape.cy + (Math.sin(a) * rxp) / fh] as const;
    const ryHandle = [shape.cx + (-Math.sin(a) * ryp) / fw, shape.cy + (Math.cos(a) * ryp) / fh] as const;
    body = (
      <>
        <defs>
          <radialGradient id="mask-radial">
            <stop offset={inner} stopColor={OVERLAY} stopOpacity="0.38" />
            <stop offset="1" stopColor={OVERLAY} stopOpacity="0" />
          </radialGradient>
          {/* Inverted: the overlay everywhere except the ellipse (black hides in a mask). */}
          <radialGradient id="mask-radial-hole">
            <stop offset={inner} stopColor="#000" />
            <stop offset="1" stopColor="#fff" />
          </radialGradient>
          <mask id="mask-invert">
            <rect x={0} y={0} width={width} height={height} fill="#fff" />
            <ellipse cx={cx} cy={cy} rx={rxp} ry={ryp} fill="url(#mask-radial-hole)" transform={`rotate(${shape.angle} ${cx} ${cy})`} />
          </mask>
        </defs>
        {shape.invert ? (
          <rect x={0} y={0} width={width} height={height} fill={OVERLAY} fillOpacity={0.38} mask="url(#mask-invert)" pointerEvents="none" />
        ) : (
          <ellipse
            cx={cx}
            cy={cy}
            rx={rxp}
            ry={ryp}
            fill="url(#mask-radial)"
            transform={`rotate(${shape.angle} ${cx} ${cy})`}
            pointerEvents="none"
          />
        )}
        <g transform={`rotate(${shape.angle} ${cx} ${cy})`}>
          <ellipse cx={cx} cy={cy} rx={rxp} ry={ryp} className="mask-ellipse" pointerEvents="none" />
          <ellipse cx={cx} cy={cy} rx={rxp * inner} ry={ryp * inner} className="mask-ellipse is-inner" pointerEvents="none" />
        </g>
        {handle(shape.cx, shape.cy, "center", "Drag to move", true)}
        {handle(rxHandle[0], rxHandle[1], "rx", "Drag to resize and rotate")}
        {handle(ryHandle[0], ryHandle[1], "ry", "Drag to resize")}
      </>
    );
  } else {
    body = (
      <>
        {shape.strokes.map((s, i) => (
          <polyline
            key={i}
            points={s.points.map(([x, y]) => px(x, y).join(",")).join(" ")}
            className={`mask-stroke${s.erase ? " is-erase" : ""}`}
            strokeWidth={Math.max(1, s.size * fw)}
            strokeOpacity={s.erase ? 0.5 : 0.25 + 0.2 * (s.flow / 100)}
          />
        ))}
        {cursor && (
          <>
            <circle cx={px(...cursor)[0]} cy={px(...cursor)[1]} r={(brush.size * fw) / 2} className="brush-cursor" />
            <circle
              cx={px(...cursor)[0]}
              cy={px(...cursor)[1]}
              r={((brush.size * fw) / 2) * (1 - brush.feather / 100)}
              className="brush-cursor is-inner"
            />
          </>
        )}
      </>
    );
  }

  return (
    <svg
      ref={svg}
      className={`mask-tool is-${shape.kind}${brush.erase ? " is-erasing" : ""}`}
      viewBox={`0 0 ${width} ${height}`}
      width={width}
      height={height}
      onPointerDown={paintDown}
      onPointerMove={paintMove}
      onPointerUp={paintUp}
      onPointerCancel={paintUp}
      onPointerLeave={() => setCursor(null)}
      onClick={(e) => e.stopPropagation()}
    >
      {body}
    </svg>
  );
}
