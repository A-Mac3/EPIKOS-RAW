import { useEffect, useRef, useState } from "react";
import { listStyles, renderPreview } from "../api";
import type { Adjustments, Preview, StyleInfo } from "../types";

const THUMB_W = 180;
const THUMB_H = 120;
const THUMB_DELAY_MS = 450;

/**
 * The built-in styles plus a small render of the current photo in each. Thumbnails
 * follow the photo's other settings, re-rendered once edits pause.
 */
export function useStyles(path: string | null, adjustments: Adjustments, paused = false) {
  const [styles, setStyles] = useState<StyleInfo[]>([]);
  const [thumbs, setThumbs] = useState<Record<string, Preview>>({});

  useEffect(() => {
    listStyles().then(setStyles, () => setStyles([]));
  }, []);

  // Everything except the style itself; a stable key so style-only edits don't re-render.
  const { style: _style, ...rest } = adjustments;
  const baseKey = JSON.stringify(rest);
  const latest = useRef(0);

  useEffect(() => {
    setThumbs({});
  }, [path]);

  useEffect(() => {
    // Mid-drag, thumbnails would compete with the main preview: they wait (and any
    // batch in progress stops) until the drag ends.
    const token = ++latest.current;
    if (!path || styles.length === 0 || paused) return;
    const base = JSON.parse(baseKey) as Omit<Adjustments, "style">;
    const timer = window.setTimeout(async () => {
      // Listed styles only, in the order the library shows them.
      for (const s of styles.filter((x) => x.listed)) {
        if (token !== latest.current) return;
        const adj: Adjustments = { ...base, style: { id: s.id, amount: 100, skinProtection: s.skinProtection, blend: [] } };
        try {
          const p = await renderPreview(path, adj, THUMB_W, THUMB_H);
          if (token === latest.current) setThumbs((t) => ({ ...t, [s.id]: p }));
        } catch {
          // A missing thumbnail just shows the swatch.
        }
      }
    }, THUMB_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [path, baseKey, styles, paused]);

  return { styles, thumbs };
}
