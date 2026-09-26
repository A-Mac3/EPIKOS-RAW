import { useEffect, useRef, useState } from "react";
import { deleteAiKey, hasAiKey, saveAiKey } from "../api";
import { DEFAULT_MODELS, type AiProvider, type AiSettings } from "../settings";
import { Segmented } from "./Slider";

interface Props {
  settings: AiSettings;
  onChange: (s: AiSettings) => void;
  onClose: () => void;
}

/**
 * Settings: optional AI interpretation of "Describe a look" with your own Anthropic or
 * OpenAI key. The key is saved in the macOS Keychain and never shown again; only the
 * prompt text is sent, never the photo. Off by default: the built-in interpreter
 * needs no account and works offline.
 */
export function SettingsDialog({ settings, onChange, onClose }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [key, setKey] = useState("");
  const [saved, setSaved] = useState<boolean | null>(null);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const provider = settings.provider;

  useEffect(() => {
    dialog.current?.showModal();
  }, []);
  useEffect(() => {
    setKey("");
    setStatus(null);
    if (provider === "off") {
      setSaved(null);
      return;
    }
    hasAiKey(provider).then(setSaved, () => setSaved(false));
  }, [provider]);

  const save = async () => {
    if (provider === "off") return;
    try {
      await saveAiKey(provider, key);
      setKey("");
      setSaved(true);
      setStatus({ ok: true, text: "Saved in the macOS Keychain." });
    } catch (e) {
      setStatus({ ok: false, text: String(e) });
    }
  };
  const remove = async () => {
    if (provider === "off") return;
    try {
      await deleteAiKey(provider);
      setSaved(false);
      setStatus({ ok: true, text: "Removed from the Keychain." });
    } catch (e) {
      setStatus({ ok: false, text: String(e) });
    }
  };

  return (
    <dialog ref={dialog} className="modal" onCancel={onClose}>
      <h2>Settings</h2>
      <p className="modal-sub">Describe a Look</p>

      <div className="field">
        <span className="field-label">Interpreter</span>
        <Segmented<AiProvider>
          label="Interpreter"
          value={provider}
          options={[
            { value: "off", label: "Built-in" },
            { value: "anthropic", label: "Anthropic" },
            { value: "openai", label: "OpenAI" },
          ]}
          onChange={(p) => onChange({ provider: p, model: "" })}
        />
        <span className="hint">
          {provider === "off"
            ? "The built-in interpreter works offline: moods, light, eras, film, tone-curve and colour phrases."
            : "Your prompt text (never the photo) is sent to this provider with your own key. The built-in reading still places lights and runs first; the model's changes are bounded and never touch framing."}
        </span>
      </div>

      {provider !== "off" && (
        <>
          <label className="field">
            <span className="field-label">Model</span>
            <input
              type="text"
              value={settings.model}
              placeholder={DEFAULT_MODELS[provider]}
              onChange={(e) => onChange({ ...settings, model: e.currentTarget.value })}
            />
          </label>
          <div className="field">
            <span className="field-label">API key</span>
            {saved ? (
              <div className="field-row">
                <span className="key-saved">
                  <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden>
                    <path d="M2.5 6.3l2.3 2.3 4.7-5" fill="none" stroke="currentColor" strokeWidth="1.5" />
                  </svg>
                  Saved in the Keychain
                </span>
                <button type="button" className="btn small" onClick={() => void remove()}>
                  Remove key
                </button>
              </div>
            ) : (
              <form
                className="field-row"
                onSubmit={(e) => {
                  e.preventDefault();
                  void save();
                }}
              >
                <input
                  type="password"
                  value={key}
                  autoComplete="off"
                  spellCheck={false}
                  placeholder={provider === "anthropic" ? "sk-ant-…" : "sk-…"}
                  aria-label="API key"
                  onChange={(e) => setKey(e.currentTarget.value)}
                />
                <button type="submit" className="btn primary" disabled={!key.trim()}>
                  Save to Keychain
                </button>
              </form>
            )}
            <span className="hint">Stored only in your macOS Keychain; EPIKOS RAW never shows it again or writes it to disk.</span>
          </div>
        </>
      )}
      {status && <p className={`modal-status ${status.ok ? "is-ok" : "is-error"}`}>{status.text}</p>}
      <div className="modal-actions">
        <button type="button" className="btn primary" onClick={onClose}>
          Done
        </button>
      </div>
    </dialog>
  );
}
