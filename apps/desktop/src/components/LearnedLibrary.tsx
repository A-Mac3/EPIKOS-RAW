import { useState } from "react";
import { deleteLearnedStyle, deletePreset, learnStyle, mentor, openImage, pickReference, savePreset } from "../api";
import type { Adjustments, ImageInfo, LearnedStyle, Preset } from "../types";
import { Palette } from "./ScenePanel";

type Update = (fn: (a: Adjustments) => Adjustments) => void;

interface Props {
  info: ImageInfo;
  adjustments: Adjustments;
  commit: Update;
  learned: LearnedStyle[];
  presets: Preset[];
  /** The stores changed on disk. */
  onChanged: () => void;
  /** The AI Mentor's starting target: a learned style's id, or null for Editorial. */
  mentorTarget: string | null;
  setMentorTarget: (id: string | null) => void;
}

type Busy = null | "learn-file" | "learn-edit" | "save" | { apply: string };

/** A preset's look on top of this photo's own framing, white balance and exposure. */
export function applyPreset(p: Preset): (a: Adjustments) => Adjustments {
  const l = p.adjustments;
  return (a) => ({
    ...a,
    tone: l.tone,
    local: l.local,
    texture: l.texture,
    color: l.color,
    // Placed lights belong to this photo.
    atmosphere: { ...l.atmosphere, lights: a.atmosphere.lights },
    curves: l.curves,
    splitToning: l.splitToning,
    finishing: l.finishing,
    style: l.style,
    lut: l.lut,
  });
}

/**
 * AI Style Learning and custom presets. Learned styles are measured from a reference
 * photo (tone transfer, skin, colour) and can steer the AI Mentor's starting point;
 * presets save the look of an edit. Both live in ~/.epikos and persist across sessions.
 */
