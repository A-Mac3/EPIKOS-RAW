import { useState, type ReactNode } from "react";
import { autoTone, autoUpright } from "../api";
import type { DepthState } from "../hooks/useDepth";
import { DepthSideView } from "./DepthSideView";
import { MASK_KINDS, MASK_LABEL, type MaskState } from "../hooks/useMasks";
import {
  CURVE_CHANNELS,
  HSL_BANDS,
  defaultLocal,
  WHEEL_RANGES,
  type Adjustments,
  type ColorWheel as Wheel,
  type CurveChannel,
  type SplitToning,
  type ToneCurve,
  type DemosaicMode,
  type HslBand,
  type HslChannel,
  type ImageInfo,
  type LocalAdjustment,
  type MaskTarget,
  type PickTarget,
  type Tone,
  type WbMode,
  type WheelRange,
  cropIsActive,
  defaultCrop,
} from "../types";
import { ColorWheel } from "./ColorWheel";
import { ManualMaskPanel } from "./ManualMaskPanel";
import type { BrushSettings } from "./ManualMaskTool";
import { CurveGraph } from "./CurveGraph";
import { bakeCurve, isIdentity } from "../curve";
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
  /** Preview histograms, drawn behind the curve canvas. */
  histogram: [Uint32Array, Uint32Array, Uint32Array] | null;
  /** Waiting for a click on the image to place a light. */
  picking: PickTarget;
  setPicking: (target: PickTarget) => void;
  selectedLight: number | null;
  setSelectedLight: (i: number | null) => void;
  /** Open or close the crop tool on the image. */
  cropping: boolean;
  setCropping: (on: boolean) => void;
  /** The hand-drawn mask being edited, and brush settings for new strokes. */
  manualActive: number | null;
  setManualActive: (i: number | null) => void;
  brush: BrushSettings;
  setBrush: (b: BrushSettings) => void;
}

/** PRD Section 5: the mandatory, displayed order of operations. */
const STEPS: string[] = [
  "RAW Input & Optical Calibration",
  "Global Exposure & Dynamic Range",
  "AI Subject & Semantic Masking",
  "Micro-Texture & Retouching",
  "Base Color Grading & HSL",
  "Atmospheric & Light Sculpting",
  "Creative Split-Toning & Curves",
  "Final Finishing & Handoff",
];

const TONE_SLIDERS: [keyof Tone, string][] = [
  ["contrast", "Contrast"],
  ["highlights", "Highlights"],
  ["shadows", "Shadows"],
  ["whites", "Whites"],
  ["blacks", "Blacks"],
  ["vibrance", "Vibrance"],
  ["saturation", "Saturation"],
  ["dehaze", "Dehaze"],
];

// Temperature slider is logarithmic so the useful 2 000–10 000 K range gets most travel.
const K_MIN = 2000;
const K_MAX = 25000;
const kToPos = (k: number) => Math.log(k / K_MIN) / Math.log(K_MAX / K_MIN);
const posToK = (p: number) => Math.round(K_MIN * Math.pow(K_MAX / K_MIN, p));

const fmtSigned = (digits: number, unit = "") => (v: number) =>
  `${v > 0 ? "+" : ""}${v.toFixed(digits)}${unit}`;

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

const HUE_TRACK = `linear-gradient(90deg, ${[0, 60, 120, 180, 240, 300, 360].map((h) => `hsl(${h} 80% 55%)`).join(", ")})`;

const CURVE_LABEL: Record<CurveChannel, string> = { rgb: "RGB", red: "Red", green: "Green", blue: "Blue" };


