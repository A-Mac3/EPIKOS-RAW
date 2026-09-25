import { useEffect, useRef, useState } from "react";
import { thumbnailUrl } from "../api";
import type { FileEntry } from "../types";

const THUMB_SIDE = 320;
const MAX_CONCURRENT = 3;

// Session-wide thumbnail cache and a small queue so a 500-file folder doesn't
// start 500 decodes at once.
const cache = new Map<string, Promise<string>>();
let running = 0;
const waiting: (() => void)[] = [];

/** Take a slot synchronously so bursts can't overshoot the limit. */
function acquire(): Promise<void> {
  if (running < MAX_CONCURRENT) {
    running++;
    return Promise.resolve();
  }
  return new Promise((resolve) => waiting.push(resolve));
}

/** Hand the slot straight to the next waiter, or free it. */
function release() {
  const next = waiting.shift();
  if (next) next();
  else running--;
}

function loadThumbnail(path: string): Promise<string> {
  let p = cache.get(path);
  if (!p) {
    p = acquire()
      .then(() => thumbnailUrl(path, THUMB_SIDE))
      .finally(release);
    p.catch(() => cache.delete(path));
    cache.set(path, p);
  }
  return p;
}

interface Props {
  files: FileEntry[];
  selected: string | null;
  onSelect: (path: string) => void;
}

export function Filmstrip({ files, selected, onSelect }: Props) {
  const strip = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = strip.current?.querySelector<HTMLElement>(".thumb.is-selected");
    el?.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
    // Keep keyboard focus on the selection so the focus ring never marks a second photo.
    if (el && strip.current?.contains(document.activeElement)) el.focus({ preventScroll: true });
  }, [selected]);

  return (
    <div className="filmstrip" ref={strip} role="listbox" aria-label="Photos in folder">
      {files.map((f) => (
        <Thumb key={f.path} file={f} selected={f.path === selected} onSelect={onSelect} />
      ))}
    </div>
  );
}

function Thumb({ file, selected, onSelect }: { file: FileEntry; selected: boolean; onSelect: (p: string) => void }) {
  const el = useRef<HTMLButtonElement>(null);
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const node = el.current;
    if (!node) return;
    let alive = true;
    const io = new IntersectionObserver(
      ([entry]) => {
        if (!entry.isIntersecting) return;
        io.disconnect();
        loadThumbnail(file.path).then(
          (u) => alive && setUrl(u),
          () => alive && setFailed(true),
        );
      },
      { rootMargin: "0px 400px" },
    );
    io.observe(node);
    return () => {
      alive = false;
      io.disconnect();
    };
  }, [file.path]);

  return (
    <button
      ref={el}
      type="button"
      role="option"
      aria-selected={selected}
      className={`thumb${selected ? " is-selected" : ""}`}
      onClick={() => onSelect(file.path)}
      title={`${file.name} · ${file.format}`}
    >
      {url ? <img src={url} alt="" draggable={false} /> : <span className="thumb-ph">{failed ? "No preview" : ""}</span>}
      <span className="thumb-name">{file.name}</span>
      {file.hasEdits && <span className="thumb-dot" title="Has EPIKOS edits" />}
    </button>
  );
}
