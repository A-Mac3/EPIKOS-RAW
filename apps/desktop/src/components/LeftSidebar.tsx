import { useState, type ReactNode } from "react";

export type SidebarTab = "styles" | "mentor" | "history";

const TABS: { id: SidebarTab; label: string }[] = [
  { id: "styles", label: "Presets & Styles" },
  { id: "mentor", label: "AI Mentor" },
  { id: "history", label: "History" },
];
const TAB_KEY = "epikos.sidebar.tab";

function storedTab(): SidebarTab {
  try {
    const v = localStorage.getItem(TAB_KEY);
    return TABS.some((t) => t.id === v) ? (v as SidebarTab) : "styles";
  } catch {
    return "styles";
  }
}

/** Lightroom-style left panel: Presets & Styles, AI Mentor and History. */
export function LeftSidebar({ content }: { content: Record<SidebarTab, ReactNode> }) {
  const [tab, setTab] = useState<SidebarTab>(storedTab);
  const choose = (t: SidebarTab) => {
    setTab(t);
    try {
      localStorage.setItem(TAB_KEY, t);
    } catch {
      // Remembering the tab is a convenience only.
    }
  };
  return (
    <aside className="left-panel" aria-label="Library, mentor and history">
      <div className="left-tabs" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={tab === t.id}
            className={`left-tab${tab === t.id ? " is-active" : ""}`}
            onClick={() => choose(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="left-scroll" role="tabpanel">
        {content[tab]}
      </div>
    </aside>
  );
}

/** Every step since the photo was opened; click one to go back (or forward) to it. */
export function HistoryPanel({ steps, index, goTo }: { steps: string[]; index: number; goTo: (i: number) => void }) {
  return (
    <>
    <ol className="history" reversed>
      {steps
        .map((label, i) => ({ label, i }))
        .reverse()
        .map(({ label, i }) => (
          <li key={i}>
            <button
              type="button"
              className={`history-step${i === index ? " is-current" : ""}${i > index ? " is-undone" : ""}`}
              aria-current={i === index ? "step" : undefined}
              onClick={() => goTo(i)}
            >
              <span className="history-n">{i}</span>
              <span>{label}</span>
            </button>
          </li>
        ))}
    </ol>
    {steps.length === 1 && <p className="note">Edits appear here as you make them; click one to return to it.</p>}
    </>
  );
}
