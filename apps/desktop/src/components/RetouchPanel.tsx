import { useState } from "react";
import { findDustSpots } from "../api";
import type { Adjustments, BrushStroke, Spot } from "../types";
import type { BrushSettings } from "./ManualMaskTool";
import { Segmented, Slider, Toggle } from "./Slider";

type Update = (fn: (a: Adjustments) => Adjustments) => void;

export type RetouchTool = "erase" | "heal" | null;

/** Retouch state shared by the panel and the viewer. */
export interface RetouchUi {
  tool: RetouchTool;
  setTool: (t: RetouchTool) => void;
  /** Strokes painted over something to erase, not yet applied. */
  pending: BrushStroke[];
  setPending: (s: BrushStroke[]) => void;
  /** Dust spots found and waiting for review. */
  found: Spot[];
  setFound: (s: Spot[]) => void;
  /** Radius of a new healing spot, as a share of the frame width. */
  spotSize: number;
  setSpotSize: (r: number) => void;
}

interface Props {
  path: string;
  adjustments: Adjustments;
  commit: Update;
  ui: RetouchUi;
  brush: BrushSettings;
  setBrush: (b: BrushSettings) => void;
  inpaintAvailable: boolean;
  eyesAvailable: boolean;
}

/**
 * Step 4 retouching: generative erase (paint over something, then Erase: an inpainting
 * model fills it from its surroundings), dust and blemish healing (click, or find dust
 * automatically and review), and red-eye removal.
 */
export function RetouchPanel({ path, adjustments, commit, ui, brush, setBrush, inpaintAvailable, eyesAvailable }: Props) {
  const r = adjustments.retouch;
  const [finding, setFinding] = useState<{ busy: boolean; error: string | null; searched: boolean }>({
    busy: false,
    error: null,
    searched: false,
  });
  const setRetouch = (patch: Partial<Adjustments["retouch"]>) => (x: Adjustments) => ({
    ...x,
    retouch: { ...x.retouch, ...patch },
  });

  const find = async () => {
    setFinding({ busy: true, error: null, searched: false });
    try {
      const spots = await findDustSpots(path, adjustments);
      // Leave out what's already healed.
      const fresh = spots.filter((s) => !r.spots.some((h) => Math.hypot(h.x - s.x, h.y - s.y) < Math.max(h.radius, s.radius)));
      ui.setFound(fresh);
      ui.setTool("heal");
      setFinding({ busy: false, error: null, searched: true });
    } catch (e) {
      setFinding({ busy: false, error: String(e), searched: false });
    }
  };

  return (
    <div className="retouch">
      <span className="field-label">Retouch</span>
      <Segmented<"off" | "erase" | "heal">
        label="Retouch tool"
        value={ui.tool ?? "off"}
        options={[
          { value: "off", label: "Off" },
          { value: "erase", label: "Generative erase" },
          { value: "heal", label: "Heal spots" },
        ]}
        onChange={(t) => ui.setTool(t === "off" ? null : t)}
      />
      {ui.tool === "erase" && (
        <div className="local-card">
          {!inpaintAvailable && (
            <p className="hint error">The inpainting model (lama-fp32.onnx) isn&apos;t installed: run scripts/fetch-models.sh.</p>
          )}
          <p className="hint">Paint over what should go (a person, a sign, a wire), then Erase. Hold Space to pan.</p>
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
          <div className="field-row">
            <button
              type="button"
              className="btn primary"
              disabled={!inpaintAvailable || !ui.pending.some((s) => !s.erase)}
              onClick={() => {
                const strokes = ui.pending;
                commit((x) => ({ ...x, retouch: { ...x.retouch, erase: [...x.retouch.erase, { strokes }] } }));
                ui.setPending([]);
              }}
            >
              Erase
            </button>
            <button type="button" className="btn small" disabled={ui.pending.length === 0} onClick={() => ui.setPending([])}>
              Clear painting
            </button>
          </div>
          {r.erase.length > 0 && (
            <span className="hint">
              {r.erase.length} erased {r.erase.length === 1 ? "area" : "areas"}
            </span>
          )}
          {r.erase.length > 0 && (
            <div className="field-row">
              <button
                type="button"
                className="btn small"
                onClick={() => commit((x) => ({ ...x, retouch: { ...x.retouch, erase: x.retouch.erase.slice(0, -1) } }))}
              >
                Restore last
              </button>
              <button type="button" className="btn small" onClick={() => commit(setRetouch({ erase: [] }))}>
                Restore all
              </button>
            </div>
          )}
          <p className="note">
            The fill is made once (a few seconds on the CPU) and kept: exposure, tone and the look apply to it like
            the rest of the photo.
          </p>
        </div>
      )}
      {ui.tool === "heal" && (
        <div className="local-card">
          <p className="hint">
            Click a spot or blemish to heal it from its surroundings. Found dust is circled in orange: click one to heal
            it. Click a healed (white) circle to undo it.
          </p>
          <Slider
            label="Spot size"
            value={ui.spotSize * 100}
            min={0.2}
            max={5}
            step={0.05}
            defaultValue={0.8}
            format={(v) => `${v.toFixed(2)}%`}
            onChange={(v) => ui.setSpotSize(v / 100)}
            onCommit={() => {}}
          />
          <div className="field-row">
            <button type="button" className="btn" disabled={finding.busy} onClick={find}>
              {finding.busy ? "Finding…" : "Find dust spots"}
            </button>
            {ui.found.length > 0 && (
              <button
                type="button"
                className="btn primary"
                onClick={() => {
                  const found = ui.found;
                  commit((x) => ({ ...x, retouch: { ...x.retouch, spots: [...x.retouch.spots, ...found] } }));
                  ui.setFound([]);
                }}
              >
                Heal all {ui.found.length}
              </button>
            )}
          </div>
          {finding.error && <p className="hint error">{finding.error}</p>}
          {finding.searched && ui.found.length === 0 && <p className="hint">No dust spots found.</p>}
          {ui.found.length > 0 && (
            <button type="button" className="btn small" onClick={() => ui.setFound([])}>
              Dismiss found spots
            </button>
          )}
          {r.spots.length > 0 && (
            <div className="field-row">
              <span className="hint">
                {r.spots.length} healed {r.spots.length === 1 ? "spot" : "spots"}
              </span>
              <button type="button" className="btn small" onClick={() => commit(setRetouch({ spots: [] }))}>
                Remove all
              </button>
            </div>
          )}
        </div>
      )}
      <Toggle
        label="Red-eye removal"
        checked={r.redEye}
        disabled={!eyesAvailable}
        onChange={(redEye) => commit(setRetouch({ redEye }))}
      />
      {!eyesAvailable && <p className="hint">Red-eye removal finds the eyes with the face-parsing model, which isn&apos;t installed.</p>}
    </div>
  );
}
