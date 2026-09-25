import { useEffect, useRef, useState } from "react";
import { exportTiff, pickExportPath } from "../api";
import type { Adjustments, ExportReport, ImageInfo, OutputSpace } from "../types";
import { Segmented } from "./Slider";

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

const SPACE_KEY = "epikos.export.space";

type State =
  | { kind: "idle" }
  | { kind: "exporting"; dest: string }
  | { kind: "done"; report: ExportReport }
  | { kind: "error"; message: string };

/** PRD Step 8 handoff: full-resolution 16-bit TIFF with embedded ICC profile. */
export function ExportDialog({ info, adjustments, onClose }: Props) {
  const [space, setSpace] = useState<OutputSpace>(
    () => (localStorage.getItem(SPACE_KEY) as OutputSpace | null) ?? "srgb",
  );
  const [state, setState] = useState<State>({ kind: "idle" });
  const dialog = useRef<HTMLDialogElement>(null);
  const busy = state.kind === "exporting";

  useEffect(() => {
    dialog.current?.showModal();
  }, []);

  const run = async () => {
    localStorage.setItem(SPACE_KEY, space);
    const stem = info.name.replace(/\.[^.]+$/, "");
    const folder = info.path.slice(0, info.path.length - info.name.length);
    const dest = await pickExportPath(`${folder}${stem}.tif`);
    if (!dest) return;
    setState({ kind: "exporting", dest });
    try {
      // Snapshot of the current edits, including any not yet autosaved.
      const report = await exportTiff(info.path, adjustments, dest, space);
      setState({ kind: "done", report });
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
        {info.name} · {info.width}×{info.height} · 16-bit TIFF (ZIP, lossless)
      </p>

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

      {state.kind === "exporting" && <p className="modal-status">Developing at full resolution…</p>}
      {state.kind === "done" && (
        <p className="modal-status is-ok" title={state.report.path}>
          Saved {state.report.path.split("/").pop()} · {(state.report.bytes / 1e6).toFixed(0)} MB ·{" "}
          {((state.report.developMs + state.report.writeMs) / 1000).toFixed(1)} s
        </p>
      )}
      {state.kind === "error" && <p className="modal-status is-error">{state.message}</p>}

      <div className="modal-actions">
        <button type="button" className="btn" disabled={busy} onClick={onClose}>
          {state.kind === "done" ? "Close" : "Cancel"}
        </button>
        <button type="button" className="btn primary" disabled={busy} onClick={() => void run()}>
          {busy ? "Exporting…" : state.kind === "done" ? "Export again…" : "Export…"}
        </button>
      </div>
    </dialog>
  );
}
