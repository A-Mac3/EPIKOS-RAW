import { useEffect, useRef, useState } from "react";
import { localMask } from "../api";
import type { Adjustments, Mask } from "../types";

/**
 * The mask of local adjustment `index` as refined, for the review highlight: fetched
 * while `show` is on and again when its mask, refinement or the geometry change.
 */
export function useLocalMask(path: string | null, adjustments: Adjustments, index: number | null, show: boolean) {
  const local = index !== null ? adjustments.local[index] : undefined;
  const key =
    path && local && show
      ? JSON.stringify([path, index, local.mask, local.grow, local.feather, local.refine, adjustments.lens])
      : null;
  const [state, setState] = useState<{ index: number; mask: Mask } | null>(null);
  const latest = useRef(adjustments);
  latest.current = adjustments;

  useEffect(() => {
    if (!key || !path || index === null) return;
    let live = true;
    const t = setTimeout(() => {
      localMask(path, latest.current, index).then(
        (mask) => live && setState({ index, mask }),
        () => live && setState(null),
      );
    }, 120);
    return () => {
      live = false;
      clearTimeout(t);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  useEffect(() => setState(null), [path]);

  return key && state && state.index === index ? state.mask : null;
}
