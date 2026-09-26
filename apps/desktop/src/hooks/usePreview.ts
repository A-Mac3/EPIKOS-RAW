import { useEffect, useRef, useState } from "react";
import { renderPreview } from "../api";
import type { Adjustments, Preview } from "../types";

interface Request {
  path: string;
  adjustments: Adjustments;
  width: number;
  height: number;
}

/**
 * Renders previews without queueing: while one render is in flight, newer requests
 * replace each other and only the latest is rendered next. Slider drags therefore
 * never fall behind the pointer.
 */
export function usePreview(
  path: string | null,
  adjustments: Adjustments | null,
  width: number,
  height: number,
) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const latest = useRef<Request | null>(null);
  const inFlight = useRef(false);
  const currentPath = useRef(path);
  currentPath.current = path;

  useEffect(() => {
    if (!path || !adjustments || width < 1 || height < 1) return;
    latest.current = { path, adjustments, width, height };
    if (inFlight.current) return;

    const pump = async () => {
      inFlight.current = true;
      setBusy(true);
      while (latest.current) {
        const req = latest.current;
        latest.current = null;
        // Show every finished frame (it's still newer than what's on screen), but never
        // one that belongs to a previously opened image.
        try {
          const p = await renderPreview(req.path, req.adjustments, req.width, req.height);
          if (req.path === currentPath.current) {
            setPreview({ ...p, geometry: { rotation: req.adjustments.lens.rotation, vertical: req.adjustments.lens.vertical } });
            setError(null);
          }
        } catch (e) {
          if (req.path === currentPath.current) setError(String(e));
        }
      }
      inFlight.current = false;
      setBusy(false);
    };
    void pump();
  }, [path, adjustments, width, height]);

  // Drop the old image immediately when switching files.
  useEffect(() => {
    setPreview(null);
    setError(null);
  }, [path]);

  return { preview, error, busy };
}

/** Geometry sliders re-render only once they pause this long (or are released). */
const GEOMETRY_SETTLE_MS = 150;

const geometryOf = (a: Adjustments) => `${a.lens.rotation}|${a.lens.vertical}`;

/**
 * `adjustments`, except that while a control is dragged, a change of straighten or
 * perspective reaches the engine only once the drag pauses for GEOMETRY_SETTLE_MS (or
 * ends): the viewer shows those changes instantly with a CSS transform meanwhile, and
 * the engine renders the exact result once.
 */
export function useGeometrySettled(adjustments: Adjustments, dragging: boolean): Adjustments {
  const [settled, setSettled] = useState(adjustments);
  useEffect(() => {
    if (dragging && geometryOf(adjustments) !== geometryOf(settled)) {
      const t = window.setTimeout(() => setSettled(adjustments), GEOMETRY_SETTLE_MS);
      return () => window.clearTimeout(t);
    }
    setSettled(adjustments);
    // `settled` is read for the comparison only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [adjustments, dragging]);
  return settled;
}
