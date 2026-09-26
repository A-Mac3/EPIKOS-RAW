import { useRef, useState, type PointerEvent } from "react";
import type { StyleInfo, StyleWeight } from "../types";

const SIZE = 180;
const R = SIZE / 2 - 14;
/** Slot anchors on the rim: top, right, bottom, left (unit circle, y down). */
const ANCHORS: [number, number][] = [
  [0, -1],
  [1, 0],
  [0, 1],
  [-1, 0],
];

/** Weight of each slot for a puck at `p` (unit disc): 100 % at an anchor, 25 % each at the centre. */
export function fusionWeights(p: [number, number]): number[] {
  const raw = ANCHORS.map(([ax, ay]) => Math.max(0, 1 - Math.hypot(p[0] - ax, p[1] - ay) / Math.SQRT2) ** 2);
  const total = raw.reduce((a, b) => a + b, 0) || 1;
  return raw.map((w) => w / total);
}

/** Puck position that reproduces `weights` (their weighted anchor average). */
export function puckFor(weights: number[]): [number, number] {
  return ANCHORS.reduce<[number, number]>(
    ([x, y], [ax, ay], i) => [x + ax * (weights[i] ?? 0), y + ay * (weights[i] ?? 0)],
    [0, 0],
  );
}

interface Props {
  styles: StyleInfo[];
  /** Style id in each of the four slots. */
  slots: string[];
  onSlots: (slots: string[]) => void;
  blend: StyleWeight[];
  /** Live blend during a drag. */
  onBlend: (blend: StyleWeight[]) => void;
  onCommit: () => void;
}

/**
 * PRD Section 4 "AI Style Fusion Matrix": four styles on the rim of a wheel; the puck's
 * position blends them. Skin protection is applied to the blend automatically.
 */
