/** AI look settings (not secret: the key itself lives in the macOS Keychain). */
export type AiProvider = "off" | "anthropic" | "openai";

export interface AiSettings {
  provider: AiProvider;
  model: string;
}

const KEY = "epikos.ai";
export const DEFAULT_MODELS: Record<Exclude<AiProvider, "off">, string> = {
  anthropic: "claude-sonnet-5",
  openai: "gpt-4o",
};

export function loadAiSettings(): AiSettings {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    if (v && ["off", "anthropic", "openai"].includes(v.provider)) return { provider: v.provider, model: String(v.model ?? "") };
  } catch {
    // Fall through to the default.
  }
  return { provider: "off", model: "" };
}

export function storeAiSettings(s: AiSettings) {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    // A convenience only.
  }
}
