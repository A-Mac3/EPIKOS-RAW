import { useEffect, useRef, useState } from "react";
import { importLut, removeLut } from "../api";
import { STYLE_CATEGORIES, type Adjustments, type LutInfo, type Preview, type StyleInfo, type StyleWeight } from "../types";
import { FusionWheel } from "./FusionWheel";
import { Slider } from "./Slider";

type Update = (fn: (a: Adjustments) => Adjustments) => void;

interface Props {
  styles: StyleInfo[];
  thumbs: Record<string, Preview>;
  adjustments: Adjustments;
  edit: Update;
  endEdit: () => void;
  commit: Update;
  /** Hover preview: settings to show while the pointer rests on a card, or null. */
  onPreview: (adjustments: Adjustments | null) => void;
  luts: LutInfo[];
  /** Imported LUTs changed. */
  onLutsChanged: () => void;
}

const SLOTS_KEY = "epikos.fusion.slots";
const HOVER_DELAY_MS = 180;
const NO_STYLE = { id: "", amount: 100, skinProtection: 0, blend: [] };

function storedSlots(): string[] | null {
  try {
    const v = JSON.parse(localStorage.getItem(SLOTS_KEY) ?? "null");
    return Array.isArray(v) && v.length === 4 && v.every((x) => typeof x === "string") ? v : null;
  } catch {
    return null;
  }
}

/**
 * Presets & Styles: the parametric style library by category, the Style Fusion Matrix
 * and imported 3D LUTs. Resting the pointer on a card previews it on the photo.
 */
