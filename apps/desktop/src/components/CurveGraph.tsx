import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { curvePoints, isIdentity } from "../curve";
import type { CurveChannel, Curves } from "../types";

const SIZE = 200;
const PAD = 6;
const INNER = SIZE - 2 * PAD;
const STROKE: Record<CurveChannel, string> = {
  rgb: "#e4e4e4",
  red: "#ef6b6b",
  green: "#7cc88a",
  blue: "#6f9be0",
};
/** Dragging this far outside the graph removes a point. */
const REMOVE_MARGIN = 0.12;

type Pt = [number, number];

const toSvg = ([x, y]: Pt) => [PAD + x * INNER, PAD + (1 - y) * INNER] as const;
const path = (pts: Pt[]) =>
  pts
    .map((p, i) => {
      const [x, y] = toSvg(p);
      return `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join("");

interface Props {
  curves: Curves;
  channel: CurveChannel;
  /** Preview histograms (R, G, B; 256 bins), drawn faintly behind the curve. */
  histogram?: [Uint32Array, Uint32Array, Uint32Array] | null;
  /** Live change of the channel's control points during a drag. */
  onPoints: (points: Pt[]) => void;
  /** End of a drag / key press: one undo step. */
  onCommit: () => void;
}

/**
 * Step 7 free-form point curve editor over the full curve (parametric + points).
 * Click to add a point, drag to move, double-click or drag off the graph to remove;
 * arrow keys nudge the selected point. The corners are points too.
 */
export function CurveGraph({ curves, channel, histogram, onPoints, onCommit }: Props) {
  const svg = useRef<SVGSVGElement>(null);
  const [active, setActive] = useState<number | null>(null);
  const dragging = useRef<number | null>(null);
  const curve = curves[channel];
  // Without a point curve, show the implied corners as handles.
  const points: Pt[] = curve.points.length > 0 ? curve.points : [[0, 0], [1, 1]];
  const others = (Object.keys(curves) as CurveChannel[]).filter((c) => c !== channel && !isIdentity(curves[c]));

  const toCurve = (e: PointerEvent): Pt => {
    const box = svg.current!.getBoundingClientRect();
    const s = box.width / SIZE;
    return [((e.clientX - box.left) / s - PAD) / INNER, 1 - ((e.clientY - box.top) / s - PAD) / INNER];
  };

  /** Move point `i` to `p`, kept between its neighbours; far outside removes it. */
  const move = (list: Pt[], i: number, [x, y]: Pt): { list: Pt[]; index: number | null } => {
    const interior = i > 0 && i < list.length - 1;
    if (interior && (y < -REMOVE_MARGIN || y > 1 + REMOVE_MARGIN)) {
      return { list: list.filter((_, k) => k !== i), index: null };
    }
    const lo = i > 0 ? list[i - 1][0] + 0.01 : 0;
    const hi = i < list.length - 1 ? list[i + 1][0] - 0.01 : 1;
    const next = list.slice();
    next[i] = [Math.min(hi, Math.max(lo, x)), Math.min(1, Math.max(0, y))];
    return { list: next, index: i };
  };

  const onPointerDown = (e: PointerEvent<SVGSVGElement>) => {
    const [x, y] = toCurve(e);
    // Nearest handle within reach, else add a point on the curve at this x.
    const reach = 10 / INNER;
    let hit = points.findIndex(([px, py]) => Math.hypot(px - x, py - y) < reach);
    let list = points;
    if (hit < 0) {
      const onCurve = curvePoints(curve, 200).reduce((best, p) => (Math.abs(p[0] - x) < Math.abs(best[0] - x) ? p : best));
      const nx = Math.min(0.99, Math.max(0.01, x));
      list = [...points, [nx, Math.abs(onCurve[1] - y) < reach * 2 ? onCurve[1] : y] as Pt].sort((a, b) => a[0] - b[0]);
      hit = list.findIndex((p) => p[0] === nx);
      onPoints(list);
    }
    dragging.current = hit;
    setActive(hit);
    e.currentTarget.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent<SVGSVGElement>) => {
    const i = dragging.current;
    if (i === null) return;
    const { list, index } = move(points, i, toCurve(e));
    dragging.current = index;
    setActive(index);
    onPoints(list);
  };

  const onPointerUp = () => {
    if (dragging.current === null && active === null) return;
    dragging.current = null;
    onCommit();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (active === null || active >= points.length) return;
    const step = e.shiftKey ? 0.02 : 0.005;
    const [x, y] = points[active];
    const delta: Record<string, Pt> = {
      ArrowUp: [x, y + step],
      ArrowDown: [x, y - step],
      ArrowLeft: [x - step, y],
      ArrowRight: [x + step, y],
    };
    if (e.key in delta) {
      e.preventDefault();
      onPoints(move(points, active, delta[e.key]).list);
    } else if ((e.key === "Delete" || e.key === "Backspace") && active > 0 && active < points.length - 1) {
      e.preventDefault();
      onPoints(points.filter((_, k) => k !== active));
      setActive(null);
      onCommit();
    }
  };

  return (
    <svg
      ref={svg}
      className="curve-graph"
      viewBox={`0 0 ${SIZE} ${SIZE}`}
      role="application"
      tabIndex={0}
      aria-label={`${channel} tone curve. Click to add a point, drag to move, double-click to remove.`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onKeyDown={onKeyDown}
      onKeyUp={(e) => e.key.startsWith("Arrow") && onCommit()}
    >
      {histogram && <path d={histogramPath(histogram, channel)} className={`curve-histogram is-${channel}`} />}
      {[0.25, 0.5, 0.75].map((t) => (
        <g key={t} className="curve-grid">
          <line x1={PAD + t * INNER} y1={PAD} x2={PAD + t * INNER} y2={PAD + INNER} />
          <line x1={PAD} y1={PAD + t * INNER} x2={PAD + INNER} y2={PAD + t * INNER} />
        </g>
      ))}
      <line className="curve-diagonal" x1={PAD} y1={PAD + INNER} x2={PAD + INNER} y2={PAD} />
      {others.map((c) => (
        <path key={c} d={path(curvePoints(curves[c]))} stroke={STROKE[c]} className="curve-other" />
      ))}
      <path d={path(curvePoints(curve))} stroke={STROKE[channel]} className="curve-main" />
      {points.map((p, i) => {
        const [cx, cy] = toSvg(p);
        return (
          <circle
            key={i}
            cx={cx}
            cy={cy}
            r={i === active ? 5 : 4}
            className={`curve-handle${i === active ? " is-active" : ""}`}
            onDoubleClick={(e) => {
              e.stopPropagation();
              if (i > 0 && i < points.length - 1) {
                onPoints(points.filter((_, k) => k !== i));
                setActive(null);
                onCommit();
              }
            }}
          />
        );
      })}
    </svg>
  );
}

/** Filled histogram of the channel (all three summed for RGB), square-root scaled so
 * shadows and highlights stay visible next to a tall midtone peak. */
function histogramPath(h: [Uint32Array, Uint32Array, Uint32Array], channel: CurveChannel): string {
  const idx = { rgb: -1, red: 0, green: 1, blue: 2 }[channel];
  const bins = Array.from({ length: 256 }, (_, i) => (idx < 0 ? h[0][i] + h[1][i] + h[2][i] : h[idx][i]));
  // Ignore the clipped end bins when scaling, or one spike flattens the rest.
  const peak = Math.sqrt(Math.max(1, ...bins.slice(1, 255)));
  const pts = bins.map((v, i) => {
    const [x, y] = toSvg([i / 255, Math.min(1, Math.sqrt(v) / peak) * 0.9]);
    return `L${x.toFixed(1)},${y.toFixed(1)}`;
  });
  const [x0, y0] = toSvg([0, 0]);
  const [x1] = toSvg([1, 0]);
  return `M${x0},${y0}${pts.join("")}L${x1},${y0}Z`;
}
