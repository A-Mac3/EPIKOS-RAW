import { curvePoints, isIdentity } from "../curve";
import type { CurveChannel, Curves } from "../types";

const SIZE = 180;
const STROKE: Record<CurveChannel, string> = {
  rgb: "#e4e4e4",
  red: "#ef6b6b",
  green: "#7cc88a",
  blue: "#6f9be0",
};

const path = (pts: [number, number][]) =>
  pts.map(([x, y], i) => `${i ? "L" : "M"}${(x * SIZE).toFixed(1)},${((1 - y) * SIZE).toFixed(1)}`).join("");

/** The selected channel's curve, with any other edited channels drawn faintly. */
export function CurveGraph({ curves, channel }: { curves: Curves; channel: CurveChannel }) {
  const others = (Object.keys(curves) as CurveChannel[]).filter((c) => c !== channel && !isIdentity(curves[c]));
  return (
    <svg className="curve-graph" viewBox={`0 0 ${SIZE} ${SIZE}`} width={SIZE} height={SIZE} role="img"
      aria-label={`${channel} tone curve`}>
      {/* Quarter grid: shadows, darks, lights, highlights. */}
      {[0.25, 0.5, 0.75].map((t) => (
        <g key={t} className="curve-grid">
          <line x1={t * SIZE} y1={0} x2={t * SIZE} y2={SIZE} />
          <line x1={0} y1={t * SIZE} x2={SIZE} y2={t * SIZE} />
        </g>
      ))}
      <line className="curve-diagonal" x1={0} y1={SIZE} x2={SIZE} y2={0} />
      {others.map((c) => (
        <path key={c} d={path(curvePoints(curves[c]))} stroke={STROKE[c]} className="curve-other" />
      ))}
      <path d={path(curvePoints(curves[channel]))} stroke={STROKE[channel]} className="curve-main" />
    </svg>
  );
}
