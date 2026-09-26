import { newManual, type Adjustments, type ManualAdjustment, type ManualShape } from "../types";
import type { BrushSettings } from "./ManualMaskTool";
import { Slider, Toggle } from "./Slider";

type Update = (fn: (a: Adjustments) => Adjustments) => void;

interface Props {
  manual: ManualAdjustment[];
  edit: Update;
  endEdit: () => void;
  commit: Update;
  /** The mask being edited on the image. */
  active: number | null;
  setActive: (i: number | null) => void;
  brush: BrushSettings;
  setBrush: (b: BrushSettings) => void;
}

const KINDS: { kind: ManualShape["kind"]; label: string; hint: string }[] = [
  { kind: "brush", label: "Brush", hint: "Paint where the edit applies" },
  { kind: "linear", label: "Linear gradient", hint: "Sky or ground: fades from one line to another" },
  { kind: "radial", label: "Radial gradient", hint: "An ellipse, soft-edged; invert to edit around it" },
];

const EDITS: [keyof Omit<ManualAdjustment, "shape">, string, number, number, number, (v: number) => string][] = [
  ["exposure", "Exposure", -3, 3, 0.01, (v) => `${v >= 0 ? "+" : ""}${v.toFixed(2)} EV`],
  ["contrast", "Contrast", -100, 100, 1, (v) => `${v >= 0 ? "+" : ""}${v.toFixed(0)}`],
  ["warmth", "Temperature", -100, 100, 1, (v) => `${v >= 0 ? "+" : ""}${v.toFixed(0)}`],
  ["tint", "Tint", -100, 100, 1, (v) => `${v >= 0 ? "+" : ""}${v.toFixed(0)}`],
  ["dehaze", "Dehaze", -100, 100, 1, (v) => `${v >= 0 ? "+" : ""}${v.toFixed(0)}`],
  ["saturation", "Saturation", -100, 100, 1, (v) => `${v >= 0 ? "+" : ""}${v.toFixed(0)}`],
  ["clarity", "Clarity", -100, 100, 1, (v) => `${v >= 0 ? "+" : ""}${v.toFixed(0)}`],
];

const TRACK: Partial<Record<string, string>> = {
  warmth: "linear-gradient(90deg, #3d6fd6, #d8d8d8 50%, #e6a73a)",
  tint: "linear-gradient(90deg, #4caf50, #d8d8d8 50%, #d04fb0)",
};

const name = (m: ManualAdjustment, i: number) =>
  `${m.shape.kind === "brush" ? "Brush" : m.shape.kind === "linear" ? "Linear gradient" : "Radial gradient"} ${i + 1}`;

/**
 * Step 3 Manual Masking: brush, linear and radial gradient masks drawn on the image,
 * each with its own exposure, contrast, temperature, tint, dehaze, saturation and
 * clarity, applied alongside the AI masks.
 */
export function ManualMaskPanel({ manual, edit, endEdit, commit, active, setActive, brush, setBrush }: Props) {
  const set = (i: number, patch: Partial<ManualAdjustment>) => (x: Adjustments) => ({
    ...x,
    manual: x.manual.map((m, k) => (k === i ? { ...m, ...patch } : m)),
  });
  const setShape = (i: number, patch: Partial<ManualShape>) => (x: Adjustments) => ({
    ...x,
    manual: x.manual.map((m, k) => (k === i ? { ...m, shape: { ...m.shape, ...patch } as ManualShape } : m)),
  });
  const current = active !== null ? manual[active] : undefined;

  return (
    <div className="manual-masks">
      <div className="manual-add" role="group" aria-label="Add a mask">
        {KINDS.map((k) => (
          <button
            key={k.kind}
            type="button"
            className="btn"
            title={k.hint}
            onClick={() => {
              commit((x) => ({ ...x, manual: [...x.manual, newManual(k.kind)] }));
              setActive(manual.length);
            }}
          >
            + {k.label}
          </button>
        ))}
      </div>
      {manual.length > 0 && (
        <div className="manual-list" role="radiogroup" aria-label="Hand-drawn masks">
          {manual.map((m, i) => (
            <button
              key={i}
              type="button"
              role="radio"
              aria-checked={active === i}
              className={`manual-item${active === i ? " is-active" : ""}`}
              onClick={() => setActive(active === i ? null : i)}
            >
              <span className={`manual-icon is-${m.shape.kind}`} aria-hidden />
              {name(m, i)}
            </button>
          ))}
        </div>
      )}
      {current && active !== null ? (
        <div className="local-card">
          <p className="hint">
            {current.shape.kind === "brush"
              ? "Paint on the image. Hold Space to pan."
              : current.shape.kind === "linear"
                ? "Drag the dots: full effect at the solid line, none at the dashed one. Drag the middle to move."
                : "Drag the centre to move, the side dots to resize and rotate."}
          </p>
          {current.shape.kind === "brush" && (
            <>
              <Slider
                label="Brush size"
                value={brush.size * 100}
                min={0.5}
                max={30}
                step={0.1}
                defaultValue={5}
                format={(v) => `${v.toFixed(1)}%`}
                onChange={(v) => setBrush({ ...brush, size: v / 100 })}
                onCommit={() => {}}
              />
              <Slider
                label="Feather"
                value={brush.feather}
                min={0}
                max={100}
                step={1}
                defaultValue={50}
                format={(v) => `${v.toFixed(0)}%`}
                onChange={(v) => setBrush({ ...brush, feather: v })}
                onCommit={() => {}}
              />
              <Slider
                label="Flow"
                value={brush.flow}
                min={1}
                max={100}
                step={1}
                defaultValue={100}
                format={(v) => `${v.toFixed(0)}%`}
                onChange={(v) => setBrush({ ...brush, flow: v })}
                onCommit={() => {}}
              />
              <div className="field-row">
                <Toggle label="Erase" checked={brush.erase} onChange={(erase) => setBrush({ ...brush, erase })} />
                <button
                  type="button"
                  className="btn small"
                  disabled={current.shape.strokes.length === 0}
                  onClick={() => commit(setShape(active, { strokes: [] }))}
                >
                  Clear strokes
                </button>
              </div>
            </>
          )}
          {current.shape.kind === "radial" && (
            <>
              <Slider
                label="Feather"
                value={current.shape.feather}
                min={0}
                max={100}
                step={1}
                defaultValue={50}
                format={(v) => `${v.toFixed(0)}%`}
                onChange={(v) => edit(setShape(active, { feather: v }))}
                onCommit={endEdit}
              />
              <Toggle
                label="Invert (edit outside)"
                checked={current.shape.invert}
                onChange={(invert) => commit(setShape(active, { invert }))}
              />
            </>
          )}
          {EDITS.map(([key, label, min, max, step, format]) => (
            <Slider
              key={key}
              label={label}
              value={current[key]}
              min={min}
              max={max}
              step={step}
              defaultValue={0}
              format={format}
              track={TRACK[key]}
              onChange={(v) => edit(set(active, { [key]: v }))}
              onCommit={endEdit}
            />
          ))}
          <button
            type="button"
            className="btn small"
            onClick={() => {
              const i = active;
              setActive(null);
              commit((x) => ({ ...x, manual: x.manual.filter((_, k) => k !== i) }));
            }}
          >
            Remove {name(current, active)}
          </button>
        </div>
      ) : (
        <p className="note">
          Add a brush or gradient, then shape it on the image. Hand-drawn masks work alongside the AI masks; both follow
          the crop.
        </p>
      )}
    </div>
  );
}
