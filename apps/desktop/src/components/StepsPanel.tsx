import { useState, type ReactNode } from "react";
import type { Adjustments, DemosaicMode, ImageInfo, WbMode } from "../types";
import { Segmented, Slider, Toggle } from "./Slider";

type Update = (fn: (a: Adjustments) => Adjustments) => void;

interface Props {
  info: ImageInfo;
  adjustments: Adjustments;
  /** Live update during a drag. */
  edit: Update;
  /** End of a drag: closes the undo step. */
  endEdit: () => void;
  /** Discrete change: one undo step. */
  commit: Update;
}

/** PRD Section 5: the mandatory, displayed order of operations. */
const STEPS: { title: string; planned: string }[] = [
  { title: "RAW Input & Optical Calibration", planned: "Lens profile database, auto-geometry, sensor noise reduction" },
  { title: "Global Exposure & Dynamic Range", planned: "Shadow lift, auto exposure" },
  { title: "AI Subject & Semantic Masking", planned: "Subject, background, skin, eyes, hair, foreground and sky masks" },
  { title: "Micro-Texture & Retouching", planned: "Blemish smoothing, specular balancing, character line sculpting" },
  { title: "Base Color Grading & HSL", planned: "Skin tone protection, foliage shift, background re-coloration" },
  { title: "Atmospheric & Light Sculpting", planned: "Volumetric light shafts, localized glow, depth-based fog and haze" },
  { title: "Creative Split-Toning & Curves", planned: "Highlight warmth, shadow cooling, black point and matte" },
  { title: "Final Finishing & Handoff", planned: "Analog grain, vignette, PSD / DNG export. 16-bit TIFF export is available now (Export… / ⌘E)" },
];

// Temperature slider is logarithmic so the useful 2 000–10 000 K range gets most travel.
const K_MIN = 2000;
const K_MAX = 25000;
const kToPos = (k: number) => Math.log(k / K_MIN) / Math.log(K_MAX / K_MIN);
const posToK = (p: number) => Math.round(K_MIN * Math.pow(K_MAX / K_MIN, p));

const fmtSigned = (digits: number, unit = "") => (v: number) =>
  `${v > 0 ? "+" : ""}${v.toFixed(digits)}${unit}`;

