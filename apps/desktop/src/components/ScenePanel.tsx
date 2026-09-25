import { useEffect, useState } from "react";
import { analyzeImage, syncLook, undoSync } from "../api";
import type { Adjustments, ImageInfo, SceneAnalysis, ShotGroup, Swatch, SyncReport } from "../types";

/** Let the first preview render before the (≈3 s) analysis competes for the engine. */
const START_DELAY_MS = 700;

// One analysis per photo per session: it reads the scene, which edits don't change.
const cache = new Map<string, Promise<SceneAnalysis>>();

function analysisFor(info: ImageInfo): Promise<SceneAnalysis> {
  let p = cache.get(info.path);
  if (!p) {
    p = analyzeImage(info.path, info.document.adjustments);
    p.catch(() => cache.delete(info.path));
    cache.set(info.path, p);
  }
  return p;
}

type State = { kind: "loading" } | { kind: "done"; analysis: SceneAnalysis } | { kind: "error"; message: string };

/** PRD Section 2.1: what the engine reads in this photo (genre, light, skin). */
export function SceneCard({ info }: { info: ImageInfo }) {
  const [open, setOpen] = useState(true);
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    let alive = true;
    setState({ kind: "loading" });
    const start = window.setTimeout(
      () =>
        analysisFor(info).then(
          (analysis) => alive && setState({ kind: "done", analysis }),
          (e) => alive && setState({ kind: "error", message: String(e) }),
        ),
      cache.has(info.path) ? 0 : START_DELAY_MS,
    );
    return () => {
      alive = false;
      window.clearTimeout(start);
    };
  }, [info]);

  const top = state.kind === "done" ? state.analysis.genres[0] : undefined;
  return (
    <section className="scene">
      <button type="button" className="step-head" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span className="styles-icon" aria-hidden>
          ◎
        </span>
        <span className="step-title">Scene analysis</span>
        {state.kind === "loading" && <span className="badge">Reading…</span>}
        {top && <span className="badge is-on">{top.label}</span>}
        <span className="chevron" aria-hidden>
          {open ? "▾" : "▸"}
        </span>
      </button>
      {open && (
        <div className="step-body scene-body">
          {state.kind === "loading" && <p className="note">Reading genre, light and skin…</p>}
          {state.kind === "error" && <p className="error">{state.message}</p>}
          {state.kind === "done" && <Analysis a={state.analysis} />}
        </div>
      )}
    </section>
  );
}

function Analysis({ a }: { a: SceneAnalysis }) {
  const L = a.lighting;
  const light = [L.hardnessLabel, L.direction, L.backlit ? "backlit" : null].filter(Boolean).join(" · ");
  const clip = L.highlightsClipped >= 0.05 ? `, ${L.highlightsClipped.toFixed(1)}% clipped` : "";
  const crush = L.shadowsCrushed >= 0.05 ? `, ${L.shadowsCrushed.toFixed(1)}% crushed` : "";
  const weather = [L.hazeLabel, L.snow >= 0.15 ? "snow likely" : null].filter(Boolean).join(" · ");
  return (
    <>
      <div className="scene-group">
        <h4>Genre</h4>
        {a.genres.length === 0 ? (
          <p className="note">No genre fits clearly.</p>
        ) : (
          <ul className="genres">
            {a.genres.map((g) => (
              <li key={g.id} title={g.evidence}>
                <div className="genre-row">
                  <span>{g.label}</span>
                  <span className="genre-score">{Math.round(g.score * 100)}%</span>
                </div>
                <div className="genre-bar">
                  <span style={{ width: `${g.score * 100}%` }} />
                </div>
                <small>{g.evidence}</small>
              </li>
            ))}
          </ul>
        )}
        {a.genres.length > 1 && <p className="note">More than one genre: the photo is a hybrid.</p>}
      </div>

      <div className="scene-group">
        <h4>Light</h4>
        <dl className="facts">
          {L.colorTemperature !== null && (
            <>
              <dt>Colour</dt>
              <dd>{Math.round(L.colorTemperature / 50) * 50} K as shot</dd>
            </>
          )}
          <dt>Light</dt>
          <dd>{light}</dd>
          <dt>Range</dt>
          <dd>
            {L.dynamicRangeEv.toFixed(1)} EV, {L.key}
            {clip}
            {crush}
          </dd>
          <dt>Air</dt>
          <dd>{weather}</dd>
          {L.timeOfDay && (
            <>
              <dt>Time</dt>
              <dd>{L.timeOfDay}</dd>
            </>
          )}
        </dl>
      </div>

      {a.skin && (
        <div className="scene-group">
          <h4>Skin</h4>
          <dl className="facts">
            <dt>Tone</dt>
            <dd>
              {a.skin.toneLabel} (Monk ~{Math.round(a.skin.toneDepth)}/10), {a.skin.undertone} undertone
            </dd>
            <dt>Shine</dt>
            <dd>{a.skin.shineLabel}</dd>
            <dt>Texture</dt>
            <dd>{a.skin.textureLabel}</dd>
            <dt>Area</dt>
            <dd>{a.skin.coverage.toFixed(1)}% of the frame</dd>
          </dl>
        </div>
      )}

      <div className="scene-group">
        <h4>Palette</h4>
        <Palette swatches={a.palette} />
      </div>

      {a.limits.map((l) => (
        <p key={l} className="note">
          {l}
        </p>
      ))}
      <p className="note">
        Rule-based reading in {(a.analysisMs / 1000).toFixed(1)} s. Hover a genre to see why it matched.
      </p>
    </>
  );
}

