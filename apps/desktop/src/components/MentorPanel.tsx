import { useEffect, useRef, useState } from "react";
import { critique, mentor } from "../api";
import type { Adjustments, Feedback, ImageInfo, MentorReport } from "../types";

type Update = (fn: (a: Adjustments) => Adjustments) => void;

/** Let the first preview render before the (≈3 s) reading competes for the engine. */
const START_DELAY_MS = 900;
/** Feedback follows edits once they pause. */
const FEEDBACK_DELAY_MS = 800;

// One reading per photo per session (it describes the photo, which edits don't change),
// with the settings it was made for.
const cache = new Map<string, { basis: string; report: MentorReport }>();

type State = { kind: "loading" } | { kind: "done"; report: MentorReport; basis: string } | { kind: "error"; message: string };

/**
 * The AI Photography Mentor: a rule-based reading of the photo (histogram, dynamic
 * range, colour cast, light, detected subjects) explaining how and why to edit it, a
 * recommended starting point, and live feedback on the current edit.
 */
export function MentorPanel({ info, adjustments, commit }: { info: ImageInfo; adjustments: Adjustments; commit: Update }) {
  const [state, setState] = useState<State>({ kind: "loading" });
  const [feedback, setFeedback] = useState<Feedback[] | null>(null);
  const [applying, setApplying] = useState(false);
  const latest = useRef(adjustments);
  latest.current = adjustments;

  const read = async (adj: Adjustments) => {
    const basis = JSON.stringify(adj);
    const report = await mentor(info.path, adj);
    cache.set(info.path, { basis, report });
    return { report, basis };
  };

  useEffect(() => {
    let alive = true;
    const hit = cache.get(info.path);
    if (hit) {
      setState({ kind: "done", ...hit });
      return;
    }
    setState({ kind: "loading" });
    const t = window.setTimeout(() => {
      read(latest.current).then(
        (r) => alive && setState({ kind: "done", ...r }),
        (e) => alive && setState({ kind: "error", message: String(e) }),
      );
    }, START_DELAY_MS);
    return () => {
      alive = false;
      window.clearTimeout(t);
    };
    // Re-read only when another photo opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [info.path]);

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
      const current = JSON.stringify(latest.current);
      const r = current === state.basis ? state : { kind: "done" as const, ...(await read(latest.current)) };
      setState(r);
      commit(() => r.report.recommended);
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    } finally {
      setApplying(false);
    }
  };

  return (
    <div className="mentor">
      {state.kind === "loading" && <p className="note">Reading the photo: histogram, dynamic range, colour, light and subjects…</p>}
      {state.kind === "error" && <p className="error">{state.message}</p>}
      {state.kind === "done" && (
        <>
          <p className="mentor-summary">{state.report.summary}</p>
          {state.report.insights.length === 0 ? (
            <p className="note">Well exposed, level and neutral: a good base. Try a style from Presets & Styles.</p>
          ) : (
            <ol className="insights">
              {state.report.insights.map((i, k) => (
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
          {state.report.changes.length > 0 && (
            <div className="mentor-apply">
              <button type="button" className="btn primary" disabled={applying} onClick={() => void apply()}>
                {applying ? "Applying…" : "Apply Recommended Starting Point"}
              </button>
              <p className="hint">{state.report.changes.join(" · ")}</p>
            </div>
          )}
          <button
            type="button"
            className="btn link"
            onClick={() => {
              cache.delete(info.path);
              setState({ kind: "loading" });
              read(latest.current).then(
                (r) => setState({ kind: "done", ...r }),
                (e) => setState({ kind: "error", message: String(e) }),
              );
            }}
          >
            Read again
          </button>
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
