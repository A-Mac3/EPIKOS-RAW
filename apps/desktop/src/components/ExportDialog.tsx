import { useEffect, useRef, useState } from "react";
import { exportImage, handoffApps, maskModels, openInApp, pickExportPath } from "../api";
import type {
  Adjustments,
  ExportFormat,
  ExportReport,
  HandoffApp,
  ImageInfo,
  MaskModels,
  OutputSpace,
} from "../types";
import { hasLocation } from "../format";
import { Segmented, Toggle } from "./Slider";

interface Props {
  info: ImageInfo;
  adjustments: Adjustments;
  onClose: () => void;
}

const SPACES: { value: OutputSpace; label: string; hint: string }[] = [
  { value: "srgb", label: "sRGB", hint: "Matches the editor preview exactly. Best for web and most apps." },
  { value: "displayP3", label: "Display P3", hint: "Wider gamut for Apple displays and modern screens." },
  { value: "proPhoto", label: "ProPhoto RGB", hint: "Widest gamut. Recommended for further editing in Photoshop or Lightroom." },
];

const FORMATS: { value: ExportFormat; label: string; ext: string; hint: string }[] = [
  {
    value: "tiff",
    label: "16-bit TIFF",
    ext: "tif",
    hint: "The finished look, lossless (ZIP). AI masks become named alpha channels in Photoshop.",
  },
  {
    value: "psd",
    label: "Layered PSD",
    ext: "psd",
    hint: "The finished look as a layer, plus one empty group per AI mask with that mask applied: drop adjustment layers into a group to confine them to the subject, sky or skin.",
  },
  {
    value: "dng",
    label: "Enhanced DNG",
    ext: "dng",
    hint: "Scene-referred linear DNG (demosaiced, denoised, lens-corrected, white balance and exposure applied; no creative look) for raw-style editing in Lightroom, Camera Raw or Capture One. AI masks become DNG 1.6 semantic masks.",
  },
];

const FORMAT_KEY = "epikos.export.format";
const SPACE_KEY = "epikos.export.space";
const LOCATION_KEY = "epikos.export.includeLocation";
const MASKS_KEY = "epikos.export.aiMasks";
const DEPTH_KEY = "epikos.export.depth";
const SIZE_KEY = "epikos.export.longEdge";
const OPEN_KEY = "epikos.export.openIn";

type Size = "full" | "4096" | "2048";
const SIZES: { value: Size; label: string }[] = [
  { value: "full", label: "Full size" },
  { value: "4096", label: "4096 px" },
  { value: "2048", label: "2048 px" },
];

/** localStorage can throw (private mode, blocked storage); fall back quietly. */
function stored(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function store(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Preferences just won't persist.
  }
}

type State =
  | { kind: "idle" }
  | { kind: "exporting"; dest: string }
  | { kind: "done"; report: ExportReport; opened: string | null; openError: string | null }
  | { kind: "error"; message: string };

/**
 * PRD Step 8 handoff: full-resolution 16-bit TIFF, layered PSD or enhanced DNG, carrying
 * the AI masks when asked, then opened in the next editor.
 */
