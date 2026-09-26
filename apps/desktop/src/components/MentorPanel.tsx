import { useEffect, useRef, useState } from "react";
import { critique, mentor } from "../api";
import type { Adjustments, Feedback, ImageInfo, LearnedStyle, MentorReport } from "../types";

type Update = (fn: (a: Adjustments) => Adjustments) => void;

/** Let the first preview render before the reading competes for the engine. */
const START_DELAY_MS = 900;
/** The reading follows the edit once it pauses (slider drags change it constantly). */
const REREAD_DELAY_MS = 1200;
/** Feedback follows edits once they pause. */
const FEEDBACK_DELAY_MS = 800;

// The latest reading per photo this session, with the settings it was made for, so a
// photo reopens with its reading while a fresh one is made.
const cache = new Map<string, { basis: string; report: MentorReport }>();

type State =
  | { kind: "loading" }
  | { kind: "done"; report: MentorReport; basis: string }
  | { kind: "error"; message: string };

/**
 * The AI Photography Mentor: a rule-based reading of the photo and of the current edit
 * (histogram, dynamic range, colour cast, light, skin, subject separation,
 * composition), explaining how and why to edit it, with a recommended starting point
 * (global and local) and a suggested crop. It re-reads the edit whenever it pauses.
 */
export function MentorPanel({
  info,
  adjustments,
  commit,
  target,
  targets,
  setTarget,
}: {
  info: ImageInfo;
  adjustments: Adjustments;
  commit: Update;
  /** Learned style id the starting point aims at; null for Editorial. */
  target: string | null;
  targets: LearnedStyle[];
  setTarget: (id: string | null) => void;
}) {
  const [state, setState] = useState<State>(() => {
    const hit = cache.get(info.path);
    return hit ? { kind: "done", ...hit } : { kind: "loading" };
  });
  const [updating, setUpdating] = useState(false);
  const [feedback, setFeedback] = useState<Feedback[] | null>(null);
  const [applying, setApplying] = useState(false);
  const latest = useRef(adjustments);
  latest.current = adjustments;
  // A reading belongs to the edit and the target it aimed at.
  const basis = JSON.stringify([adjustments, target]);
  const latestTarget = useRef(target);
  latestTarget.current = target;
  // Only the newest request may update the panel.
  const request = useRef(0);

  const read = async (adj: Adjustments) => {
    const t = latestTarget.current;
    const b = JSON.stringify([adj, t]);
    const report = await mentor(info.path, adj, t);
    cache.set(info.path, { basis: b, report });
    return { report, basis: b };
  };

  // Another photo: show its last reading (if any) until the new one arrives.
  useEffect(() => {
    const hit = cache.get(info.path);
    setState(hit ? { kind: "done", ...hit } : { kind: "loading" });
  }, [info.path]);

  // Live loop: read the current edit whenever it has settled and differs from what
  // the shown reading was made for.
  useEffect(() => {
    const shown = cache.get(info.path);
    if (shown?.basis === basis) return;
    const id = ++request.current;
    const t = window.setTimeout(
      () => {
        setUpdating(true);
        read(latest.current).then(
          (r) => {
            if (id === request.current) setState({ kind: "done", ...r });
          },
          (e) => {
            if (id === request.current && !cache.has(info.path)) setState({ kind: "error", message: String(e) });
          },
        ).finally(() => {
          if (id === request.current) setUpdating(false);
        });
      },
      shown ? REREAD_DELAY_MS : START_DELAY_MS,
    );
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [info.path, basis]);

  useEffect(() => {
    let alive = true;
    const t = window.setTimeout(() => {
      critique(info.path, adjustments).then(
        (f) => alive && setFeedback(f),
        () => alive && setFeedback(null),
      );
    }, FEEDBACK_DELAY_MS);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
  }, [info.path, adjustments]);

  const apply = async () => {
    if (state.kind !== "done") return;
    setApplying(true);
    try {
      // Suggestions are made against the settings they were read for; after other
      // edits, read again so none of those edits is lost.
      const current = JSON.stringify([latest.current, latestTarget.current]);
      const r = current === state.basis ? state : { kind: "done" as const, ...(await read(latest.current)) };
      setState(r);
      commit(() => r.report.recommended);
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    } finally {
      setApplying(false);
    }
  };

  const report = state.kind === "done" ? state.report : null;
  const stale = state.kind === "done" && state.basis !== basis;
  return (
    <div className="mentor">
      <p className="mentor-live" aria-live="polite">
        <span className={`mentor-dot${updating || stale ? " is-busy" : ""}`} aria-hidden />
        {updating ? "Re-reading your edit…" : stale ? "Waiting for the edit to settle…" : "Live: follows every edit"}
      </p>
      <label className="mentor-target">
        <span>Starting target</span>
        <select value={target ?? ""} onChange={(e) => setTarget(e.currentTarget.value || null)}>
          <option value="">Editorial (built-in)</option>
          {targets.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name} (learned)
            </option>
          ))}
        </select>
      </label>
      {state.kind === "loading" && <p className="note">Reading the photo: histogram, dynamic range, colour, light, skin and subjects…</p>}
      {state.kind === "error" && <p className="error">{state.message}</p>}
      {report && (
        <>
          <p className="mentor-summary">{report.summary}</p>
          {report.insights.length === 0 ? (
            <p className="note">Well exposed, level and neutral: a good base. Try a style from Presets & Styles.</p>
          ) : (
            <ol className="insights">
              {report.insights.map((i, k) => (
                <li key={k}>
                  <span className="insight-topic">{i.topic}</span>
                  <p>{i.observation}</p>
                  <p className="insight-why">
                    <strong>Why:</strong> {i.why}
                  </p>
                  <p className="insight-how">
                    <strong>How:</strong> {i.how}
                  </p>
                </li>
              ))}
            </ol>
          )}
          {report.changes.length > 0 && (
            <div className="mentor-apply">
              <button type="button" className="btn primary" disabled={applying} onClick={() => void apply()}>
                {applying ? "Applying…" : "Apply Recommended Starting Point"}
              </button>
              <p className="hint">{report.changes.join(" · ")}</p>
            </div>
          )}
          {report.crop && (
            <div className="mentor-apply">
              <button
                type="button"
                className="btn primary-outline"
                onClick={() => {
                  const c = report.crop!;
                  commit((a) => ({ ...a, crop: c.crop, lens: { ...a.lens, rotation: c.rotation } }));
                }}
              >
                Apply suggested crop
              </button>
              <p className="hint">
                {report.crop.reason}
                {Math.abs(report.crop.rotation) >= 0.05 ? `, straightened ${report.crop.rotation > 0 ? "+" : ""}${report.crop.rotation.toFixed(1)}°` : ""}.
                Fine-tune with Crop & straighten (C).
              </p>
            </div>
          )}
        </>
      )}

      <div className="feedback-card">
        <h4>Edit feedback</h4>
        {feedback === null ? (
          <p className="note">Checking the current edit…</p>
        ) : (
          <ul>
            {feedback.map((f, k) => (
              <li key={k} className={`is-${f.level}`}>
                <span aria-hidden>{f.level === "praise" ? "✓" : f.level === "warning" ? "!" : "→"}</span>
                <span>{f.text}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <p className="note">
        Rule-based: every point comes from measurements of this photo (and your edit), not from a language model.
      </p>
    </div>
  );
}
