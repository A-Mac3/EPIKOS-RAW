import { useSyncExternalStore } from "react";

/**
 * Whether a slider is being dragged right now, app-wide. The viewer hides mask
 * overlays (AI and hand-drawn) while it is, so the edit shows unobscured, and brings
 * them back on release.
 */
let dragging = false;
const listeners = new Set<() => void>();

export function setSliderDragging(on: boolean) {
  if (dragging === on) return;
  dragging = on;
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useSliderDragging(): boolean {
  return useSyncExternalStore(subscribe, () => dragging);
}

// A release anywhere (the pointer may leave the slider mid-drag) ends it.
if (typeof window !== "undefined") {
  for (const type of ["pointerup", "pointercancel", "blur"]) {
    window.addEventListener(type, () => setSliderDragging(false));
  }
}