export function Palette({ swatches, compact = false }: { swatches: Swatch[]; compact?: boolean }) {
  return (
    <div className={`palette${compact ? " is-compact" : ""}`} aria-label="Dominant colours">
      {swatches.map((s) => (
        <span key={s.hex} style={{ background: s.hex, flexGrow: s.weight }} title={`${s.hex} · ${Math.round(s.weight * 100)}%`} />
      ))}
    </div>
  );
}

type SyncState =
  | { kind: "idle" }
  | { kind: "confirm" }
  | { kind: "running" }
  | { kind: "done"; report: SyncReport; targets: string[] }
  | { kind: "undone"; count: number }
  | { kind: "error"; message: string };

interface SyncProps {
  info: ImageInfo;
  adjustments: Adjustments;
  group: ShotGroup | null;
  /** Save the open photo first, so the sync copies what's on screen. */
  flushSave: () => Promise<void>;
  /** Sidecars changed on disk (sync or undo). */
  onChanged: () => void;
}

/** PRD Section 2.2: Story-Arc Batch Sync from the open photo to the rest of its group. */
export function BatchSyncCard({ info, adjustments, group, flushSave, onChanged }: SyncProps) {
  const [open, setOpen] = useState(true);
  const [state, setState] = useState<SyncState>({ kind: "idle" });
  const targets = group ? group.frames.filter((f) => f !== info.path) : [];
  const name = (p: string) => p.split(/[\\/]/).pop() ?? p;

  useEffect(() => setState({ kind: "idle" }), [info.path, group?.id]);

  const run = async () => {
    setState({ kind: "running" });
    try {
      await flushSave();
      const report = await syncLook(info.path, adjustments, targets);
      setState({ kind: "done", report, targets });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    } finally {
      onChanged();
    }
  };
  const undo = async (paths: string[]) => {
    setState({ kind: "running" });
    try {
      const restored = await undoSync(paths);
      setState({ kind: "undone", count: restored.length });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    } finally {
      onChanged();
    }
  };

  return (
    <section className="scene">
      <button type="button" className="step-head" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        <span className="styles-icon" aria-hidden>
          ⇶
        </span>
        <span className="step-title">Story-arc sync</span>
        {group && <span className="badge">{group.frames.length} in group</span>}
        <span className="chevron" aria-hidden>
          {open ? "▾" : "▸"}
        </span>
      </button>
      {open && (
        <div className="step-body scene-body">
          {!group ? (
            <p className="note">Grouping the folder…</p>
          ) : (
            <>
              <div className="scene-group">
                <h4>{group.label}</h4>
                <Palette swatches={group.palette} compact />
                <p className="note">
                  {group.hero === info.path
                    ? "This photo is the group's hero (closest to its average light and colour)."
                    : `Hero: ${name(group.hero)}.`}
                  {group.splitReason && ` New group because: ${group.splitReason}.`}
                </p>
              </div>
              {targets.length === 0 ? (
                <p className="note">This photo is alone in its group, so there is nothing to sync.</p>
              ) : (
                <SyncControls
                  state={state}
                  count={targets.length}
                  onAsk={() => setState({ kind: "confirm" })}
                  onCancel={() => setState({ kind: "idle" })}
                  onRun={() => void run()}
                  onUndo={(paths) => void undo(paths)}
                  name={name}
                />
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}

function SyncControls({
  state,
  count,
  onAsk,
  onCancel,
  onRun,
  onUndo,
  name,
}: {
  state: SyncState;
  count: number;
  onAsk: () => void;
  onCancel: () => void;
  onRun: () => void;
  onUndo: (paths: string[]) => void;
  name: (p: string) => string;
}) {
  const photos = `${count} other photo${count === 1 ? "" : "s"}`;
  switch (state.kind) {
    case "idle":
    case "undone":
      return (
        <>
          <button type="button" className="btn primary-outline" onClick={onAsk}>
            Sync this look to {photos}
          </button>
          {state.kind === "undone" && <p className="note">Restored {state.count} photo{state.count === 1 ? "" : "s"}.</p>}
        </>
      );
    case "confirm":
      return (
        <div className="confirm">
          <p>
            Replace the look on {photos} with this one? Each photo keeps its own white balance, lens and noise
            settings, and gets its own exposure, skin protection and micro-contrast. Their current edits are backed up
            so you can undo.
          </p>
          <div className="confirm-actions">
            <button type="button" className="btn primary" onClick={onRun}>
              Sync {count}
            </button>
            <button type="button" className="btn" onClick={onCancel}>
              Cancel
            </button>
          </div>
        </div>
      );
    case "running":
      return <p className="note">Calibrating each photo…</p>;
    case "error":
      return (
        <>
          <p className="error">{state.message}</p>
          <button type="button" className="btn" onClick={onAsk}>
            Try again
          </button>
        </>
      );
    case "done": {
      const { report } = state;
      return (
        <>
          <ul className="sync-report">
            {report.frames.map((f) => (
              <li key={f.path}>
                <strong>{name(f.path)}</strong>
                <span>
                  {f.exposureDelta >= 0 ? "+" : ""}
                  {f.exposureDelta.toFixed(2)} EV
                  {f.whiteBalanceShifted ? " · WB shift" : ""}
                  {Math.abs(f.textureScale - 1) >= 0.02 ? ` · texture ×${f.textureScale.toFixed(2)}` : ""}
                  {` · skin ${Math.round(f.skinProtection)}`}
                </span>
              </li>
            ))}
            {report.skipped.map((s) => (
              <li key={s.path} className="is-skipped">
                <strong>{name(s.path)}</strong>
                <span>skipped: {s.reason}</span>
              </li>
            ))}
          </ul>
          <button type="button" className="btn" onClick={() => onUndo(state.targets)}>
            Undo sync
          </button>
        </>
      );
    }
  }
}
