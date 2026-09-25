import { useRef, type KeyboardEvent, type PointerEvent } from "react";
import type { ColorWheel as Wheel } from "../types";

interface Props {
  label: string;
  value: Wheel;
  /** Live change while dragging. */
  onChange: (v: Pick<Wheel, "hue" | "amount">) => void;
  /** End of a drag or key press: one undo step. */
  onCommit: () => void;
}

const SIZE = 84;

/** Conic hue ring starting at 3 o'clock, hue increasing counter-clockwise (as the puck maps it). */
const RING = `conic-gradient(from 90deg, ${Array.from({ length: 13 }, (_, i) => {
  const cw = i * 30;
  return `hsl(${(360 - cw) % 360} 85% 55%) ${cw}deg`;
}).join(", ")})`;

/**
 * Three-way colour wheel: the puck's angle is the tint's hue, its distance from the
 * centre the amount. Double-click resets; arrow keys nudge (←/→ hue, ↑/↓ amount).
 */
export function ColorWheel({ label, value, onChange, onCommit }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const r = SIZE / 2;
  const rad = (value.hue * Math.PI) / 180;
  const dist = (value.amount / 100) * (r - 6);
  const puck = { left: r + dist * Math.cos(rad), top: r - dist * Math.sin(rad) };

  const fromPointer = (e: PointerEvent) => {
    const box = ref.current!.getBoundingClientRect();
    const dx = e.clientX - (box.left + box.width / 2);
    const dy = -(e.clientY - (box.top + box.height / 2));
    const hue = ((Math.atan2(dy, dx) * 180) / Math.PI + 360) % 360;
    const amount = Math.min(100, (Math.hypot(dx, dy) / (box.width / 2 - 6)) * 100);
    onChange({ hue: Math.round(hue), amount: Math.round(amount) });
  };

  const onKey = (e: KeyboardEvent) => {
    const step = e.shiftKey ? 10 : 2;
    const next = { hue: value.hue, amount: value.amount };
    if (e.key === "ArrowLeft") next.hue = (value.hue + step) % 360;
    else if (e.key === "ArrowRight") next.hue = (value.hue - step + 360) % 360;
    else if (e.key === "ArrowUp") next.amount = Math.min(100, value.amount + step);
    else if (e.key === "ArrowDown") next.amount = Math.max(0, value.amount - step);
    else return;
    e.preventDefault();
    onChange(next);
  };

  // A neutral wheel shows its tint colour dimmed, so the hue is still readable.
  const tint = `hsl(${value.hue} 85% 55%)`;
  return (
    <div className="wheel">
      <div
        ref={ref}
        className="wheel-disc"
        style={{ width: SIZE, height: SIZE, background: RING }}
        role="slider"
        tabIndex={0}
        aria-label={`${label} tint`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(value.amount)}
        aria-valuetext={`hue ${Math.round(value.hue)}°, amount ${Math.round(value.amount)}`}
        title="Drag to tint · double-click to reset"
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          fromPointer(e);
        }}
        onPointerMove={(e) => {
          if (e.currentTarget.hasPointerCapture(e.pointerId)) fromPointer(e);
        }}
        onPointerUp={onCommit}
        onKeyDown={onKey}
        onKeyUp={onCommit}
        onDoubleClick={() => {
          onChange({ hue: value.hue, amount: 0 });
          onCommit();
        }}
      >
        <span className="wheel-puck" style={{ ...puck, background: value.amount > 0 ? tint : undefined }} />
      </div>
      <div className="wheel-label">
        <span>{label}</span>
        <output>{value.amount > 0 ? `${Math.round(value.hue)}° · ${Math.round(value.amount)}` : "—"}</output>
      </div>
    </div>
  );
}
