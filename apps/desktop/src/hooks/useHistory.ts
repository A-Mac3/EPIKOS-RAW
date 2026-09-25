import { useCallback, useRef, useState } from "react";

const LIMIT = 200;

interface State<T> {
  past: T[];
  present: T;
  future: T[];
}

/**
 * Undo/redo for continuous controls. `edit` updates the value live (e.g. while a slider
 * is dragged); `endEdit` turns everything since the first `edit` into one undo step.
 */
export function useHistory<T>(initial: T) {
  const [state, setState] = useState<State<T>>({ past: [], present: initial, future: [] });
  const pending = useRef<T | null>(null);
  const presentRef = useRef(initial);
  presentRef.current = state.present;

  const edit = useCallback((update: (value: T) => T) => {
    if (pending.current === null) pending.current = presentRef.current;
    setState((s) => ({ ...s, present: update(s.present) }));
  }, []);

  const endEdit = useCallback(() => {
    const before = pending.current;
    pending.current = null;
    if (before === null) return;
    setState((s) =>
      Object.is(before, s.present)
        ? s
        : { past: [...s.past, before].slice(-LIMIT), present: s.present, future: [] },
    );
  }, []);

  /** Discrete change (toggle, select): one immediate undo step. */
  const commit = useCallback(
    (update: (value: T) => T) => {
      edit(update);
      endEdit();
    },
    [edit, endEdit],
  );

  /**
   * Change the current value without a new undo step: the change joins the last step
   * (e.g. a correction that arrives just after the edit it belongs to).
   */
  const amend = useCallback((update: (value: T) => T) => {
    setState((s) => ({ ...s, present: update(s.present) }));
  }, []);

  const undo = useCallback(() => {
    pending.current = null;
    setState((s) =>
      s.past.length === 0
        ? s
        : { past: s.past.slice(0, -1), present: s.past[s.past.length - 1], future: [s.present, ...s.future] },
    );
  }, []);

  const redo = useCallback(() => {
    pending.current = null;
    setState((s) =>
      s.future.length === 0
        ? s
        : { past: [...s.past, s.present], present: s.future[0], future: s.future.slice(1) },
    );
  }, []);

  /** Start a fresh history (e.g. a different image was opened). */
  const reset = useCallback((value: T) => {
    pending.current = null;
    setState({ past: [], present: value, future: [] });
  }, []);

  return {
    value: state.present,
    canUndo: state.past.length > 0,
    canRedo: state.future.length > 0,
    edit,
    endEdit,
    commit,
    amend,
    undo,
    redo,
    reset,
  };
}
