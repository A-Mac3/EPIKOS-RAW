import { useEffect, useRef, useState } from "react";
import { detectDepth } from "../api";
import type { Adjustments, Mask } from "../types";

/**
 * Step 6 depth overlay. The engine estimates depth by itself whenever fog or light
 * shafts need it; this only fetches it for display while "Show depth" is on, again
 * when the lens geometry changes.
 */
export function useDepth(path: string | null, adjustments: Adjustments) {
  const [show, setShow] = useState(false);
  const [depth, setDepth] = useState<Mask | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const geometry = JSON.stringify(adjustments.lens);
  const latest = useRef(adjustments);
  latest.current = adjustments;

  useEffect(() => {
    setDepth(null);
    setError(null);
    setShow(false);
  }, [path]);

  useEffect(() => {
    if (!show || !path) return;
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
  }, [show, path, geometry]);

  return { show, setShow, depth: show ? depth : null, busy, error };
}

export type DepthState = ReturnType<typeof useDepth>;
