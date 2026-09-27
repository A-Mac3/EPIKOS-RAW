import { useCallback, useEffect, useRef, useState } from "react";
import { detectMask, maskModels } from "../api";
import { MASK_TARGETS, type Adjustments, type Mask, type MaskModels, type MaskTarget } from "../types";

/** The masks Detect makes, in order (background comes free with the subject). */
export const MASK_KINDS: MaskTarget[] = MASK_TARGETS;

export const MASK_LABEL: Record<MaskTarget, string> = {
  subject: "Subject",
  background: "Background",
  sky: "Sky",
  skin: "Skin",
  faceSkin: "Facial skin",
  bodySkin: "Body skin",
  eyes: "Eyes",
  eyebrows: "Eyebrows",
  eyelashes: "Eyelashes",
  teeth: "Teeth",
  hair: "Hair",
  foreground: "Foreground",
};

/** Lens settings change the geometry a mask was traced on. */
const geometryKey = (a: Adjustments) => JSON.stringify(a.lens);

/**
 * Step 3 masks for the open image. Detection is an explicit action (the models take a
 * few seconds together on the CPU) and runs them one after the other; results are
 * dropped when another image is opened. Local adjustments don't need this: the engine
 * makes the masks they use on its own.
 */
export function useMasks(path: string | null, adjustments: Adjustments) {
  const [models, setModels] = useState<MaskModels | null>(null);
  const [masks, setMasks] = useState<Partial<Record<MaskTarget, Mask>>>({});
  const [detecting, setDetecting] = useState<MaskTarget | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [overlay, setOverlay] = useState<MaskTarget | null>(null);
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

  const available = useCallback(
    (target: MaskTarget) => models?.targets.some((t) => t.target === target && t.available) ?? false,
    [models],
  );

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
  }, [path, adjustments, available]);

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