export function StylePanel({ styles, thumbs, adjustments, edit, endEdit, commit, onPreview, luts, onLutsChanged }: Props) {
  const style = adjustments.style;
  const fused = style.blend.length > 0;
  const listed = styles.filter((s) => s.listed);
  const active = fused ? null : (styles.find((s) => s.id === style.id) ?? null);
  const [openCats, setOpenCats] = useState<Set<string>>(() => new Set([active?.category ?? "Portraits & Skin"]));
  const [fusionOpen, setFusionOpen] = useState(fused);
  const [lutError, setLutError] = useState<string | null>(null);
  const [slots, setSlots] = useState<string[]>(() => {
    const fromBlend = style.blend.map((b) => b.id);
    return fromBlend.length > 0 ? fromBlend : (storedSlots() ?? []);
  });
  useEffect(() => {
    // Styles arrive after mount; fill empty slots once they do.
    if (slots.length < 4 && listed.length >= 4) {
      const defaults = ["dark-melanin-glow", "portra-400", "teal-orange", "golden-hour-flare"];
      setSlots((s) => [...s, ...[...defaults, ...listed.map((x) => x.id)].filter((id) => !s.includes(id))].slice(0, 4));
    }
  }, [listed.length, slots.length]);
  const hoverTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(hoverTimer.current), []);

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
  const hover = (next: ((a: Adjustments) => Adjustments) | null) => {
    window.clearTimeout(hoverTimer.current);
    if (!next) {
      onPreview(null);
      return;
    }
    hoverTimer.current = window.setTimeout(() => onPreview(next(adjustments)), HOVER_DELAY_MS);
  };
  const applyStyle = (s: StyleInfo) => (x: Adjustments) =>
    x.style.id === s.id && x.style.blend.length === 0
      ? { ...x, style: NO_STYLE }
      : { ...x, style: { id: s.id, amount: 100, skinProtection: s.skinProtection, blend: [] } };
  const toggleCat = (c: string) =>
    setOpenCats((o) => {
      const n = new Set(o);
      if (n.has(c)) n.delete(c);
      else n.add(c);
      return n;
    });
  const lutMissing = adjustments.lut.name && !luts.some((l) => l.name === adjustments.lut.name);

  return (
    <section className="styles library">
      {STYLE_CATEGORIES.map((cat) => {
        const inCat = listed.filter((s) => s.category === cat);
        const isOpen = openCats.has(cat);
        const activeHere = inCat.find((s) => s.id === style.id && !fused);
        return (
          <div key={cat} className="library-group">
            <button type="button" className="library-head" aria-expanded={isOpen} onClick={() => toggleCat(cat)}>
              <span>{cat}</span>
              {activeHere && <span className="badge is-on">{activeHere.name}</span>}
              <span className="library-count">{inCat.length}</span>
              <span className="chevron" aria-hidden>
                {isOpen ? "▾" : "▸"}
              </span>
            </button>
            {isOpen && (
              <div className="style-grid" role="radiogroup" aria-label={cat}>
                {inCat.map((s) => (
                  <button
                    key={s.id}
                    type="button"
                    role="radio"
                    aria-checked={s.id === style.id && !fused}
                    className={`style-card${s.id === style.id && !fused ? " is-active" : ""}`}
                    title={`${s.description}\n\nClick to apply; rest the pointer here to preview.`}
                    onPointerEnter={() => hover(applyStyle(s))}
                    onPointerLeave={() => hover(null)}
                    onClick={() => {
                      hover(null);
                      commit(applyStyle(s));
                    }}
                  >
                    <Thumb preview={thumbs[s.id]} swatch={s.swatch} />
                    <span className="style-name">{s.name}</span>
                  </button>
                ))}
              </div>
            )}
          </div>
        );
      })}

      <div className="library-group">
        <button type="button" className="library-head" aria-expanded={openCats.has("luts")} onClick={() => toggleCat("luts")}>
          <span>My LUTs</span>
          {adjustments.lut.name && <span className="badge is-on">{adjustments.lut.name}</span>}
          <span className="library-count">{luts.length}</span>
          <span className="chevron" aria-hidden>
            {openCats.has("luts") ? "▾" : "▸"}
          </span>
        </button>
        {openCats.has("luts") && (
          <div className="lut-list">
            {luts.map((l) => {
              const on = adjustments.lut.name === l.name;
              return (
                <div key={l.name} className={`lut-row${on ? " is-active" : ""}`}>
                  <button
                    type="button"
                    className="lut-apply"
                    title={`${l.title ?? l.name} · ${l.size}³\n\nClick to apply; rest the pointer here to preview.`}
                    onPointerEnter={() => hover((x) => ({ ...x, lut: { name: l.name, amount: 100 } }))}
                    onPointerLeave={() => hover(null)}
                    onClick={() => {
                      hover(null);
                      commit((x) => ({ ...x, lut: on ? { name: "", amount: 100 } : { name: l.name, amount: 100 } }));
                    }}
                  >
                    <span className="lut-name">{l.name}</span>
                    <span className="hint">{l.size}³</span>
                  </button>
                  <button
                    type="button"
                    className="btn icon"
                    aria-label={`Remove ${l.name}`}
                    title="Remove this LUT from EPIKOS RAW (the original .cube file is kept)"
                    onClick={async () => {
                      try {
                        await removeLut(l.name);
                        if (on) commit((x) => ({ ...x, lut: { name: "", amount: 100 } }));
                        onLutsChanged();
                      } catch (e) {
                        setLutError(String(e));
                      }
                    }}
                  >
                    ×
                  </button>
                </div>
              );
            })}
            <button
              type="button"
              className="btn"
              onClick={async () => {
                setLutError(null);
                try {
                  const lut = await importLut();
                  if (lut) {
                    onLutsChanged();
                    commit((x) => ({ ...x, lut: { name: lut.name, amount: 100 } }));
                  }
                } catch (e) {
                  setLutError(String(e));
                }
              }}
            >
              Import .cube LUT…
            </button>
            {lutError && <p className="hint error">{lutError}</p>}
            {lutMissing && (
              <p className="hint error">
                This photo uses the LUT “{adjustments.lut.name}”, which isn&apos;t imported on this computer.
              </p>
            )}
            {adjustments.lut.name && (
              <Slider
                label="LUT amount"
                value={adjustments.lut.amount}
                min={0}
                max={100}
                step={1}
                defaultValue={100}
                format={(v) => `${v.toFixed(0)}%`}
                onChange={(v) => edit((x) => ({ ...x, lut: { ...x.lut, amount: v } }))}
                onCommit={endEdit}
              />
            )}
            <p className="note">
              3D LUTs (.cube) apply after the curves, on the image as the screen shows it. Imported LUTs are copied into
              EPIKOS RAW, so edits keep working if the original moves.
            </p>
          </div>
        )}
      </div>

      <button
        type="button"
        className={`btn fusion-toggle${fusionOpen ? " is-active" : ""}`}
        aria-expanded={fusionOpen}
        onClick={() => setFusionOpen((o) => !o)}
      >
        Style Fusion Matrix {fusionOpen ? "▾" : "▸"}
      </button>
      {fusionOpen && slots.length === 4 && (
        <>
          <FusionWheel
            styles={listed}
            slots={slots}
            onSlots={(next) => {
              setSlots(next);
              try {
                localStorage.setItem(SLOTS_KEY, JSON.stringify(next));
              } catch {
                // Remembering the corners is a convenience only.
              }
              // Keep the current weights on the new corner choices.
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
            Choose any style for each of the four corners, then drag the puck to blend them: a corner is 100% of that
            style, the centre an even mix. Skin protection follows the blend.
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
            Skin is detected automatically and shielded from the style&apos;s scene grade. The style layers on top of your
            Step 4–8 settings, which stay editable. {fused ? "" : "Click the card again to remove it."}
          </p>
          {fused && (
            <button type="button" className="btn" onClick={() => commit((x) => ({ ...x, style: NO_STYLE }))}>
              Remove fusion
            </button>
          )}
        </>
      ) : (
        <p className="note">Rest the pointer on a style to preview it; click to apply. Each adapts to this photo and protects skin tones.</p>
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
