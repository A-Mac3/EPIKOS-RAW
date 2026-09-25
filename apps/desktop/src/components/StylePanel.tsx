import { useEffect, useRef, useState } from "react";
import type { Adjustments, Preview, StyleInfo, StyleWeight } from "../types";
import { FusionWheel } from "./FusionWheel";
import { LookPromptBox } from "./LookPromptBox";
import { Slider } from "./Slider";

type Update = (fn: (a: Adjustments) => Adjustments) => void;

interface Props {
  path: string;
  styles: StyleInfo[];
  thumbs: Record<string, Preview>;
  adjustments: Adjustments;
  edit: Update;
  endEdit: () => void;
  commit: Update;
}

/** Parametric Style Engine: one-click styles layered over the Step 4–5 settings. */
export function StylePanel({ path, styles, thumbs, adjustments, edit, endEdit, commit }: Props) {
  const [open, setOpen] = useState(true);
  const style = adjustments.style;
  const fused = style.blend.length > 0;
  const active = fused ? null : (styles.find((s) => s.id === style.id) ?? null);
  const [fusionOpen, setFusionOpen] = useState(fused);
  const [slots, setSlots] = useState<string[]>(() => {
    const fromBlend = style.blend.map((b) => b.id);
    const rest = styles.map((s) => s.id).filter((id) => !fromBlend.includes(id));
    return [...fromBlend, ...rest].slice(0, 4);
  });
  useEffect(() => {
    // Styles arrive after mount; fill empty slots once they do.
    if (slots.length < 4 && styles.length >= 4) {
      setSlots((s) => [...s, ...styles.map((x) => x.id).filter((id) => !s.includes(id))].slice(0, 4));
    }
  }, [styles, slots.length]);
  // Skin protection of a blend: the styles' own defaults, by weight.
  const protectionFor = (blend: StyleWeight[]) => {
    const total = blend.reduce((a, b) => a + b.weight, 0) || 1;
    const sum = blend.reduce((a, b) => a + (styles.find((s) => s.id === b.id)?.skinProtection ?? 0) * b.weight, 0);
    return Math.round(sum / total);
  };
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
        {fused && <span className="badge is-on">Fusion</span>}
        <span className="chevron" aria-hidden>
          {open ? "▾" : "▸"}
        </span>
      </button>
      {open && (
        <div className="step-body">
          <LookPromptBox path={path} adjustments={adjustments} commit={commit} />
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
                    x.style.id === s.id && x.style.blend.length === 0
                      ? { ...x, style: { id: "", amount: 100, skinProtection: 0, blend: [] } }
                      : { ...x, style: { id: s.id, amount: 100, skinProtection: s.skinProtection, blend: [] } },
                  )
                }
              >
                <Thumb preview={thumbs[s.id]} swatch={s.swatch} />
                <span className="style-name">{s.name}</span>
              </button>
            ))}
          </div>
          <button
            type="button"
            className={`btn fusion-toggle${fusionOpen ? " is-active" : ""}`}
            aria-expanded={fusionOpen}
            onClick={() => setFusionOpen((o) => !o)}
          >
            Style Fusion Matrix {fusionOpen ? "▾" : "▸"}
          </button>
          {fusionOpen && (
            <>
              <FusionWheel
                styles={styles}
                slots={slots}
                onSlots={(next) => {
                  setSlots(next);
                  // Keep the current weights on the new slot choices.
                  if (fused) {
                    const w = slots.map((id) => style.blend.find((b) => b.id === id)?.weight ?? 0);
                    const blend = next.map((id, i) => ({ id, weight: w[i] })).filter((b) => b.weight > 0);
                    commit((x) => ({ ...x, style: { ...x.style, id: "", blend, skinProtection: protectionFor(blend) } }));
                  }
                }}
                blend={style.blend}
                onBlend={(blend) =>
                  edit((x) => ({
                    ...x,
                    style: { id: "", amount: x.style.amount || 100, blend, skinProtection: protectionFor(blend) },
                  }))
                }
                onCommit={endEdit}
              />
              <p className="note">
                Drag the puck to blend up to four styles; each corner is 100% of that style, the centre an even mix.
                Skin protection follows the blend.
              </p>
            </>
          )}
          {active || fused ? (
            <>
              <p className="hint">
                {active
                  ? active.description
                  : style.blend
                      .map((b) => `${styles.find((s) => s.id === b.id)?.name ?? b.id} ${Math.round(b.weight * 100)}%`)
                      .join(" · ")}
              </p>
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
                defaultValue={active ? active.skinProtection : protectionFor(style.blend)}
                format={(v) => `${v.toFixed(0)}%`}
                onChange={(v) => edit(setStyle({ skinProtection: v }))}
                onCommit={endEdit}
              />
              <p className="note">
                Skin is detected automatically and shielded from the style&apos;s scene grade. The style layers on top
                of your Step 4–8 settings, which stay editable.{" "}
                {fused ? "" : "Click the card again to remove it."}
              </p>
              {fused && (
                <button
                  type="button"
                  className="btn"
                  onClick={() => commit((x) => ({ ...x, style: { id: "", amount: 100, skinProtection: 0, blend: [] } }))}
                >
                  Remove fusion
                </button>
              )}
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
