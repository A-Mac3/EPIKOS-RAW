import { useCallback, useEffect, useRef, useState } from "react";
import { detectMask, maskModels } from "../api";
import type { Adjustments, Mask, MaskKind, MaskModels } from "../types";

export const MASK_KINDS: MaskKind[] = ["subject", "sky"];

/** Lens settings change the geometry a mask was traced on. */
const geometryKey = (a: Adjustments) => JSON.stringify(a.lens);

/**
 * Step 3 masks for the open image. Detection is an explicit action (IS-Net takes about a
 * second on the CPU) and runs the models one after the other; results are dropped
 * when another image is opened.
 */
export function useMasks(path: string | null, adjustments: Adjustments) {
  const [models, setModels] = useState<MaskModels | null>(null);
  const [masks, setMasks] = useState<Partial<Record<MaskKind, Mask>>>({});
  const [detecting, setDetecting] = useState<MaskKind | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [overlay, setOverlay] = useState<MaskKind | null>(null);
  const [detectedGeometry, setDetectedGeometry] = useState<string | null>(null);
  const currentPath = useRef(path);
  currentPath.current = path;

  useEffect(() => {
    maskModels().then(setModels, (e) => setError(String(e)));
  }, []);

  useEffect(() => {
    setMasks({});
    setDetecting(null);
    setError(null);
    setOverlay(null);
    setDetectedGeometry(null);
  }, [path]);

  const available = (kind: MaskKind) => models?.models.some((m) => m.kind === kind && m.available) ?? false;

  const detect = useCallback(async () => {
    if (!path) return;
    setError(null);
    const geometry = geometryKey(adjustments);
    for (const kind of MASK_KINDS) {
      if (!available(kind)) continue;
      setDetecting(kind);
      try {
        const mask = await detectMask(path, adjustments, kind);
        if (currentPath.current !== path) return;
        setMasks((m) => ({ ...m, [kind]: mask }));
        setDetectedGeometry(geometry);
      } catch (e) {
        if (currentPath.current !== path) return;
        setError(String(e));
        break;
      }
    }
    if (currentPath.current === path) setDetecting(null);
  }, [path, adjustments, models]);

  const stale = detectedGeometry !== null && detectedGeometry !== geometryKey(adjustments);

  return {
    models,
    masks,
    detecting,
    error,
    stale,
    overlay,
    setOverlay,
    detect,
    available,
    overlayMask: overlay ? (masks[overlay] ?? null) : null,
  };
}

export type MaskState = ReturnType<typeof useMasks>;