export function StepsPanel({ info, adjustments: a, edit, endEdit, commit }: Props) {
  const [open, setOpen] = useState<Set<number>>(() => new Set([0, 1]));
  const toggle = (i: number) =>
    setOpen((s) => {
      const next = new Set(s);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });

  const wb = a.whiteBalance;
  const asShot = info.asShot ?? { temperature: 5500, tint: 0 };
  // In As Shot mode the sliders show the camera's value; touching them switches to Custom.
  const shownWb = wb.mode === "asShot" ? asShot : wb;
  const editWb = (patch: Partial<{ temperature: number; tint: number }>) =>
    edit((x) => ({
      ...x,
      whiteBalance: {
        mode: "custom",
        temperature: x.whiteBalance.mode === "asShot" ? asShot.temperature : x.whiteBalance.temperature,
        tint: x.whiteBalance.mode === "asShot" ? asShot.tint : x.whiteBalance.tint,
        ...patch,
      },
    }));

  const ca = a.lens.chromaticAberration;
  const dist = a.lens.distortion;
  const setLens = (fn: (l: Adjustments["lens"]) => Adjustments["lens"]): ((x: Adjustments) => Adjustments) =>
    (x) => ({ ...x, lens: fn(x.lens) });

  const content: Record<number, ReactNode> = {
    0: (
      <>
        <Field label="Demosaic" hint="Applies at full resolution; the preview uses a fast superpixel develop.">
          <select
            value={a.demosaic}
            onChange={(e) => commit((x) => ({ ...x, demosaic: e.currentTarget.value as DemosaicMode }))}
          >
            <option value="auto">Auto</option>
            <option value="malvar">Malvar–He–Cutler (Bayer)</option>
            <option value="xtrans">Markesteijn (X-Trans)</option>
            <option value="bilinear">Bilinear</option>
          </select>
        </Field>
        <Toggle
          label="Chromatic aberration"
          checked={ca.enabled}
          onChange={(v) => commit(setLens((l) => ({ ...l, chromaticAberration: { ...l.chromaticAberration, enabled: v } })))}
        />
        <Slider
          label="Red / cyan"
          value={ca.red}
          min={-0.01}
          max={0.01}
          step={0.0001}
          defaultValue={0}
          format={fmtSigned(4)}
          disabled={!ca.enabled}
          onChange={(v) => edit(setLens((l) => ({ ...l, chromaticAberration: { ...l.chromaticAberration, red: v } })))}
          onCommit={endEdit}
        />
        <Slider
          label="Blue / yellow"
          value={ca.blue}
          min={-0.01}
          max={0.01}
          step={0.0001}
          defaultValue={0}
          format={fmtSigned(4)}
          disabled={!ca.enabled}
          onChange={(v) => edit(setLens((l) => ({ ...l, chromaticAberration: { ...l.chromaticAberration, blue: v } })))}
          onCommit={endEdit}
        />
        <Toggle
          label="Lens distortion"
          checked={dist.enabled}
          onChange={(v) => commit(setLens((l) => ({ ...l, distortion: { ...l.distortion, enabled: v } })))}
        />
        <Slider
          label="Barrel / pincushion (k1)"
          value={dist.k1}
          min={-0.5}
          max={0.5}
          step={0.001}
          defaultValue={0}
          format={fmtSigned(3)}
          disabled={!dist.enabled}
          onChange={(v) => edit(setLens((l) => ({ ...l, distortion: { ...l.distortion, k1: v } })))}
          onCommit={endEdit}
        />
        <Slider
          label="Edge (k2)"
          value={dist.k2}
          min={-0.5}
          max={0.5}
          step={0.001}
          defaultValue={0}
          format={fmtSigned(3)}
          disabled={!dist.enabled}
          onChange={(v) => edit(setLens((l) => ({ ...l, distortion: { ...l.distortion, k2: v } })))}
          onCommit={endEdit}
        />
      </>
    ),
    1: (
      <>
        <Slider
          label="Exposure"
          value={a.exposure}
          min={-5}
          max={5}
          step={0.01}
          defaultValue={0}
          format={fmtSigned(2, " EV")}
          onChange={(v) => edit((x) => ({ ...x, exposure: v }))}
          onCommit={endEdit}
        />
        <Toggle
          label="Highlight recovery"
          checked={a.highlightRecovery}
          onChange={(v) => commit((x) => ({ ...x, highlightRecovery: v }))}
        />
        {info.monochrome ? (
          <p className="note">Monochrome sensor: white balance does not apply.</p>
        ) : (
          <>
            <div className="field-row">
              <span className="field-label">White balance</span>
              <Segmented<WbMode>
                label="White balance mode"
                value={wb.mode}
                options={[
                  { value: "asShot", label: "As Shot" },
                  { value: "custom", label: "Custom" },
                ]}
                onChange={(mode) =>
                  commit((x) => ({
                    ...x,
                    whiteBalance:
                      mode === "asShot"
                        ? { ...x.whiteBalance, mode }
                        : { mode, temperature: asShot.temperature, tint: asShot.tint },
                  }))
                }
              />
            </div>
            <Slider
              label="Temperature"
              value={shownWb.temperature}
              min={0}
              max={1}
              step={0.001}
              defaultValue={asShot.temperature}
              toPosition={kToPos}
              fromPosition={posToK}
              format={(v) => `${Math.round(v)} K`}
              track="linear-gradient(90deg, #3d6fd6, #d8d8d8 45%, #e6a73a)"
              onChange={(v) => editWb({ temperature: v })}
              onCommit={endEdit}
            />
            <Slider
              label="Tint"
              value={shownWb.tint}
              min={-150}
              max={150}
              step={1}
              defaultValue={asShot.tint}
              format={fmtSigned(0)}
              track="linear-gradient(90deg, #3fae55, #d8d8d8 50%, #c04fc0)"
              onChange={(v) => editWb({ tint: v })}
              onCommit={endEdit}
            />
          </>
        )}
      </>
    ),
  };

  return (
    <div className="steps">
      {STEPS.map((step, i) => {
        const available = i in content;
        const isOpen = open.has(i);
        return (
          <section key={step.title} className={`step${available ? "" : " is-planned"}`}>
            <button
              type="button"
              className="step-head"
              aria-expanded={isOpen}
              onClick={() => toggle(i)}
            >
              <span className="step-num">{i + 1}</span>
              <span className="step-title">{step.title}</span>
              {!available && <span className="badge">Planned</span>}
              <span className="chevron" aria-hidden>
                {isOpen ? "▾" : "▸"}
              </span>
            </button>
            {isOpen && (
              <div className="step-body">
                {content[i]}
                <p className="note">{available ? "Coming next: " : ""}{step.planned}.</p>
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      {children}
      {hint && <span className="hint">{hint}</span>}
    </label>
  );
}