export function ExportDialog({ info, adjustments, onClose }: Props) {
  const [format, setFormat] = useState<ExportFormat>(() => (stored(FORMAT_KEY) as ExportFormat | null) ?? "tiff");
  const [space, setSpace] = useState<OutputSpace>(() => (stored(SPACE_KEY) as OutputSpace | null) ?? "srgb");
  const [includeLocation, setIncludeLocation] = useState(() => stored(LOCATION_KEY) !== "false");
  const [aiMasks, setAiMasks] = useState(() => stored(MASKS_KEY) === "true");
  const [depthChannel, setDepthChannel] = useState(() => stored(DEPTH_KEY) === "true");
  const [size, setSize] = useState<Size>(() => (stored(SIZE_KEY) as Size | null) ?? "full");
  const [openIn, setOpenIn] = useState(() => stored(OPEN_KEY) ?? "");
  const [apps, setApps] = useState<HandoffApp[]>([]);
  const [models, setModels] = useState<MaskModels | null>(null);
  const [state, setState] = useState<State>({ kind: "idle" });
  const photoHasLocation = hasLocation(info.capture);
  const dialog = useRef<HTMLDialogElement>(null);
  const busy = state.kind === "exporting";

  useEffect(() => {
    dialog.current?.showModal();
    handoffApps().then(setApps, () => setApps([]));
    maskModels().then(setModels, () => setModels(null));
  }, []);

  const openApp = apps.find((a) => a.path === openIn) ?? null;
  const maskModelsMissing = models ? models.models.filter((m) => !m.available).map((m) => m.kind) : [];
  const depthAvailable = models?.depth.available ?? false;

  const run = async () => {
    store(FORMAT_KEY, format);
    store(SPACE_KEY, space);
    store(LOCATION_KEY, String(includeLocation));
    store(MASKS_KEY, String(aiMasks));
    store(DEPTH_KEY, String(depthChannel));
    store(SIZE_KEY, size);
    store(OPEN_KEY, openIn);
    const stem = info.name.replace(/\.[^.]+$/, "");
    const folder = info.path.slice(0, info.path.length - info.name.length);
    const fmt = FORMATS.find((f) => f.value === format)!;
    const suffix = format === "dng" ? "-enhanced" : "";
    const dest = await pickExportPath(`${folder}${stem}${suffix}.${fmt.ext}`, format);
    if (!dest) return;
    setState({ kind: "exporting", dest });
    try {
      // Snapshot of the current edits, including any not yet autosaved.
      const report = await exportImage(info.path, adjustments, dest, {
        format,
        colorSpace: space,
        includeLocation,
        aiMasks,
        depthChannel: depthChannel && depthAvailable,
        longEdge: size === "full" ? null : Number(size),
      });
      let opened: string | null = null;
      let openError: string | null = null;
      if (openApp) {
        try {
          await openInApp(openApp.path, report.path);
          opened = openApp.name;
        } catch (e) {
          openError = String(e);
        }
      }
      setState({ kind: "done", report, opened, openError });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  };

  const hint = SPACES.find((s) => s.value === space)!.hint;

  return (
    <dialog
      ref={dialog}
      className="modal"
      onCancel={(e) => {
        if (busy) e.preventDefault();
        else onClose();
      }}
    >
      <h2>Export</h2>
      <p className="modal-sub">
        {info.name} · {info.width}×{info.height} · 16-bit, 300 ppi
      </p>

      <div className="field">
        <span className="field-label">Format</span>
        <Segmented<ExportFormat>
          label="Format"
          value={format}
          options={FORMATS.map(({ value, label }) => ({ value, label }))}
          onChange={setFormat}
        />
        <span className="hint">{FORMATS.find((f) => f.value === format)!.hint}</span>
      </div>

      {format !== "dng" && (
        <div className="field">
          <span className="field-label">Colour space</span>
          <Segmented<OutputSpace>
            label="Colour space"
            value={space}
            options={SPACES.map(({ value, label }) => ({ value, label }))}
            onChange={setSpace}
          />
          <span className="hint">{hint} The ICC profile is embedded.</span>
        </div>
      )}

      <div className="field">
        <span className="field-label">Size</span>
        <Segmented<Size> label="Size" value={size} options={SIZES} onChange={setSize} />
        <span className="hint">Long edge. Smaller sizes never upscale.</span>
      </div>

      <div className="field">
        <span className="field-label">AI layers</span>
        <Toggle
          label={
            format === "psd"
              ? "Subject, sky and skin as masked layer groups"
              : format === "dng"
                ? "Subject, sky and skin as semantic masks"
                : "Subject, sky and skin masks as alpha channels"
          }
          checked={aiMasks}
          onChange={setAiMasks}
        />
        {aiMasks && maskModelsMissing.length > 0 && (
          <span className="hint">
            {maskModelsMissing.join(" and ")} model not installed, so that mask is left out. Skin is always included.
          </span>
        )}
        {depthAvailable && (
          <Toggle label="Depth map too" checked={depthChannel} onChange={setDepthChannel} />
        )}
        <span className="hint">
          {format === "tiff" &&
            "Photoshop opens these as named channels (Channels panel) for selections and Lens Blur. Other editors use the colour image and ignore them."}
          {format === "psd" && "Each mask becomes a group's mask in the Layers panel; the composite opens anywhere."}
          {format === "dng" &&
            "Stored as DNG 1.6 semantic masks and a DNG 1.5 depth map, as Apple ProRAW does; readers without DNG 1.6 support ignore them."}
        </span>
      </div>

      <div className="field">
        <span className="field-label">Metadata</span>
        <span className="hint">
          Camera, lens, exposure and capture time are copied from the RAW.
        </span>
        {photoHasLocation ? (
          <Toggle label="Include location (GPS)" checked={includeLocation} onChange={setIncludeLocation} />
        ) : (
          <span className="hint">This photo has no location data.</span>
        )}
      </div>

      {apps.length > 0 && (
        <label className="field">
          <span className="field-label">Then open in</span>
          <select value={openIn} onChange={(e) => setOpenIn(e.currentTarget.value)}>
            <option value="">Don&apos;t open</option>
            {apps.map((a) => (
              <option key={a.path} value={a.path}>
                {a.name}
              </option>
            ))}
          </select>
        </label>
      )}

      {state.kind === "exporting" && (
        <p className="modal-status">
          Developing at full resolution{aiMasks || depthChannel ? " and fitting the AI layers" : ""}…
        </p>
      )}
      {state.kind === "done" && (
        <p className="modal-status is-ok" title={state.report.path}>
          Saved {state.report.path.split("/").pop()} ({FORMATS.find((f) => f.value === state.report.format)?.label}) · {state.report.width}×{state.report.height} ·{" "}
          {(state.report.bytes / 1e6).toFixed(0)} MB · {((state.report.developMs + state.report.writeMs) / 1000).toFixed(1)} s
          {state.report.wroteLocation ? " · with location" : ""}
          {state.report.alphaChannels.length > 0 && ` · channels: ${state.report.alphaChannels.join(", ")}`}
          {state.opened && ` · opened in ${state.opened}`}
        </p>
      )}
      {state.kind === "done" && state.openError && <p className="modal-status is-error">{state.openError}</p>}
      {state.kind === "error" && <p className="modal-status is-error">{state.message}</p>}

      <div className="modal-actions">
        <button type="button" className="btn" disabled={busy} onClick={onClose}>
          {state.kind === "done" ? "Close" : "Cancel"}
        </button>
        <button type="button" className="btn primary" disabled={busy} onClick={() => void run()}>
          {busy
            ? "Exporting…"
            : state.kind === "done"
              ? "Export again…"
              : openApp
                ? `Export & open in ${openApp.name}…`
                : "Export…"}
        </button>
      </div>
    </dialog>
  );
}
