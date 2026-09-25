import { useState } from "react";
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
 * PRD Section 4 "Natural Language Look Prompting": type a look, get real slider,
 * curve, wheel and light settings (one undo step), and see exactly what was understood.
 */
export function LookPromptBox({ path, adjustments, commit }: Props) {
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
    <div className="look-prompt">
      <span className="field-label">Describe a look</span>
      <textarea
        rows={2}
        value={text}
        placeholder={EXAMPLE}
        onChange={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void run();
          }
        }}
      />
      <div className="look-prompt-actions">
        <button type="button" className="btn" disabled={busy || !text.trim()} onClick={() => void run()}>
          {busy ? "Applying…" : "Apply look"}
        </button>
        {!text && (
          <button type="button" className="btn link" onClick={() => setText(EXAMPLE)}>
            Try the example
          </button>
        )}
      </div>
      {error && <p className="hint error">{error}</p>}
      {result && (
        <div className="look-prompt-result">
          {result.matched.length === 0 ? (
            <p className="hint">Nothing recognised; nothing changed. Try moods, weather, light, eras or colours.</p>
          ) : (
            <ul className="chips">
              {result.matched.map((m, i) => (
                <li key={i} title={m.effect} className={m.strength < 0 ? "is-negated" : ""}>
                  <strong>{m.phrase}</strong>
                  <span>{m.strength < 0 ? `removed: ${m.effect}` : m.effect}</span>
                </li>
              ))}
            </ul>
          )}
          {result.unknown.length > 0 && (
            <p className="hint">Not understood: {result.unknown.join(", ")}</p>
          )}
          {result.matched.length > 0 && <p className="note">Applied as one step: ⌘Z undoes it. Every slider it moved stays editable.</p>}
        </div>
      )}
    </div>
  );
}
