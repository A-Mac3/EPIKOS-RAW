import { useId } from "react";

interface Props {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  /** Value restored on double-click. */
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
  const pos = toPosition(value);
  const fill = ((pos - min) / (max - min)) * 100;
  return (
    <div className={`slider${disabled ? " is-disabled" : ""}`}>
      <div className="slider-head">
        <label htmlFor={id}>{label}</label>
        <output htmlFor={id}>{format(value)}</output>
      </div>
      <input
        id={id}
        type="range"
        min={min}
        max={max}
        step={step}
        value={pos}
        disabled={disabled}
        style={{
          ["--fill" as string]: `${fill}%`,
          ...(track ? { ["--track" as string]: track } : {}),
        }}
        onChange={(e) => onChange(fromPosition(Number(e.currentTarget.value)))}
        onPointerUp={onCommit}
        onKeyUp={onCommit}
        onBlur={onCommit}
        onDoubleClick={() => {
          onChange(defaultValue);
          onCommit();
        }}
        title="Double-click to reset"
      />
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
