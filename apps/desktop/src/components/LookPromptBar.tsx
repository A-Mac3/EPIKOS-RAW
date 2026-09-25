import { forwardRef, useState } from "react";
import { interpretLook } from "../api";
import type { Adjustments, LookPrompt } from "../types";

interface Props {
  path: string;
  adjustments: Adjustments;
  /** One undo step. */
  commit: (fn: (a: Adjustments) => Adjustments) => void;
}

const EXAMPLE = "An eerie, foggy 1970s Scandinavian film scene with subtle golden light on the face";

/**
 * PRD Section 4 "Natural Language Look Prompting": a bar across the top of the viewer.
 * Type a look and press Enter; it becomes real slider, curve, wheel and light
 * settings as one undo step, and a strip over the photo shows what was understood.
 */
export const LookPromptBar = forwardRef<HTMLInputElement, Props>(function LookPromptBar(
  { path, adjustments, commit },
  ref,
) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<LookPrompt | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    const prompt = text.trim();
    if (!prompt || busy) return;
    setBusy(true);
    setError(null);
    try {
      const r = await interpretLook(path, prompt, adjustments);
      setResult(r);
      if (r.matched.length > 0) commit(() => r.adjustments);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="prompt-bar">
      <form
        className="prompt-bar-input"
        onSubmit={(e) => {
          e.preventDefault();
          void run();
        }}
      >
        <span className="prompt-bar-icon" aria-hidden>
          ✦
        </span>
        <input
          ref={ref}
          type="text"
          value={text}
          aria-label="Describe a look"
          placeholder={`Describe a look… e.g. “${EXAMPLE}”`}
          onChange={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setResult(null);
              e.currentTarget.blur();
            }
          }}
        />
        {!text && (
          <button type="button" className="btn link" onClick={() => setText(EXAMPLE)}>
            Example
          </button>
        )}
        <button type="submit" className="btn" disabled={busy || !text.trim()}>
          {busy ? "Applying…" : "Apply"}
        </button>
        <kbd title="Focus this bar">⌘K</kbd>
      </form>
      {(result || error) && (
        <div className="prompt-bar-result" role="status">
          {error ? (
            <span className="error">{error}</span>
          ) : result && result.matched.length === 0 ? (
            <span className="hint">Nothing recognised, nothing changed. Try moods, weather, light, eras or colours.</span>
          ) : (
            <ul className="chips">
              {result?.matched.map((m, i) => (
                <li key={i} title={m.effect} className={m.strength < 0 ? "is-negated" : ""}>
                  <strong>{m.phrase}</strong>
                  <span>{m.strength < 0 ? `removed: ${m.effect}` : m.effect}</span>
                </li>
              ))}
            </ul>
          )}
          {result && result.unknown.length > 0 && (
            <span className="hint">Not understood: {result.unknown.join(", ")}</span>
          )}
          {result && result.matched.length > 0 && <span className="hint">One step: ⌘Z undoes it.</span>}
          <button type="button" className="btn icon" aria-label="Dismiss" onClick={() => setResult(null)}>
            ×
          </button>
        </div>
      )}
    </div>
  );
});