export function FusionWheel({ styles, slots, onSlots, blend, onBlend, onCommit }: Props) {
  const ref = useRef<SVGSVGElement>(null);
  const weights = slots.map((id) => blend.find((b) => b.id === id)?.weight ?? 0);
  const [puck, setPuck] = useState<[number, number]>(() => puckFor(weights));
  const shown = blend.length > 0 ? weights : [0, 0, 0, 0];

  const fromPointer = (e: PointerEvent) => {
    const box = ref.current!.getBoundingClientRect();
    let x = (e.clientX - box.left - box.width / 2) / ((R * box.width) / SIZE);
    let y = (e.clientY - box.top - box.height / 2) / ((R * box.height) / SIZE);
    const m = Math.hypot(x, y);
    if (m > 1) [x, y] = [x / m, y / m];
    setPuck([x, y]);
    const w = fusionWeights([x, y]);
    onBlend(slots.map((id, i) => ({ id, weight: Math.round(w[i] * 1000) / 1000 })).filter((b) => b.weight > 0.005));
  };

  const style = (id: string) => styles.find((s) => s.id === id);
  const label = (i: number) => {
    const [ax, ay] = ANCHORS[i];
    return { x: SIZE / 2 + ax * (R + 2), y: SIZE / 2 + ay * (R + 2) };
  };

  return (
    <div className="fusion">
      <div className="fusion-slots">
        {slots.map((id, i) => (
          <label key={i} className="fusion-slot">
            <span className="swatch" style={{ background: swatch(style(id)) }} />
            <select
              value={id}
              aria-label={`Fusion slot ${i + 1}`}
              onChange={(e) => {
                const next = slots.slice();
                next[i] = e.currentTarget.value;
                onSlots(next);
              }}
            >
              {[...new Set(styles.map((s) => s.category))].map((cat) => (
                <optgroup key={cat} label={cat}>
                  {styles
                    .filter((s) => s.category === cat)
                    .map((s) => (
                      <option key={s.id} value={s.id}>
                        {s.name}
                      </option>
                    ))}
                </optgroup>
              ))}
            </select>
            <output>{Math.round(shown[i] * 100)}%</output>
          </label>
        ))}
      </div>
      <svg
        ref={ref}
        className="fusion-wheel"
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        width={SIZE}
        height={SIZE}
        role="slider"
        aria-label="Style fusion position"
        aria-valuetext={slots.map((id, i) => `${style(id)?.name ?? id} ${Math.round(shown[i] * 100)}%`).join(", ")}
        tabIndex={0}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          fromPointer(e);
        }}
        onPointerMove={(e) => {
          if (e.currentTarget.hasPointerCapture(e.pointerId)) fromPointer(e);
        }}
        onPointerUp={onCommit}
      >
        <defs>
          <filter id="fusion-glow" x="-100%" y="-100%" width="300%" height="300%">
            <feGaussianBlur stdDeviation="3" result="b" />
            <feMerge>
              <feMergeNode in="b" />
              <feMergeNode in="SourceGraphic" />
            </feMerge>
          </filter>
          <radialGradient id="fusion-vignette" cx="0.5" cy="0.5" r="0.5">
            <stop offset="0.55" stopColor="#000" stopOpacity="0" />
            <stop offset="1" stopColor="#000" stopOpacity="0.45" />
          </radialGradient>
          {slots.map((id, i) => (
            <radialGradient
              key={i}
              id={`fusion-g${i}`}
              cx={0.5 + ANCHORS[i][0] * 0.5}
              cy={0.5 + ANCHORS[i][1] * 0.5}
              r="0.75"
            >
              <stop offset="0" stopColor={style(id)?.swatch[1] ?? "#888"} stopOpacity="0.9" />
              <stop offset="1" stopColor={style(id)?.swatch[0] ?? "#444"} stopOpacity="0" />
            </radialGradient>
          ))}
        </defs>
        {/* Spectrum mesh: a conic hue ring softened towards a neutral centre, with each
            corner style's colour blooming from its anchor. */}
        <foreignObject x={SIZE / 2 - R} y={SIZE / 2 - R} width={R * 2} height={R * 2}>
          <div className="fusion-spectrum" />
        </foreignObject>
        {slots.map((_, i) => (
          <circle key={i} cx={SIZE / 2} cy={SIZE / 2} r={R} fill={`url(#fusion-g${i})`} className="fusion-bloom" />
        ))}
        <circle cx={SIZE / 2} cy={SIZE / 2} r={R} fill="url(#fusion-vignette)" />
        <circle cx={SIZE / 2} cy={SIZE / 2} r={R} className="fusion-rim" />
        <line x1={SIZE / 2 - R} y1={SIZE / 2} x2={SIZE / 2 + R} y2={SIZE / 2} className="fusion-axis" />
        <line x1={SIZE / 2} y1={SIZE / 2 - R} x2={SIZE / 2} y2={SIZE / 2 + R} className="fusion-axis" />
        {slots.map((id, i) => {
          const p = label(i);
          return <circle key={`a${i}`} cx={p.x} cy={p.y} r={5} className="fusion-anchor" fill={style(id)?.swatch[1]} />;
        })}
        {slots.map((_, i) => {
          // Live share of each corner, just inside its anchor.
          const [ax, ay] = ANCHORS[i];
          const x = SIZE / 2 + ax * (R - 20);
          const y = SIZE / 2 + ay * (R - 20) + 3;
          return (
            <text key={`p${i}`} x={x} y={y} className={`fusion-pct${shown[i] > 0.005 ? " is-on" : ""}`} textAnchor="middle">
              {Math.round(shown[i] * 100)}%
            </text>
          );
        })}
        {blend.length > 0 && (
          <g filter="url(#fusion-glow)">
            <circle cx={SIZE / 2 + puck[0] * R} cy={SIZE / 2 + puck[1] * R} r={9} className="fusion-puck-halo" />
            <circle cx={SIZE / 2 + puck[0] * R} cy={SIZE / 2 + puck[1] * R} r={6} className="fusion-puck" />
          </g>
        )}
      </svg>
    </div>
  );
}

function swatch(s?: StyleInfo) {
  return s ? `linear-gradient(135deg, ${s.swatch[0]}, ${s.swatch[1]})` : "#555";
}
