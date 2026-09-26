import { useCallback, useRef, useState } from "react";

const LIMIT = 200;

interface Entry<T> {
  value: T;
  /** What changed to reach this state ("Opened" for the first). */
  label: string;
}

interface State<T> {
  items: Entry<T>[];
  index: number;
}

/**
 * Undo/redo for continuous controls, as a labelled timeline (for the History panel).
 * `edit` updates the value live (e.g. while a slider is dragged); `endEdit` turns
 * everything since the first `edit` into one step, labelled by `describe`.
 */
export function useHistory<T>(initial: T, describe: (before: T, after: T) => string = () => "Edit") {
  const [state, setState] = useState<State<T>>({ items: [{ value: initial, label: "Opened" }], index: 0 });
  const pending = useRef<T | null>(null);
  /** A continuous edit (slider or handle drag) is in progress. */
  const [dragging, setDragging] = useState(false);
  const presentRef = useRef(initial);
  presentRef.current = state.items[state.index].value;
  const describeRef = useRef(describe);
  describeRef.current = describe;

  const edit = useCallback((update: (value: T) => T) => {
    if (pending.current === null) {
      pending.current = presentRef.current;
      setDragging(true);
    }
    setState((s) => {
      const items = s.items.slice();
      items[s.index] = { ...items[s.index], value: update(items[s.index].value) };
      return { ...s, items };
    });
  }, []);

  const endEdit = useCallback(() => {
    const before = pending.current;
    pending.current = null;
    setDragging(false);
    if (before === null) return;
    setState((s) => {
      const present = s.items[s.index];
      if (Object.is(before, present.value)) return s;
      // A new step drops any redo states after this one.
      const items = [
        ...s.items.slice(0, s.index),
        { value: before, label: present.label },
        { value: present.value, label: describeRef.current(before, present.value) },
      ].slice(-LIMIT);
      return { items, index: items.length - 1 };
    });
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
    setState((s) => {
      const items = s.items.slice();
      items[s.index] = { ...items[s.index], value: update(items[s.index].value) };
      return { ...s, items };
    });
  }, []);

  /** Jump to step `i` of the timeline (the History panel). */
  const goTo = useCallback((i: number) => {
    pending.current = null;
    setDragging(false);
    setState((s) => ({ ...s, index: Math.max(0, Math.min(s.items.length - 1, i)) }));
  }, []);

  const undo = useCallback(() => {
    pending.current = null;
    setState((s) => ({ ...s, index: Math.max(0, s.index - 1) }));
  }, []);

  const redo = useCallback(() => {
    pending.current = null;
    setState((s) => ({ ...s, index: Math.min(s.items.length - 1, s.index + 1) }));
  }, []);

  /** Start a fresh history (e.g. a different image was opened). */
  const reset = useCallback((value: T) => {
    pending.current = null;
    setState({ items: [{ value, label: "Opened" }], index: 0 });
  }, []);

  return {
    value: state.items[state.index].value,
    canUndo: state.index > 0,
    canRedo: state.index < state.items.length - 1,
    /** True while a slider or handle is being dragged (for fast proxy renders). */
    dragging,
    /** Step labels, oldest first, and which one is current. */
    steps: state.items.map((e) => e.label),
    index: state.index,
    edit,
    endEdit,
    commit,
    amend,
    goTo,
    undo,
    redo,
    reset,
  };
}