export function LearnedLibrary({
  info,
  adjustments,
  commit,
  learned,
  presets,
  onChanged,
  mentorTarget,
  setMentorTarget,
}: Props) {
  const [name, setName] = useState("");
  const [presetName, setPresetName] = useState("");
  const [busy, setBusy] = useState<Busy>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const run = async (b: Busy, f: () => Promise<void>) => {
    setBusy(b);
    setError(null);
    setNote(null);
    try {
      await f();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const learnFromFile = () =>
    run("learn-file", async () => {
      const path = await pickReference();
      if (!path) return;
      // The reference as its own sidecar edits it (a finished JPEG has none).
      const ref = await openImage(path);
      const s = await learnStyle(path, ref.document.adjustments, name);
      setName("");
      onChanged();
      setMentorTarget(s.id);
      setNote(`Learned “${s.name}” from ${s.source}; the AI Mentor now aims at it.`);
    });

  const learnFromEdit = () =>
    run("learn-edit", async () => {
      const s = await learnStyle(info.path, adjustments, name || `${info.name.replace(/\.[^.]+$/, "")} look`);
      setName("");
      onChanged();
      setNote(`Learned “${s.name}” from this edit.`);
    });

  const applyLearned = (s: LearnedStyle) =>
    run({ apply: s.id }, async () => {
      const r = await mentor(info.path, adjustments, s.id);
      commit(() => r.recommended);
      setMentorTarget(s.id);
      setNote(`Matched to “${s.name}”: ${r.changes.slice(-4).join(" · ") || "already close"}.`);
    });

  return (
    <div className="learned">
      <div className="library-group">
        <div className="library-head is-static">
          <span>Learned Styles</span>
          <span className="library-count">{learned.length}</span>
        </div>
        <div className="learn-form">
          <input
            type="text"
            value={name}
            placeholder="Style name (optional)"
            aria-label="Name for the learned style"
            onChange={(e) => setName(e.currentTarget.value)}
          />
          <div className="learn-actions">
            <button type="button" className="btn primary" disabled={busy !== null} onClick={() => void learnFromFile()}>
              {busy === "learn-file" ? "Learning…" : "Learn Style from Photo…"}
            </button>
            <button
              type="button"
              className="btn"
              disabled={busy !== null}
              title="Learn the look of the photo as you've edited it"
              onClick={() => void learnFromEdit()}
            >
              {busy === "learn-edit" ? "Learning…" : "From this edit"}
            </button>
          </div>
        </div>
        <div className="learned-list" role="radiogroup" aria-label="AI Mentor starting target">
          <label className={`learned-row${mentorTarget === null ? " is-target" : ""}`}>
            <input type="radio" name="mentor-target" checked={mentorTarget === null} onChange={() => setMentorTarget(null)} />
            <span className="learned-body">
              <span className="learned-name">Editorial (built-in)</span>
              <span className="hint">Rich blacks, dimensional skin, olive foliage, S-curve</span>
            </span>
          </label>
          {learned.map((s) => {
            const busyHere = typeof busy === "object" && busy?.apply === s.id;
            return (
              <div key={s.id} className={`learned-row${mentorTarget === s.id ? " is-target" : ""}`}>
                <input
                  type="radio"
                  name="mentor-target"
                  aria-label={`Use ${s.name} as the AI Mentor's target`}
                  checked={mentorTarget === s.id}
                  onChange={() => setMentorTarget(s.id)}
                />
                <span className="learned-body">
                  <span className="learned-name" title={`From ${s.source}`}>
                    {s.name}
                  </span>
                  <Palette swatches={s.palette} compact />
                  <span className="hint">{describe(s)}</span>
                </span>
                <span className="learned-actions">
                  <button type="button" className="btn small" disabled={busy !== null} onClick={() => void applyLearned(s)}>
                    {busyHere ? "Matching…" : "Apply"}
                  </button>
                  <button
                    type="button"
                    className="btn icon"
                    aria-label={`Delete ${s.name}`}
                    title="Delete this learned style"
                    onClick={() =>
                      void run(null, async () => {
                        await deleteLearnedStyle(s.id);
                        if (mentorTarget === s.id) setMentorTarget(null);
                        onChanged();
                      })
                    }
                  >
                    ×
                  </button>
                </span>
              </div>
            );
          })}
        </div>
        <p className="note">
          The selected look is the AI Mentor&apos;s starting target. Apply matches this photo to it now: tone, skin richness
          for this skin&apos;s own depth, and colour. Kept in ~/.epikos/learned_styles.json.
        </p>
      </div>

      <div className="library-group">
        <div className="library-head is-static">
          <span>My Presets</span>
          <span className="library-count">{presets.length}</span>
        </div>
        <div className="learn-form">
          <input
            type="text"
            value={presetName}
            placeholder="Preset name"
            aria-label="Name for the preset"
            onChange={(e) => setPresetName(e.currentTarget.value)}
          />
          <button
            type="button"
            className="btn"
            disabled={busy !== null}
            onClick={() =>
              void run("save", async () => {
                await savePreset(presetName, adjustments);
                setPresetName("");
                onChanged();
              })
            }
          >
            {busy === "save" ? "Saving…" : "Save current look"}
          </button>
        </div>
        {presets.length > 0 && (
          <div className="lut-list">
            {presets.map((p) => (
              <div key={p.id} className="lut-row">
                <button type="button" className="lut-apply" onClick={() => commit(applyPreset(p))} title="Apply this look">
                  <span className="lut-name">{p.name}</span>
                </button>
                <button
                  type="button"
                  className="btn icon"
                  aria-label={`Delete ${p.name}`}
                  onClick={() =>
                    void run(null, async () => {
                      await deletePreset(p.id);
                      onChanged();
                    })
                  }
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}
        <p className="note">A preset keeps the look, not the photo&apos;s framing, white balance or exposure. Kept in ~/.epikos/presets.json.</p>
      </div>
      {note && <p className="hint">{note}</p>}
      {error && <p className="hint error">{error}</p>}
    </div>
  );
}

function describe(s: LearnedStyle): string {
  const t = s.signature.tone;
  const parts = [
    t.black < 0.14 ? "deep blacks" : t.black > 0.22 ? "lifted blacks" : "natural blacks",
    t.contrast > 0.27 ? "punchy mids" : t.contrast < 0.2 ? "soft mids" : "balanced mids",
  ];
  const k = s.signature.skin;
  if (k) parts.push(k.richness > 0.6 ? "rich skin" : k.richness < 0.3 ? "muted skin" : "natural skin");
  return parts.join(" · ");
}
