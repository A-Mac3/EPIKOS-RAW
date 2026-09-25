import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

export type LightKind = "key" | "rim" | "sun";

const KINDS: { kind: LightKind; label: string; hint: string }[] = [
  { kind: "key", label: "Key", hint: "Lights what you drop it on, from just in front" },
  { kind: "rim", label: "Rim", hint: "Sits just behind what you drop it on and outlines it" },
  { kind: "sun", label: "Sun", hint: "Far behind everything: glow and backlight" },
];

interface Props {
  /** Drop at `x`, `y` (client pixels); the viewer decides whether that's on the photo. */
  onDrop: (kind: LightKind, clientX: number, clientY: number) => void;
  /** The pointer reached the palette or a drag started: time to fetch the depth map. */
  onPrepare?: () => void;
  disabled?: boolean;
}

/**
 * PRD Section 4 "3D Atmospheric Light Sculptor": drag a light from this palette onto the
 * photo. Uses pointer events rather than HTML drag-and-drop, which WebKit's
 * embedded web view handles unreliably.
 */
export function LightPalette({ onDrop, onPrepare, disabled }: Props) {
  const [drag, setDrag] = useState<{ kind: LightKind; x: number; y: number } | null>(null);
  const onDropRef = useRef(onDrop);
  onDropRef.current = onDrop;
  const cleanup = useRef<(() => void) | null>(null);
  useEffect(() => () => cleanup.current?.(), []);

  const start = (kind: LightKind) => (e: ReactPointerEvent) => {
    if (disabled) return;
    e.preventDefault();
    onPrepare?.();
    setDrag({ kind, x: e.clientX, y: e.clientY });
    // Listeners go on synchronously: a quick click-and-release must still end the drag.
    const move = (ev: PointerEvent) => setDrag({ kind, x: ev.clientX, y: ev.clientY });
    const up = (ev: PointerEvent) => {
      end();
      onDropRef.current(kind, ev.clientX, ev.clientY);
    };
    const key = (ev: KeyboardEvent) => ev.key === "Escape" && end();
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("keydown", key);
      cleanup.current = null;
      setDrag(null);
    };
    cleanup.current?.();
    cleanup.current = end;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("keydown", key);
  };

  return (
    <>
      <div
        className="light-palette"
        role="toolbar"
        aria-label="Drag a light onto the photo"
        onPointerEnter={() => onPrepare?.()}
      >
        <span className="light-palette-title">Lights</span>
        {KINDS.map(({ kind, label, hint }) => (
          <button
            key={kind}
            type="button"
            className={`light-source is-${kind}`}
            title={`${label} light: drag onto the photo. ${hint}.`}
            disabled={disabled}
            onPointerDown={start(kind)}
          >
            <span className="light-source-dot" aria-hidden />
            {label}
          </button>
        ))}
      </div>
      {drag && (
        <div className={`light-ghost is-${drag.kind}`} style={{ left: drag.x, top: drag.y }} aria-hidden />
      )}
    </>
  );
}