export function StepsPanel({
  info,
  adjustments: a,
  edit,
  endEdit,
  commit,
  masks,
  depth,
  histogram,
  picking,
  setPicking,
  selectedLight,
  setSelectedLight,
  cropping,
  setCropping,
  manualActive,
  setManualActive,
  brush,
  setBrush,
}: Props) {
  const [maskTab, setMaskTab] = useState<"ai" | "manual">(() => (manualActive !== null ? "manual" : "ai"));
  const [open, setOpen] = useState<Set<number>>(() => new Set([0, 1]));
  const [hslMode, setHslMode] = useState<HslMode>("saturation");
  const [curveChannel, setCurveChannel] = useState<CurveChannel>("rgb");
  const [auto, setAuto] = useState<{ busy: "tone" | "upright" | null; error: string | null }>({
    busy: null,
    error: null,
  });
  const runAuto = async (kind: "tone" | "upright") => {
    setAuto({ busy: kind, error: null });
    try {
      if (kind === "tone") {
        const r = await autoTone(info.path, a);
        // Auto tone doesn't judge haze: keep the Dehaze setting.
        commit((x) => ({ ...x, exposure: r.exposure, tone: { ...r.tone, dehaze: x.tone.dehaze } }));
      } else {
        const r = await autoUpright(info.path, a);
        commit((x) => ({ ...x, lens: { ...x.lens, rotation: r.rotation, vertical: r.vertical } }));
      }
      setAuto({ busy: null, error: null });
    } catch (e) {
      setAuto({ busy: null, error: String(e) });
    }
  };
  const subjectAvailable = masks.available("subject");
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
  const texSlider = (
    label: string,
    key: "clarity" | "microTexture" | "blemishSmoothing" | "specularBalance" | "characterLines",
    min: number,
  ) => (
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
  const setBg = (patch: Partial<Adjustments["color"]["background"]>) => (x: Adjustments) => ({
    ...x,
    color: { ...x.color, background: { ...x.color.background, ...patch } },
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

  // The canvas edits points only: show parametric shaping (older sidecars) as points.
  const curve = bakeCurve(a.curves[curveChannel]);
  const shownCurves = { ...a.curves, [curveChannel]: curve };
  const setCurve = (patch: Partial<ToneCurve>) => (x: Adjustments) => ({
    ...x,
    curves: { ...x.curves, [curveChannel]: { ...bakeCurve(x.curves[curveChannel]), ...patch } },
  });
  const fin = a.finishing;
  const finSlider = (
    label: string,
    key: keyof Adjustments["finishing"],
    def: number,
    opts?: { signed?: boolean; disabled?: boolean },
  ) => (
    <Slider
      label={label}
      value={fin[key]}
      min={opts?.signed ? -100 : 0}
      max={100}
      step={1}
      defaultValue={def}
      format={opts?.signed ? fmtSigned(0) : (v) => v.toFixed(0)}
      disabled={opts?.disabled}
      onChange={(v) => edit((x) => ({ ...x, finishing: { ...x.finishing, [key]: v } }))}
      onCommit={endEdit}
    />
  );
  const st = a.splitToning;
  const setSplit = (patch: Partial<SplitToning>) => (x: Adjustments) => ({
    ...x,
    splitToning: { ...x.splitToning, ...patch },
  });
  const toneRow = (
    label: string,
    hueKey: "highlightHue" | "shadowHue",
    satKey: "highlightSaturation" | "shadowSaturation",
    defHue: number,
  ) => (
    <>
      <div className="field-row">
        <span className="field-label">{label}</span>
        <span
          className="tone-swatch"
          style={{ background: `hsl(${st[hueKey]} ${Math.max(8, st[satKey])}% 50%)` }}
          aria-hidden
        />
      </div>
      <Slider
        label="Hue"
        value={st[hueKey]}
        min={0}
        max={360}
        step={1}
        defaultValue={defHue}
        format={(v) => `${v.toFixed(0)}°`}
        track={HUE_TRACK}
        onChange={(v) => edit(setSplit({ [hueKey]: v }))}
        onCommit={endEdit}
      />
      <Slider
        label="Saturation"
        value={st[satKey]}
        min={0}
        max={100}
        step={1}
        defaultValue={0}
        format={(v) => v.toFixed(0)}
        track={`linear-gradient(90deg, hsl(${st[hueKey]} 0% 45%), hsl(${st[hueKey]} 80% 50%))`}
        onChange={(v) => edit(setSplit({ [satKey]: v }))}
        onCommit={endEdit}
      />
    </>
  );

  const content: Record<number, ReactNode> = {
    0: (
      <>
        <Toggle
          label="Lens profile"
          checked={a.lens.profile}
          disabled={!info.lensProfile}
          onChange={(v) => commit(setLens((l) => ({ ...l, profile: v })))}
        />
        <p className="hint">
          {info.lensProfile
            ? `${info.lensProfile}: distortion, lateral chromatic aberration and vignetting.`
            : info.bitmap
              ? "A JPEG or PNG is already corrected by the camera or editor that made it."
              : `No profile for ${info.capture.lensModel ?? "this lens"} in the camera file or the built-in Lensfun database (${masks.models?.lensDatabase ?? "…"} lenses).`}
        </p>
        <span className="field-label">Geometry</span>
        <div className="geometry-actions">
          <button type="button" className="btn" disabled={auto.busy !== null} onClick={() => void runAuto("upright")}>
            {auto.busy === "upright" ? "Measuring…" : "Auto upright"}
          </button>
          <button
            type="button"
            className={`btn${cropping ? " is-active" : ""}`}
            aria-pressed={cropping}
            title="Crop & straighten on the image (C)"
            onClick={() => setCropping(!cropping)}
          >
            {cropping ? "Done cropping" : "Crop & straighten"}
          </button>
        </div>
        {cropIsActive(a.crop) && (
          <p className="hint">
            Cropped to {Math.round(a.crop.width * info.width)}×{Math.round(a.crop.height * info.height)} px
            {a.crop.aspect !== "free" ? ` (${a.crop.aspect})` : ""}.{" "}
            <button type="button" className="btn link" onClick={() => commit((x) => ({ ...x, crop: defaultCrop() }))}>
              Reset crop
            </button>
          </p>
        )}
        <Slider
          label="Straighten"
          value={a.lens.rotation}
          min={-45}
          max={45}
          step={0.05}
          defaultValue={0}
          format={fmtSigned(2, "°")}
          onChange={(v) => edit(setLens((l) => ({ ...l, rotation: v })))}
          onCommit={endEdit}
        />
        <Slider
          label="Vertical perspective"
          value={a.lens.vertical}
          min={-100}
          max={100}
          step={1}
          defaultValue={0}
          format={fmtSigned(0)}
          onChange={(v) => edit(setLens((l) => ({ ...l, vertical: v })))}
          onCommit={endEdit}
        />
        {auto.error && <p className="hint error">{auto.error}</p>}
        <p className="hint">
          Auto upright levels the horizon and straightens converging verticals from the photo&apos;s own lines. The frame
          is enlarged just enough to stay filled.
        </p>
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
        {TONE_SLIDERS.map(([key, label]) => (
          <Slider
            key={key}
            label={label}
            value={a.tone[key]}
            min={-100}
            max={100}
            step={1}
            defaultValue={0}
            format={fmtSigned(0)}
            onChange={(v) => edit((x) => ({ ...x, tone: { ...x.tone, [key]: v } }))}
            onCommit={endEdit}
          />
        ))}
        <button type="button" className="btn" disabled={auto.busy !== null} onClick={() => void runAuto("tone")}>
          {auto.busy === "tone" ? "Measuring…" : "Auto exposure & tone"}
        </button>
        <p className="hint">
          Auto sets mid-tones to mid-grey from the whole frame (not the skin, so deep skin keeps its depth), lifts
          shadows when much of the frame is dark and pulls back highlights that would clip.
        </p>
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
    2: (
      <>
        <Segmented<"ai" | "manual">
          label="Mask type"
          value={maskTab}
          options={[
            { value: "ai", label: "AI masks" },
            { value: "manual", label: `Manual masking${a.manual.length ? ` (${a.manual.length})` : ""}` },
          ]}
          onChange={(t) => {
            setMaskTab(t);
            if (t === "ai") setManualActive(null);
          }}
        />
        {maskTab === "ai" ? (
          <>
            <MaskControls masks={masks} />
            <LocalAdjustments local={a.local} masks={masks} edit={edit} endEdit={endEdit} commit={commit} />
          </>
        ) : (
          <ManualMaskPanel
            manual={a.manual}
            edit={edit}
            endEdit={endEdit}
            commit={commit}
            active={manualActive}
            setActive={setManualActive}
            brush={brush}
            setBrush={setBrush}
          />
        )}
      </>
    ),
    3: (
      <>
        {texSlider("Clarity", "clarity", -100)}
        {texSlider("Micro-texture", "microTexture", -100)}
        <span className="field-label">Skin retouching</span>
        {texSlider("Blemish smoothing", "blemishSmoothing", 0)}
        {texSlider("Specular highlight balancing", "specularBalance", 0)}
        {texSlider("Character lines", "characterLines", -100)}
        <Toggle
          label="Retouch the subject only"
          checked={tex.retouchSubjectOnly}
          disabled={!subjectAvailable}
          onChange={(v) => commit(setTex({ retouchSubjectOnly: v }))}
        />
        <p className="hint">
          Character lines deepen (+) or soften (−) the lines that give a face its character (smile lines, furrows)
          without touching pores. With the subject only, skin-coloured wood, sand or brick behind the subject stays
          untouched{subjectAvailable ? "" : " (needs the subject model)"}.
        </p>
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

        <span className="field-label">Foliage</span>
        {(["hue", "saturation", "luminance"] as const).map((key) => (
          <Slider
            key={key}
            label={cap(key)}
            value={color.foliage[key]}
            min={-100}
            max={100}
            step={1}
            defaultValue={0}
            format={fmtSigned(0)}
            track={key === "hue" ? "linear-gradient(90deg, #2f9c8f, #5c9a3a 50%, #c99a2e)" : undefined}
            onChange={(v) => edit((x) => ({ ...x, color: { ...x.color, foliage: { ...x.color.foliage, [key]: v } } }))}
            onCommit={endEdit}
          />
        ))}
        <p className="hint">Turns greenery towards teal (−) or autumn gold (+), leaving the subject alone.</p>

        <span className="field-label">Background</span>
        {subjectAvailable ? (
          <>
            <Slider
              label="Tint hue"
              value={color.background.hue}
              min={0}
              max={360}
              step={1}
              defaultValue={210}
              format={(v) => `${v.toFixed(0)}°`}
              track={HUE_TRACK}
              onChange={(v) => edit(setBg({ hue: v }))}
              onCommit={endEdit}
            />
            <Slider
              label="Tint amount"
              value={color.background.amount}
              min={0}
              max={100}
              step={1}
              defaultValue={0}
              format={(v) => v.toFixed(0)}
              onChange={(v) => edit(setBg({ amount: v }))}
              onCommit={endEdit}
            />
            <Slider
              label="Saturation"
              value={color.background.saturation}
              min={-100}
              max={100}
              step={1}
              defaultValue={0}
              format={fmtSigned(0)}
              onChange={(v) => edit(setBg({ saturation: v }))}
              onCommit={endEdit}
            />
            <Slider
              label="Luminance"
              value={color.background.luminance}
              min={-100}
              max={100}
              step={1}
              defaultValue={0}
              format={fmtSigned(0)}
              onChange={(v) => edit(setBg({ luminance: v }))}
              onCommit={endEdit}
            />
            <p className="hint">Re-colours everything outside the subject mask.</p>
          </>
        ) : (
          <p className="hint">Background re-colouration needs the subject model (scripts/fetch-models.sh).</p>
        )}
      </>
    ),
    5: (
      <>
        <span className="field-label">Glow</span>
        {atSlider("Amount", "glow", 0)}
        {atSlider("Size", "glowSize", 50, { disabled: at.glow === 0 })}
        {atSlider("Warmth", "glowWarmth", 0, { warmth: true, disabled: at.glow === 0 })}
        <Toggle
          label="Subject only"
          checked={at.glowSubjectOnly}
          disabled={!subjectAvailable || at.glow === 0}
          onChange={(v) => commit(setAt({ glowSubjectOnly: v }))}
        />

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
                setPicking(null);
                commit(setAt({ shaftAuto: true }));
              } else {
                setPicking("shafts");
              }
            }}
          />
        </div>
        <button
          type="button"
          className={`btn${picking === "shafts" ? " is-active" : ""}`}
          onClick={() => setPicking(picking === "shafts" ? null : "shafts")}
        >
          {picking === "shafts"
            ? "Click the image… (Esc to cancel)"
            : at.shaftAuto
              ? "Place light on the image"
              : "Move light"}
        </button>

        <span className="field-label">3D light sculptor</span>
        <div className="light-list">
          {at.lights.map((l, i) => (
            <button
              key={i}
              type="button"
              className={`light-chip${i === selectedLight ? " is-active" : ""}`}
              onClick={() => setSelectedLight(i)}
              style={{ ["--light" as string]: l.warmth >= 0 ? `hsl(40 90% ${75 - l.warmth / 5}%)` : `hsl(210 80% ${75 + l.warmth / 5}%)` }}
            >
              Light {i + 1}
            </button>
          ))}
          <button
            type="button"
            className={`btn${picking === "light" ? " is-active" : ""}`}
            disabled={at.lights.length >= 4}
            onClick={() => setPicking(picking === "light" ? null : "light")}
          >
            {picking === "light" ? "Click the image…" : "+ Add light"}
          </button>
        </div>
        {selectedLight !== null && at.lights[selectedLight] && (() => {
          const i = selectedLight;
          const light = at.lights[i];
          const setLight = (patch: Partial<typeof light>) => (x: Adjustments) => ({
            ...x,
            atmosphere: {
              ...x.atmosphere,
              lights: x.atmosphere.lights.map((l, k) => (k === i ? { ...l, ...patch } : l)),
            },
          });
          const lightSlider = (label: string, key: "intensity" | "reach" | "warmth" | "halo", def: number) => (
            <Slider
              label={label}
              value={light[key]}
              min={key === "warmth" ? -100 : 0}
              max={100}
              step={1}
              defaultValue={def}
              format={key === "warmth" ? fmtSigned(0) : (v) => v.toFixed(0)}
              track={key === "warmth" ? WARMTH_TRACK : undefined}
              onChange={(v) => edit(setLight({ [key]: v }))}
              onCommit={endEdit}
            />
          );
          return (
            <>
              <DepthSideView
                depth={depth.map}
                light={light}
                onMove={(x, d) => edit(setLight({ x, depth: d }))}
                onCommit={endEdit}
              />
              <Slider
                label="Depth (camera ↔ far)"
                value={light.depth}
                min={0}
                max={1}
                step={0.01}
                defaultValue={0.5}
                format={(v) => `${Math.round(v * 100)}%`}
                onChange={(v) => edit(setLight({ depth: v }))}
                onCommit={endEdit}
              />
              {lightSlider("Intensity", "intensity", 50)}
              {lightSlider("Reach", "reach", 50)}
              {lightSlider("Warmth", "warmth", 40)}
              {lightSlider("Halo", "halo", 40)}
              <button
                type="button"
                className="btn"
                onClick={() => {
                  commit((x) => ({
                    ...x,
                    atmosphere: { ...x.atmosphere, lights: x.atmosphere.lights.filter((_, k) => k !== i) },
                  }));
                  setSelectedLight(null);
                }}
              >
                Remove light {i + 1}
              </button>
            </>
          );
        })()}
        <p className="hint">
          Drag a light on the photo to move it; drag it in the side view to set its depth. Behind the subject it rims
          the silhouette and its glow shows around it; in front it lights the faces turned towards it.
        </p>

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
    6: (
      <>
        <div className="field-row">
          <span className="field-label">Curve</span>
          <Segmented<CurveChannel>
            label="Curve channel"
            value={curveChannel}
            options={CURVE_CHANNELS.map((c) => ({ value: c, label: CURVE_LABEL[c] }))}
            onChange={setCurveChannel}
          />
        </div>
        <CurveGraph
          curves={shownCurves}
          channel={curveChannel}
          histogram={histogram}
          onPoints={(points) => edit(setCurve({ points }))}
          onCommit={endEdit}
        />
        <p className="hint">
          Click the curve to add a point and drag it; double-click a point or drag it off the graph to remove it. Arrow
          keys nudge the selected point.
        </p>
        <button
          type="button"
          className="btn"
          disabled={isIdentity(curve)}
          onClick={() =>
            commit(setCurve({ shadows: 0, darks: 0, lights: 0, highlights: 0, black: 0, white: 0, points: [] }))
          }
        >
          Reset {CURVE_LABEL[curveChannel]} curve
        </button>

        <Toggle
          label="Standard S-Curve"
          checked={a.curves.sCurve.enabled}
          onChange={(v) => commit((x) => ({ ...x, curves: { ...x.curves, sCurve: { ...x.curves.sCurve, enabled: v } } }))}
        />
        <Slider
          label="S-Curve strength"
          value={a.curves.sCurve.amount}
          min={0}
          max={100}
          step={1}
          defaultValue={50}
          format={(v) => v.toFixed(0)}
          disabled={!a.curves.sCurve.enabled}
          onChange={(v) => edit((x) => ({ ...x, curves: { ...x.curves, sCurve: { ...x.curves.sCurve, amount: v } } }))}
          onCommit={endEdit}
        />
        <Slider
          label="S-Curve contrast pivot"
          value={a.curves.sCurve.pivot}
          min={0}
          max={100}
          step={1}
          defaultValue={50}
          format={(v) => (v === 50 ? "mid" : v < 50 ? `darker ${50 - v}` : `brighter ${v - 50}`)}
          disabled={!a.curves.sCurve.enabled}
          onChange={(v) => edit((x) => ({ ...x, curves: { ...x.curves, sCurve: { ...x.curves.sCurve, pivot: v } } }))}
          onCommit={endEdit}
        />
        <p className="hint">
          A classic contrast S under your RGB curve (shown dashed): tones below the pivot darken, above it brighten, and
          black and white stay put. Move the pivot down to keep shadows open, up to protect highlights.
        </p>

        <span className="field-label">Split toning</span>
        {toneRow("Highlights", "highlightHue", "highlightSaturation", 40)}
        {toneRow("Shadows", "shadowHue", "shadowSaturation", 215)}
        <Slider
          label="Balance"
          value={st.balance}
          min={-100}
          max={100}
          step={1}
          defaultValue={0}
          format={fmtSigned(0)}
          onChange={(v) => edit(setSplit({ balance: v }))}
          onCommit={endEdit}
        />
        <p className="hint">Positive balance gives more of the image the highlight tint.</p>
      </>
    ),
    7: (
      <>
        <span className="field-label">Vignette</span>
        {finSlider("Amount", "vignette", 0, { signed: true })}
        {finSlider("Midpoint", "vignetteMidpoint", 50, { disabled: fin.vignette === 0 })}
        {finSlider("Feather", "vignetteFeather", 50, { disabled: fin.vignette === 0 })}
        {finSlider("Roundness", "vignetteRoundness", 0, { signed: true, disabled: fin.vignette === 0 })}

        <span className="field-label">Film grain</span>
        {finSlider("Amount", "grain", 0)}
        {finSlider("Size", "grainSize", 25, { disabled: fin.grain === 0 })}
        {finSlider("Roughness", "grainRoughness", 50, { disabled: fin.grain === 0 })}
        <p className="hint">Grain scales with the frame, so the preview shows it softer than the full-size export.</p>

        <span className="field-label">Handoff</span>
        <p className="hint">
          Export… (⌘E) writes a 16-bit TIFF, a layered PSD with the AI masks as masked groups, or an enhanced linear
          DNG with semantic masks, and can open it in Photoshop, Lightroom, Capture One or DxO.
        </p>
      </>
    ),
  };

  return (
    <div className="steps">
      {STEPS.map((title, i) => {
        const isOpen = open.has(i);
        return (
          <section key={title} className="step">
            <button type="button" className="step-head" aria-expanded={isOpen} onClick={() => toggle(i)}>
              <span className="step-num">{i + 1}</span>
              <span className="step-title">{title}</span>
              <span className="chevron" aria-hidden>
                {isOpen ? "▾" : "▸"}
              </span>
            </button>
            {isOpen && <div className="step-body">{content[i]}</div>}
          </section>
        );
      })}
    </div>
  );
}

function MaskControls({ masks: m }: { masks: MaskState }) {
  if (!m.models) return <p className="note">Checking for mask models…</p>;
  const targets = m.models.targets;
  const missing = targets.filter((t) => !t.available);
  const detected = MASK_KINDS.filter((k) => m.masks[k]);

  return (
    <>
      {missing.length > 0 && (
        <p className="hint">
          {missing.map((t) => MASK_LABEL[t.target]).join(", ")}: model not installed. Run{" "}
          <code>scripts/fetch-models.sh</code> or copy the <code>.onnx</code> files into <code>{m.models.dir}</code>.
        </p>
      )}
      <button
        type="button"
        className="btn"
        disabled={missing.length === targets.length || m.detecting !== null}
        onClick={() => void m.detect()}
      >
        {m.detecting
          ? `Detecting ${MASK_LABEL[m.detecting].toLowerCase()}…`
          : detected.length > 0
            ? "Re-detect masks"
            : "Detect masks"}
      </button>
      {m.error && <p className="hint error">{m.error}</p>}
      {detected.length > 0 && (
        <>
          <ul className="mask-list">
            {detected.map((k) => (
              <li key={k}>
                <span>{MASK_LABEL[k]}</span>
                <span className="hint">
                  {(100 * m.masks[k]!.coverage).toFixed(k === "eyes" ? 1 : 0)}% of frame
                  {m.masks[k]!.inferMs > 0 ? ` · ${m.masks[k]!.inferMs} ms` : ""}
                </span>
              </li>
            ))}
          </ul>
          <Field label="Overlay">
            <select
              value={m.overlay ?? "off"}
              onChange={(e) => {
                const v = e.currentTarget.value;
                m.setOverlay(v === "off" ? null : (v as MaskTarget));
              }}
            >
              <option value="off">Off</option>
              {detected.map((k) => (
                <option key={k} value={k}>
                  {MASK_LABEL[k]}
                </option>
              ))}
            </select>
          </Field>
          {m.stale && <p className="hint">Lens or geometry changed since detection. Re-detect to realign the masks.</p>}
        </>
      )}
      <p className="note">
        Runs on this computer&apos;s CPU: IS-Net (subject), U²-Net skyseg (sky), BiSeNet face parsing (eyes, hair),
        Depth Anything V2 (foreground) and a colour model (skin).
      </p>
    </>
  );
}

const LOCAL_SLIDERS: [Exclude<keyof LocalAdjustment, "mask">, string][] = [
  ["exposure", "Exposure"],
  ["contrast", "Contrast"],
  ["saturation", "Saturation"],
  ["warmth", "Warmth"],
  ["tint", "Tint"],
  ["clarity", "Clarity"],
];

/** Green to magenta, for local Tint. */
const TINT_TRACK = "linear-gradient(90deg, #4caf50, #d8d8d8 50%, #d04fb0)";

/** Step 3 edits confined to a mask. */
function LocalAdjustments({
  local,
  masks,
  edit,
  endEdit,
  commit,
}: {
  local: LocalAdjustment[];
  masks: MaskState;
  edit: Update;
  endEdit: () => void;
  commit: Update;
}) {
  const [target, setTarget] = useState<MaskTarget>("subject");
  const usable = MASK_KINDS.filter((k) => masks.available(k));
  const setLocal = (i: number, patch: Partial<LocalAdjustment>) => (x: Adjustments) => ({
    ...x,
    local: x.local.map((l, k) => (k === i ? { ...l, ...patch } : l)),
  });

  return (
    <>
      <span className="field-label">Local adjustments</span>
      {local.map((l, i) => (
        <div key={i} className="local-card">
          <div className="field-row">
            <select
              aria-label={`Mask of adjustment ${i + 1}`}
              value={l.mask}
              onChange={(e) => commit(setLocal(i, { mask: e.currentTarget.value as MaskTarget }))}
            >
              {MASK_KINDS.map((k) => (
                <option key={k} value={k} disabled={!masks.available(k)}>
                  {MASK_LABEL[k]}
                </option>
              ))}
            </select>
            <button
              type="button"
              className="btn link"
              onClick={() => commit((x) => ({ ...x, local: x.local.filter((_, k) => k !== i) }))}
            >
              Remove
            </button>
          </div>
          {!masks.available(l.mask) && <p className="hint error">This mask&apos;s model isn&apos;t installed; the edit has no effect.</p>}
          {LOCAL_SLIDERS.map(([key, label]) => (
            <Slider
              key={key}
              label={label}
              value={l[key]}
              min={key === "exposure" ? -3 : -100}
              max={key === "exposure" ? 3 : 100}
              step={key === "exposure" ? 0.01 : 1}
              defaultValue={0}
              format={key === "exposure" ? fmtSigned(2, " EV") : fmtSigned(0)}
              track={key === "warmth" ? WARMTH_TRACK : key === "tint" ? TINT_TRACK : undefined}
              onChange={(v) => edit(setLocal(i, { [key]: v }))}
              onCommit={endEdit}
            />
          ))}
        </div>
      ))}
      <div className="field-row">
        <select aria-label="Mask for a new adjustment" value={target} onChange={(e) => setTarget(e.currentTarget.value as MaskTarget)}>
          {MASK_KINDS.map((k) => (
            <option key={k} value={k} disabled={!usable.includes(k)}>
              {MASK_LABEL[k]}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="btn"
          disabled={!usable.includes(target) || local.length >= 8}
          onClick={() => commit((x) => ({ ...x, local: [...x.local, defaultLocal(target)] }))}
        >
          + Add
        </button>
      </div>
      <p className="hint">
        Each edit follows its mask, made on demand the first time it&apos;s used (a second or two on the CPU) and snapped to
        the photo&apos;s edges. Brighten the eyes, cool the background, add clarity to hair.
      </p>
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
