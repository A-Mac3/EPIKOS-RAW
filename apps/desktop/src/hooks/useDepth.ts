import { useEffect, useRef, useState } from "react";
import { detectDepth } from "../api";
import type { Adjustments, Mask } from "../types";

/**
 * Step 6 depth for the UI. The engine estimates depth by itself whenever fog, shafts
 * or virtual lights need it; this fetches it for display while "Show depth" is on or
 * while the 3D light sculptor needs the scene's relief, again when the lens geometry
 * changes.
 */
export function useDepth(path: string | null, adjustments: Adjustments, needed = false) {
  const [show, setShow] = useState(false);
  const [depth, setDepth] = useState<Mask | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const geometry = JSON.stringify(adjustments.lens);
  // `needed`: e.g. while placing a light, so the first click lands on the surface.
  const want = show || needed || adjustments.atmosphere.lights.length > 0;
  const latest = useRef(adjustments);
  latest.current = adjustments;

  useEffect(() => {
    setDepth(null);
    setError(null);
    setShow(false);
  }, [path]);

  useEffect(() => {
    if (!want || !path) return;
    let cancelled = false;
    setBusy(true);
    setError(null);
    detectDepth(path, latest.current)
      .then((d) => !cancelled && setDepth(d))
      .catch((e) => !cancelled && setError(String(e)))
      .finally(() => !cancelled && setBusy(false));
    return () => {
      cancelled = true;
    };
  }, [want, path, geometry]);

  return { show, setShow, depth: show ? depth : null, map: depth, busy, error };
}

export type DepthState = ReturnType<typeof useDepth>;

/** Scene depth at (x, y) (0–1 on the frame) on the lights' scale: 0 camera … 1 far. */
export function depthAt(map: Mask | null, x: number, y: number): number | null {
  if (!map) return null;
  const px = Math.min(map.width - 1, Math.max(0, Math.round(x * (map.width - 1))));
  const py = Math.min(map.height - 1, Math.max(0, Math.round(y * (map.height - 1))));
  return 1 - map.alpha[py * map.width + px] / 255;
}
