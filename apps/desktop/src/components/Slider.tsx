import { useId, useState } from "react";
import { setSliderDragging } from "../sliderDrag";

interface Props {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  /** Value restored on double-click; on a bi-directional slider (e.g. 0 on −100…100)
   * the fill runs from it and a centre tick marks it. */
  defaultValue: number;
  format?: (v: number) => string;
  /** Map between slider position and value (e.g. logarithmic Kelvin). */
  toPosition?: (v: number) => number;
  fromPosition?: (p: number) => number;
  disabled?: boolean;
  onChange: (v: number) => void;
  /** Called when a drag or keyboard change ends: one undo step. */
  onCommit: () => void;
  /** Optional CSS gradient for the track (e.g. temperature blue → amber). */
  track?: string;
}

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

/**
 * Pro slider: a dark track with an amber fill trail (from the default for
 * bi-directional controls, with a centre-zero tick), a live value tooltip with its
 * unit while hovered or dragged, and double-click to reset. A native range input
 * underneath keeps keyboard and screen-reader behaviour.
 */
export function Slider({
  label,
  value,
  min,
  max,
  step,
  defaultValue,
  format = (v) => v.toFixed(2),
  toPosition = (v) => v,
  fromPosition = (p) => p,
  disabled,
  onChange,
  onCommit,
  track,
}: Props) {
  const id = useId();
  const [hover, setHover] = useState(false);
  const [active, setActive] = useState(false);
  const pos = toPosition(value);
  const frac = (v: number) => clamp01((v - min) / (max - min));
  const p = frac(pos);
  const zero = frac(toPosition(defaultValue));
  // Bi-directional: the range crosses zero (Exposure, Contrast…) or the track is a
  // colour scale around the as-shot value (Temperature, Tint).
  const bipolar = zero > 0.001 && zero < 0.999 && (min < 0 || !!track);
  const [from, to] = bipolar ? [Math.min(zero, p), Math.max(zero, p)] : [0, p];
  const changed = Math.abs(pos - toPosition(defaultValue)) > step / 2;
  const at = (f: number) => `calc(var(--knob) / 2 + ${f} * (100% - var(--knob)))`;
  const end = () => {
    setActive(false);
    setSliderDragging(false);
    onCommit();
  };
  return (
    <div className={`slider${disabled ? " is-disabled" : ""}${changed ? " is-changed" : ""}`}>
      <div className="slider-head">
        <label htmlFor={id}>{label}</label>
        <output htmlFor={id}>{format(value)}</output>
      </div>
      <div
        className={`pro-slider${active ? " is-active" : ""}`}
        onPointerEnter={() => setHover(true)}
        onPointerLeave={() => setHover(false)}
      >
        <span className="pro-track" style={track ? { background: track } : undefined}>
          {!track && <span className="pro-fill" style={{ left: `${from * 100}%`, width: `${(to - from) * 100}%` }} />}
        </span>
        {bipolar && <span className="pro-zero" style={{ left: at(zero) }} aria-hidden />}
        <span className="pro-thumb" style={{ left: at(p) }} aria-hidden />
        {(hover || active) && !disabled && (
          <span className="pro-tip" style={{ left: at(p) }} aria-hidden>
            {format(value)}
          </span>
        )}
        <input
          id={id}
          type="range"
          className="pro-input"
          min={min}
          max={max}
          step={step}
          value={pos}
          disabled={disabled}
          onChange={(e) => onChange(fromPosition(Number(e.currentTarget.value)))}
          onPointerDown={() => {
            setActive(true);
            setSliderDragging(true);
          }}
          onPointerUp={end}
          onKeyUp={onCommit}
          onBlur={() => {
            setActive(false);
            setSliderDragging(false);
            onCommit();
          }}
          onDoubleClick={() => {
            onChange(defaultValue);
            onCommit();
          }}
          title="Double-click to reset"
        />
      </div>
    </div>
  );
}

interface ToggleProps {
  label: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}

export function Toggle({ label, checked, disabled, onChange }: ToggleProps) {
  return (
    <label className={`toggle${disabled ? " is-disabled" : ""}`}>
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.currentTarget.checked)}
      />
      <span className="toggle-track" aria-hidden />
      <span>{label}</span>
    </label>
  );
}

interface SegmentedProps<T extends string> {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}

export function Segmented<T extends string>({ label, value, options, onChange }: SegmentedProps<T>) {
  return (
    <div className="segmented" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          className={o.value === value ? "is-active" : ""}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
