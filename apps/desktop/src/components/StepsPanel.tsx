import { useState, type ReactNode } from "react";
import type { DepthState } from "../hooks/useDepth";
import { MASK_KINDS, type MaskState } from "../hooks/useMasks";
import {
  HSL_BANDS,
  WHEEL_RANGES,
  type Adjustments,
  type ColorWheel as Wheel,
  type DemosaicMode,
  type HslBand,
  type HslChannel,
  type ImageInfo,
  type MaskKind,
  type WbMode,
  type WheelRange,
} from "../types";
import { ColorWheel } from "./ColorWheel";
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
  masks: MaskState;
  depth: DepthState;
  /** Waiting for a click on the image to place the light source. */
  picking: boolean;
  setPicking: (on: boolean) => void;
}

/** PRD Section 5: the mandatory, displayed order of operations. */
const STEPS: { title: string; planned: string }[] = [
  { title: "RAW Input & Optical Calibration", planned: "Lens profile database, auto-geometry" },
  { title: "Global Exposure & Dynamic Range", planned: "Shadow lift, auto exposure" },
  { title: "AI Subject & Semantic Masking", planned: "Skin, eyes, hair and foreground masks; masks driving local adjustments in later steps" },
  { title: "Micro-Texture & Retouching", planned: "Character line sculpting; retouching confined to the subject mask" },
  { title: "Base Color Grading & HSL", planned: "Foliage shift and background re-coloration through the Step 3 masks" },
  { title: "Atmospheric & Light Sculpting", planned: "3D light placement on the depth map; glow confined to the subject mask" },
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

const MASK_LABEL: Record<MaskKind, string> = { subject: "Subject", sky: "Sky" };

type HslMode = keyof HslChannel;

/** Display hue (HSV degrees) of each HSL band, for slider tracks. */
const BAND_HUE: Record<HslBand, number> = {
  red: 0,
  orange: 30,
  yellow: 55,
  green: 110,
  aqua: 180,
  blue: 225,
  purple: 270,
  magenta: 310,
};

function hslTrack(band: HslBand, mode: HslMode) {
  const h = BAND_HUE[band];
  switch (mode) {
    case "hue":
      return `linear-gradient(90deg, hsl(${h - 30} 70% 50%), hsl(${h} 70% 50%), hsl(${h + 30} 70% 50%))`;
    case "saturation":
      return `linear-gradient(90deg, hsl(${h} 0% 45%), hsl(${h} 85% 50%))`;
    case "luminance":
      return `linear-gradient(90deg, hsl(${h} 60% 15%), hsl(${h} 60% 50%), hsl(${h} 60% 85%))`;
  }
}

const cap = (s: string) => s[0].toUpperCase() + s.slice(1);

const WARMTH_TRACK = "linear-gradient(90deg, #6f9be0, #d8d8d8 50%, #e6a73a)";

export function StepsPanel({
  info,
  adjustments: a,
  edit,
  endEdit,
  commit,
  masks,
  depth,
  picking,
  setPicking,
}: Props) {
  const [open, setOpen] = useState<Set<number>>(() => new Set([0, 1]));
  const [hslMode, setHslMode] = useState<HslMode>("saturation");
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

  const nr = a.noiseReduction;
  const setNr = (patch: Partial<Adjustments["noiseReduction"]>) => (x: Adjustments) => ({
    ...x,
    noiseReduction: { ...x.noiseReduction, ...patch },
  });

  const ca = a.lens.chromaticAberration;
  const dist = a.lens.distortion;
  const setLens = (fn: (l: Adjustments["lens"]) => Adjustments["lens"]): ((x: Adjustments) => Adjustments) =>
    (x) => ({ ...x, lens: fn(x.lens) });

  const tex = a.texture;
  const setTex = (patch: Partial<Adjustments["texture"]>) => (x: Adjustments) => ({
    ...x,
    texture: { ...x.texture, ...patch },
  });
  const texSlider = (label: string, key: keyof Adjustments["texture"], min: number) => (
    <Slider
      label={label}
      value={tex[key]}
      min={min}
      max={100}
      step={1}
      defaultValue={0}
      format={min < 0 ? fmtSigned(0) : (v) => v.toFixed(0)}
      onChange={(v) => edit(setTex({ [key]: v }))}
      onCommit={endEdit}
    />
  );

  const color = a.color;
  const setBand = (band: HslBand, v: number) => (x: Adjustments) => ({
    ...x,
    color: {
      ...x.color,
      hsl: { ...x.color.hsl, [band]: { ...x.color.hsl[band], [hslMode]: v } },
    },
  });
  const setWheel = (range: WheelRange, patch: Partial<Wheel>) => (x: Adjustments) => ({
    ...x,
    color: {
      ...x.color,
      wheels: { ...x.color.wheels, [range]: { ...x.color.wheels[range], ...patch } },
    },
  });

  const at = a.atmosphere;
  const setAt = (patch: Partial<Adjustments["atmosphere"]>) => (x: Adjustments) => ({
    ...x,
    atmosphere: { ...x.atmosphere, ...patch },
  });
  const atSlider = (
    label: string,
    key: keyof Adjustments["atmosphere"],
    def: number,
    opts?: { warmth?: boolean; disabled?: boolean },
  ) => (
    <Slider
      label={label}
      value={at[key] as number}
      min={opts?.warmth ? -100 : 0}
      max={100}
      step={1}
      defaultValue={def}
      format={opts?.warmth ? fmtSigned(0) : (v) => v.toFixed(0)}
      track={opts?.warmth ? WARMTH_TRACK : undefined}
      disabled={opts?.disabled}
      onChange={(v) => edit(setAt({ [key]: v }))}
      onCommit={endEdit}
    />
  );
  const depthAvailable = masks.models?.depth.available ?? false;

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
        <span className="field-label">Noise reduction</span>
        <Slider
          label="Luminance"
          value={nr.luminance}
          min={0}
          max={100}
          step={1}
          defaultValue={0}
          format={(v) => v.toFixed(0)}
          onChange={(v) => edit(setNr({ luminance: v }))}
          onCommit={endEdit}
        />
        {!info.monochrome && (
          <Slider
            label="Color"
            value={nr.color}
            min={0}
            max={100}
            step={1}
            defaultValue={25}
            format={(v) => v.toFixed(0)}
            onChange={(v) => edit(setNr({ color: v }))}
            onCommit={endEdit}
          />
        )}
        <p className="hint">
          The preview is downsampled, which already hides most noise; judge noise reduction on the exported file.
        </p>
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
    2: <MaskControls masks={masks} />,
    3: (
      <>
        {texSlider("Clarity", "clarity", -100)}
        {texSlider("Micro-texture", "microTexture", -100)}
        <span className="field-label">Skin retouching</span>
        {texSlider("Blemish smoothing", "blemishSmoothing", 0)}
        {texSlider("Specular highlight balancing", "specularBalance", 0)}
        <p className="hint">
          Retouching acts only where skin is detected. Micro-texture is cored against noise and eased off on skin, so
          hair, fabric and bark sharpen without making skin look dirty.
        </p>
      </>
    ),
    4: (
      <>
        <div className="field-row">
          <span className="field-label">HSL</span>
          <Segmented<HslMode>
            label="HSL property"
            value={hslMode}
            options={[
              { value: "hue", label: "Hue" },
              { value: "saturation", label: "Saturation" },
              { value: "luminance", label: "Luminance" },
            ]}
            onChange={setHslMode}
          />
        </div>
        {HSL_BANDS.map((band) => (
          <Slider
            key={`${band}-${hslMode}`}
            label={cap(band)}
            value={color.hsl[band][hslMode]}
            min={-100}
            max={100}
            step={1}
            defaultValue={0}
            format={fmtSigned(0)}
            track={hslTrack(band, hslMode)}
            onChange={(v) => edit(setBand(band, v))}
            onCommit={endEdit}
          />
        ))}
        <span className="field-label">Color wheels</span>
        <div className="wheels">
          {WHEEL_RANGES.map((range) => (
            <div key={range} className="wheel-col">
              <ColorWheel
                label={cap(range)}
                value={color.wheels[range]}
                onChange={(v) => edit(setWheel(range, v))}
                onCommit={endEdit}
              />
              <Slider
                label="Lum"
                value={color.wheels[range].luminance}
                min={-100}
                max={100}
                step={1}
                defaultValue={0}
                format={fmtSigned(0)}
                onChange={(v) => edit(setWheel(range, { luminance: v }))}
                onCommit={endEdit}
              />
            </div>
          ))}
        </div>
        <Slider
          label="Skin tone protection"
          value={color.skinProtection}
          min={0}
          max={100}
          step={1}
          defaultValue={0}
          format={(v) => `${v.toFixed(0)}%`}
          onChange={(v) => edit((x) => ({ ...x, color: { ...x.color, skinProtection: v } }))}
          onCommit={endEdit}
        />
        <p className="hint">Shields detected skin from the HSL and wheel changes above.</p>
      </>
    ),
    5: (
      <>
        <span className="field-label">Glow</span>
        {atSlider("Amount", "glow", 0)}
        {atSlider("Size", "glowSize", 50, { disabled: at.glow === 0 })}
        {atSlider("Warmth", "glowWarmth", 0, { warmth: true, disabled: at.glow === 0 })}

        <span className="field-label">Depth fog</span>
        {atSlider("Amount", "fog", 0)}
        {atSlider("Starts at distance", "fogStart", 30, { disabled: at.fog === 0 })}
        {atSlider("Warmth", "fogWarmth", 0, { warmth: true, disabled: at.fog === 0 })}

        <span className="field-label">Light shafts</span>
        {atSlider("Amount", "shafts", 0)}
        {atSlider("Length", "shaftLength", 60, { disabled: at.shafts === 0 })}
        {atSlider("Warmth", "shaftWarmth", 40, { warmth: true, disabled: at.shafts === 0 })}
        <div className="field-row">
          <span className="field-label">Light source</span>
          <Segmented<"auto" | "manual">
            label="Light source"
            value={at.shaftAuto ? "auto" : "manual"}
            options={[
              { value: "auto", label: "Auto" },
              { value: "manual", label: "Placed" },
            ]}
            onChange={(v) => {
              if (v === "auto") {
                setPicking(false);
                commit(setAt({ shaftAuto: true }));
              } else {
                setPicking(true);
              }
            }}
          />
        </div>
        <button
          type="button"
          className={`btn${picking ? " is-active" : ""}`}
          onClick={() => setPicking(!picking)}
        >
          {picking ? "Click the image… (Esc to cancel)" : at.shaftAuto ? "Place light on the image" : "Move light"}
        </button>

        {depthAvailable ? (
          <>
            <Toggle label="Show depth" checked={depth.show} onChange={depth.setShow} />
            {depth.busy && <p className="hint">Estimating depth…</p>}
            {depth.error && <p className="hint error">{depth.error}</p>}
            <p className="hint">
              Depth Anything V2 estimates distance on this computer&apos;s CPU the first time fog or shafts are used.
              Fog thickens with it, and rays stay behind near subjects.
            </p>
          </>
        ) : (
          <p className="hint">
            Depth model not installed, so fog is a uniform haze. Run <code>scripts/fetch-models.sh</code>.
          </p>
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

function MaskControls({ masks: m }: { masks: MaskState }) {
  if (!m.models) return <p className="note">Checking for mask models…</p>;
  const missing = m.models.models.filter((s) => !s.available);
  const any = missing.length < m.models.models.length;
  const detected = MASK_KINDS.filter((k) => m.masks[k]);

  return (
    <>
      {missing.length > 0 && (
        <p className="hint">
          {missing.map((s) => MASK_LABEL[s.kind]).join(" and ")} model{missing.length > 1 ? "s" : ""} not installed.
          Run <code>scripts/fetch-models.sh</code> or copy the <code>.onnx</code> files into <code>{m.models.dir}</code>.
        </p>
      )}
      <button type="button" className="btn" disabled={!any || m.detecting !== null} onClick={() => void m.detect()}>
        {m.detecting
          ? `Detecting ${MASK_LABEL[m.detecting].toLowerCase()}…`
          : detected.length > 0
            ? "Re-detect subject & sky"
            : "Detect subject & sky"}
      </button>
      {m.error && <p className="hint error">{m.error}</p>}
      {detected.length > 0 && (
        <>
          <ul className="mask-list">
            {detected.map((k) => (
              <li key={k}>
                <span>{MASK_LABEL[k]}</span>
                <span className="hint">
                  {(100 * m.masks[k]!.coverage).toFixed(0)}% of frame · {m.masks[k]!.inferMs} ms
                </span>
              </li>
            ))}
          </ul>
          <div className="field-row">
            <span className="field-label">Overlay</span>
            <Segmented<MaskKind | "off">
              label="Mask overlay"
              value={m.overlay ?? "off"}
              options={[
                { value: "off", label: "Off" },
                ...detected.map((k) => ({ value: k, label: MASK_LABEL[k] })),
              ]}
              onChange={(v) => m.setOverlay(v === "off" ? null : v)}
            />
          </div>
          {m.stale && <p className="hint">Lens corrections changed since detection. Re-detect to realign the masks.</p>}
        </>
      )}
      <p className="note">Runs on this computer&apos;s CPU: IS-Net for the subject, U²-Net skyseg for the sky.</p>
    </>
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
