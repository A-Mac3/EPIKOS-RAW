import { useEffect, useRef, useState } from "react";
import type { Adjustments, Preview, StyleInfo } from "../types";
import { Slider } from "./Slider";

type Update = (fn: (a: Adjustments) => Adjustments) => void;

interface Props {
  styles: StyleInfo[];
  thumbs: Record<string, Preview>;
  adjustments: Adjustments;
  edit: Update;
  endEdit: () => void;
  commit: Update;
}

/** Parametric Style Engine: one-click styles layered over the Step 4–5 settings. */
export function StylePanel({ styles, thumbs, adjustments, edit, endEdit, commit }: Props) {
  const [open, setOpen] = useState(true);
  const active = styles.find((s) => s.id === adjustments.style.id) ?? null;
  const style = adjustments.style;
  const setStyle = (patch: Partial<Adjustments["style"]>) => (x: Adjustments) => ({
    ...x,
    style: { ...x.style, ...patch },
  });

  return (
    <section className="styles">
      <button type="button" className="step-head" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span className="styles-icon" aria-hidden>
          ◐
        </span>
        <span className="step-title">Styles</span>
        {active && <span className="badge is-on">{active.name}</span>}
        <span className="chevron" aria-hidden>
          {open ? "▾" : "▸"}
        </span>
      </button>
      {open && (
        <div className="step-body">
          <div className="style-grid" role="radiogroup" aria-label="Style">
            {styles.map((s) => (
              <button
                key={s.id}
                type="button"
                role="radio"
                aria-checked={s.id === style.id}
                className={`style-card${s.id === style.id ? " is-active" : ""}`}
                title={`${s.world}\n\n${s.description}`}
                onClick={() =>
                  commit((x) =>
                    x.style.id === s.id
                      ? { ...x, style: { id: "", amount: 100, skinProtection: 0 } }
                      : { ...x, style: { id: s.id, amount: 100, skinProtection: s.skinProtection } },
                  )
                }
              >
                <Thumb preview={thumbs[s.id]} swatch={s.swatch} />
                <span className="style-name">{s.name}</span>
              </button>
            ))}
          </div>
          {active ? (
            <>
              <p className="hint">{active.description}</p>
              <Slider
                label="Amount"
                value={style.amount}
                min={0}
                max={100}
                step={1}
                defaultValue={100}
                format={(v) => `${v.toFixed(0)}%`}
                onChange={(v) => edit(setStyle({ amount: v }))}
                onCommit={endEdit}
              />
              <Slider
                label="Skin protection"
                value={style.skinProtection}
                min={0}
                max={100}
                step={1}
                defaultValue={active.skinProtection}
                format={(v) => `${v.toFixed(0)}%`}
                onChange={(v) => edit(setStyle({ skinProtection: v }))}
                onCommit={endEdit}
              />
              <p className="note">
                Skin is detected automatically and shielded from the style&apos;s scene grade. The style layers on top
                of your Step 4–5 settings, which stay editable. Click the card again to remove it.
              </p>
            </>
          ) : (
            <p className="note">Click a style to apply it. Each one adapts to this photo and protects skin tones.</p>
          )}
        </div>
      )}
    </section>
  );
}

function Thumb({ preview, swatch }: { preview?: Preview; swatch: [string, string] }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c || !preview) return;
    c.width = preview.width;
    c.height = preview.height;
    c.getContext("2d")!.putImageData(new ImageData(preview.rgba, preview.width, preview.height), 0, 0);
  }, [preview]);
  return (
    <span className="style-thumb" style={{ background: `linear-gradient(135deg, ${swatch[0]}, ${swatch[1]})` }}>
      <canvas ref={ref} style={{ display: preview ? undefined : "none" }} />
    </span>
  );
}
