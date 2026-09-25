import { useRef, type PointerEvent } from "react";
import type { Mask, VirtualLight } from "../types";

const W = 260;
const H = 110;
const PAD = 10;

interface Props {
  depth: Mask | null;
  light: VirtualLight;
  /** Live move (x across the frame, depth 0 camera … 1 far). */
  onMove: (x: number, depth: number) => void;
  onCommit: () => void;
}

/**
 * Top-down view of the scene along the light's row: the depth profile of what the
 * camera sees (camera at the bottom, background at the top) and the light as a dot.
 * Drag the dot to move the light across and in depth: above the profile it sits
 * behind the subject (rim light), below it in front (key light).
 */
export function DepthSideView({ depth, light, onMove, onCommit }: Props) {
  const ref = useRef<SVGSVGElement>(null);
  const toX = (x: number) => PAD + x * (W - 2 * PAD);
  const toY = (z: number) => H - PAD - z * (H - 2 * PAD); // z: 0 camera (bottom) … 1 far (top)

  let profile = "";
  if (depth) {
    const row = Math.min(depth.height - 1, Math.max(0, Math.round(light.y * (depth.height - 1))));
    const n = 64;
    const pts: string[] = [];
    for (let i = 0; i <= n; i++) {
      const px = Math.round((i / n) * (depth.width - 1));
      const z = 1 - depth.alpha[row * depth.width + px] / 255;
      pts.push(`${toX(i / n).toFixed(1)},${toY(z).toFixed(1)}`);
    }
    profile = `M${toX(0)},${toY(1)} L${pts.join(" L")} L${toX(1)},${toY(1)} Z`;
  }

  const drag = (e: PointerEvent) => {
    const box = ref.current!.getBoundingClientRect();
    const sx = ((e.clientX - box.left) / box.width) * W;
    const sy = ((e.clientY - box.top) / box.height) * H;
    const x = Math.min(1, Math.max(0, (sx - PAD) / (W - 2 * PAD)));
    const z = Math.min(1, Math.max(0, (H - PAD - sy) / (H - 2 * PAD)));
    onMove(x, z);
  };

  return (
    <svg
      ref={ref}
      className="side-view"
      viewBox={`0 0 ${W} ${H}`}
      role="slider"
      aria-label="Light position in depth"
      aria-valuetext={`across ${Math.round(light.x * 100)}%, depth ${Math.round(light.depth * 100)}%`}
      tabIndex={0}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        drag(e);
      }}
      onPointerMove={(e) => {
        if (e.currentTarget.hasPointerCapture(e.pointerId)) drag(e);
      }}
      onPointerUp={onCommit}
    >
      <text x={W - PAD} y={PAD + 6} className="side-label" textAnchor="end">
        far
      </text>
      <text x={W - PAD} y={H - PAD - 2} className="side-label" textAnchor="end">
        camera
      </text>
      {profile ? (
        <path d={profile} className="side-scene" />
      ) : (
        <text x={W / 2} y={H / 2} className="side-label" textAnchor="middle">
          Estimating depth…
        </text>
      )}
      <line x1={toX(light.x)} y1={toY(0)} x2={toX(light.x)} y2={toY(light.depth)} className="side-ray" />
      <circle cx={toX(light.x)} cy={toY(light.depth)} r={7} className="side-light" />
    </svg>
  );
}
