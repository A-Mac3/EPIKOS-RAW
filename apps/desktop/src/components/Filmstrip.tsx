import { useEffect, useRef, useState } from "react";
import { thumbnailUrl } from "../api";
import type { FileEntry, StoryArc } from "../types";
import { Palette } from "./ScenePanel";

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
  /** Story-arc groups; the strip shows them as labelled runs once they arrive. */
  story: StoryArc | null;
  selected: string | null;
  onSelect: (path: string) => void;
  /** The photo whose AI masks are loaded (the open one, once any mask is made). */
  masksLoaded?: string | null;
}

/** RAW, DNG, JPEG, PNG or TIFF, from the decoder's format label. */
export function formatBadge(format: string): string {
  const f = format.toUpperCase();
  for (const k of ["DNG", "JPEG", "PNG", "TIFF"]) if (f.includes(k)) return k === "JPEG" ? "JPG" : k === "TIFF" ? "TIF" : k;
  return "RAW";
}

export function Filmstrip({ files, story, selected, onSelect, masksLoaded = null }: Props) {
  const strip = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = strip.current?.querySelector<HTMLElement>(".thumb.is-selected");
    el?.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
    // Keep keyboard focus on the selection so the focus ring never marks a second photo.
    if (el && strip.current?.contains(document.activeElement)) el.focus({ preventScroll: true });
  }, [selected]);

  const thumb = (f: FileEntry, hero = false) => (
    <Thumb
      key={f.path}
      file={f}
      hero={hero}
      selected={f.path === selected}
      masks={f.path === masksLoaded}
      onSelect={onSelect}
    />
  );
  const byPath = new Map(files.map((f) => [f.path, f]));
  const grouped = new Set(story?.groups.flatMap((g) => g.frames) ?? []);
  return (
    <div className="filmstrip" ref={strip} role="listbox" aria-label="Photos in folder">
      {story
        ? [
            ...story.groups.map((g) => (
              <div
                key={g.id}
                className="story-group"
                role="group"
                aria-label={g.label}
                title={g.splitReason ? `New group: ${g.splitReason}` : undefined}
              >
                <div className="story-head">
                  <span>{g.label}</span>
                  <Palette swatches={g.palette} compact />
                </div>
                <div className="story-thumbs">
                  {g.frames.flatMap((p) => {
                    const f = byPath.get(p);
                    return f ? [thumb(f, p === g.hero && g.frames.length > 1)] : [];
                  })}
                </div>
              </div>
            )),
            ...files.filter((f) => !grouped.has(f.path)).map((f) => thumb(f)),
          ]
        : files.map((f) => thumb(f))}
    </div>
  );
}

function Thumb({
  file,
  hero,
  selected,
  masks,
  onSelect,
}: {
  file: FileEntry;
  hero: boolean;
  selected: boolean;
  masks: boolean;
  onSelect: (p: string) => void;
}) {
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
      <span className="thumb-badges" aria-hidden>
        <span className={`thumb-badge is-${formatBadge(file.format).toLowerCase()}`}>{formatBadge(file.format)}</span>
        {masks && (
          <span className="thumb-badge is-icon" title="AI masks loaded">
            <svg viewBox="0 0 12 12" width="10" height="10">
              <circle cx="6" cy="4.2" r="2.3" fill="none" stroke="currentColor" strokeWidth="1.2" />
              <path d="M1.8 11c.5-2.3 2.2-3.6 4.2-3.6s3.7 1.3 4.2 3.6" fill="none" stroke="currentColor" strokeWidth="1.2" />
            </svg>
          </span>
        )}
        {file.hasEdits && (
          <span className="thumb-badge is-icon is-saved" title="Edits saved to .epikos sidecar">
            <svg viewBox="0 0 12 12" width="10" height="10">
              <path d="M3 1.5h4l2.5 2.5v6.5H3z" fill="none" stroke="currentColor" strokeWidth="1.1" />
              <path d="M4.4 7.2l1.2 1.2 2.2-2.4" fill="none" stroke="currentColor" strokeWidth="1.2" />
            </svg>
          </span>
        )}
      </span>
      {hero && (
        <span className="thumb-hero" title="Hero frame of this group">
          ★
        </span>
      )}
    </button>
  );
}
