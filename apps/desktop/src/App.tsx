import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { inTauri, listFolder, openImage, pickFolder, saveDocument, storyArc } from "./api";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { ExportDialog } from "./components/ExportDialog";
import { captureSummary } from "./format";
import { Filmstrip } from "./components/Filmstrip";
import { Histogram } from "./components/Histogram";
import type { LightKind } from "./components/LightPalette";
import { LookPromptBar } from "./components/LookPromptBar";
import { BatchSyncCard, SceneCard } from "./components/ScenePanel";
import { StepsPanel } from "./components/StepsPanel";
import { StylePanel } from "./components/StylePanel";
import { Viewer } from "./components/Viewer";
import { useHistory } from "./hooks/useHistory";
import { depthAt, useDepth } from "./hooks/useDepth";
import { useMasks } from "./hooks/useMasks";
import { usePreview } from "./hooks/usePreview";
import { useStyles } from "./hooks/useStyles";
import {
  defaultAdjustments,
  defaultLight,
  type Adjustments,
  type FileEntry,
  type ImageInfo,
  type PickTarget,
  type StoryArc,
} from "./types";

const AUTOSAVE_MS = 600;
const MAX_PREVIEW_SIDE = 4096;

type SaveStatus =
  | { kind: "idle" }
  | { kind: "unsaved" }
  | { kind: "saving" }
  | { kind: "saved"; warning: string | null }
  | { kind: "error"; message: string };

export default function App() {
  const [folder, setFolder] = useState<string | null>(null);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [story, setStory] = useState<StoryArc | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [info, setInfo] = useState<ImageInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [viewSize, setViewSize] = useState({ w: 0, h: 0 });
  const [before, setBefore] = useState(false);
  const [save, setSave] = useState<SaveStatus>({ kind: "idle" });
  const [exportOpen, setExportOpen] = useState(false);

  const history = useHistory<Adjustments>(defaultAdjustments());
  const adjustments = history.value;
  const beforeAdjustments = useMemo(defaultAdjustments, []);

  const { preview, error: renderError, busy } = usePreview(
    info?.path ?? null,
    info ? (before ? beforeAdjustments : adjustments) : null,
    Math.min(viewSize.w, MAX_PREVIEW_SIDE),
    Math.min(viewSize.h, MAX_PREVIEW_SIDE),
  );

  const masks = useMasks(info?.path ?? null, adjustments);
  const { styles, thumbs } = useStyles(info?.path ?? null, adjustments);
  const [picking, setPicking] = useState<PickTarget>(null);
  const promptRef = useRef<HTMLInputElement>(null);
  const [lightDrag, setLightDrag] = useState(false);
  const depth = useDepth(info?.path ?? null, adjustments, picking === "light" || lightDrag);
  const [selectedLight, setSelectedLight] = useState<number | null>(null);
  const at = adjustments.atmosphere;
  const lightMarker = at.shafts > 0 && !at.shaftAuto ? { x: at.shaftX, y: at.shaftY } : null;
  const placeShaftLight = useCallback(
    (x: number, y: number) => {
      history.commit((a) => ({
        ...a,
        atmosphere: { ...a.atmosphere, shaftAuto: false, shaftX: x, shaftY: y, shafts: a.atmosphere.shafts || 50 },
      }));
      setPicking(null);
    },
    [history.commit],
  );
  const depthMap = depth.map;
  const addLight = useCallback(
    (x: number, y: number) => {
      // Just in front of whatever was clicked, so it lights that surface.
      const surface = depthAt(depthMap, x, y);
      const light = { ...defaultLight(x, y), depth: surface === null ? 0.4 : Math.max(0, surface - 0.08) };
      history.commit((a) => ({ ...a, atmosphere: { ...a.atmosphere, lights: [...a.atmosphere.lights, light] } }));
      setSelectedLight(adjustments.atmosphere.lights.length);
      setPicking(null);
    },
    [history.commit, depthMap, adjustments.atmosphere.lights.length],
  );
  // Drop presets for the light palette, relative to the surface under the drop point.
  const presetFor = (kind: LightKind, surface: number) =>
    ({
      key: { depth: Math.max(0, surface - 0.1), intensity: 55, reach: 45, warmth: 20, halo: 0 },
      rim: { depth: Math.min(1, surface + 0.12), intensity: 75, reach: 50, warmth: 45, halo: 55 },
      sun: { depth: 0.97, intensity: 70, reach: 90, warmth: 80, halo: 80 },
    })[kind];
  // A drop that beat the depth map: its depth is corrected once the map arrives.
  const [pendingDrop, setPendingDrop] = useState<{ index: number; kind: LightKind; x: number; y: number } | null>(
    null,
  );
  const dropLight = useCallback(
    (kind: LightKind, x: number, y: number) => {
      const surface = depthAt(depthMap, x, y);
      const index = Math.min(3, adjustments.atmosphere.lights.length);
      history.commit((a) => ({
        ...a,
        atmosphere: {
          ...a.atmosphere,
          lights: [...a.atmosphere.lights, { x, y, ...presetFor(kind, surface ?? 0.5) }].slice(-4),
        },
      }));
      setSelectedLight(index);
      setPendingDrop(surface === null ? { index, kind, x, y } : null);
    },
    [history.commit, depthMap, adjustments.atmosphere.lights.length],
  );
  useEffect(() => {
    if (!pendingDrop || !depthMap) return;
    const { index, kind, x, y } = pendingDrop;
    const depth = presetFor(kind, depthAt(depthMap, x, y) ?? 0.5).depth;
    // Part of the drop's own undo step, not a new one.
    history.amend((a) => ({
      ...a,
      atmosphere: {
        ...a.atmosphere,
        lights: a.atmosphere.lights.map((l, k) => (k === index && l.x === x && l.y === y ? { ...l, depth } : l)),
      },
    }));
    setPendingDrop(null);
  }, [pendingDrop, depthMap, history.amend]);
  const depthCommit = useRef<number | undefined>(undefined);
  const nudgeLightDepth = useCallback(
    (i: number, delta: number) => {
      history.edit((a) => ({
        ...a,
        atmosphere: {
          ...a.atmosphere,
          lights: a.atmosphere.lights.map((l, k) =>
            k === i ? { ...l, depth: Math.min(1, Math.max(0, Math.round((l.depth + delta) * 100) / 100)) } : l,
          ),
        },
      }));
      // A scroll gesture is one undo step: close it once scrolling pauses.
      window.clearTimeout(depthCommit.current);
      depthCommit.current = window.setTimeout(history.endEdit, 400);
    },
    [history.edit, history.endEdit],
  );
  const moveLight = useCallback(
    (i: number, x: number, y: number) =>
      history.edit((a) => ({
        ...a,
        atmosphere: {
          ...a.atmosphere,
          lights: a.atmosphere.lights.map((l, k) => (k === i ? { ...l, x, y } : l)),
        },
      })),
    [history.edit],
  );

  // ---- Saving -------------------------------------------------------------------------

  const savedRef = useRef<Adjustments | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const pendingSave = useRef<(() => Promise<void>) | null>(null);

  const flushSave = useCallback(async () => {
    window.clearTimeout(timer.current);
    const run = pendingSave.current;
    pendingSave.current = null;
    await run?.();
  }, []);

  useEffect(() => {
    if (!info) return;
    if (adjustments === savedRef.current) {
      // Undone back to what's on disk: drop any queued save of the intermediate state.
      if (pendingSave.current) {
        window.clearTimeout(timer.current);
        pendingSave.current = null;
        setSave((s) => (s.kind === "unsaved" ? { kind: "idle" } : s));
      }
      return;
    }
    const path = info.path;
    const document = { ...info.document, adjustments };
    setSave({ kind: "unsaved" });
    pendingSave.current = async () => {
      setSave({ kind: "saving" });
      try {
        const report = await saveDocument(path, document);
        savedRef.current = adjustments;
        setSave({ kind: "saved", warning: report.warning });
        setFiles((fs) => fs.map((f) => (f.path === path ? { ...f, hasEdits: true } : f)));
      } catch (e) {
        setSave({ kind: "error", message: String(e) });
      }
    };
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => void flushSave(), AUTOSAVE_MS);
  }, [adjustments, info, flushSave]);

  // Save before the window closes.
  useEffect(() => {
    const onHide = () => void flushSave();
    window.addEventListener("pagehide", onHide);
    return () => window.removeEventListener("pagehide", onHide);
  }, [flushSave]);

  // ---- Opening ------------------------------------------------------------------------

  const openToken = useRef(0);
  const select = useCallback(
    async (path: string) => {
      await flushSave();
      const token = ++openToken.current;
      setSelected(path);
      setLoading(true);
      setOpenError(null);
      try {
        const opened = await openImage(path);
        if (token !== openToken.current) return; // a newer selection won
        savedRef.current = opened.document.adjustments;
        history.reset(opened.document.adjustments);
        setInfo(opened);
        setSave({ kind: "idle" });
      } catch (e) {
        if (token !== openToken.current) return;
        setInfo(null);
        setOpenError(String(e));
      } finally {
        if (token === openToken.current) setLoading(false);
      }
    },
    [flushSave, history.reset],
  );

  const folderToken = useRef(0);
  const chooseFolder = useCallback(async () => {
    const dir = await pickFolder();
    if (!dir) return;
    await flushSave();
    setFolderError(null);
    try {
      const entries = await listFolder(dir);
      const token = ++folderToken.current;
      setFolder(dir);
      setFiles(entries);
      setStory(null);
      setInfo(null);
      setSelected(null);
      if (entries.length > 0) void select(entries[0].path);
      // Story-arc grouping reads only previews and EXIF, so it's quick; the strip
      // regroups when it lands. A failure just leaves the plain strip.
      storyArc(dir).then(
        (arc) => token === folderToken.current && setStory(arc),
        (e) => console.warn("story arc:", e),
      );
    } catch (e) {
      setFolderError(String(e));
    }
  }, [flushSave, select]);

  // Shooting order once the story arc is known (groups in time order), else name order.
  const ordered = useMemo(() => {
    if (!story) return files;
    const rank = new Map(story.groups.flatMap((g) => g.frames).map((p, i) => [p, i]));
    return [...files].sort((a, b) => (rank.get(a.path) ?? Infinity) - (rank.get(b.path) ?? Infinity));
  }, [files, story]);
  const group = useMemo(
    () => (info && story ? (story.groups.find((g) => g.frames.includes(info.path)) ?? null) : null),
    [info, story],
  );
  // A sync or undo rewrote other photos' sidecars: refresh their edit dots.
  const refreshFiles = useCallback(() => {
    if (!folder) return;
    listFolder(folder).then(setFiles, (e) => console.warn("refresh folder:", e));
  }, [folder]);

  const step = useCallback(
    (delta: number) => {
      if (ordered.length === 0) return;
      const i = ordered.findIndex((f) => f.path === selected);
      const next = ordered[Math.max(0, Math.min(ordered.length - 1, (i < 0 ? 0 : i) + delta))];
      if (next && next.path !== selected) void select(next.path);
    },
    [ordered, selected, select],
  );

  // ---- Keyboard -----------------------------------------------------------------------

  useEffect(() => {
    setPicking(null);
    setSelectedLight(null);
    setLightDrag(false);
  }, [info?.path]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (exportOpen) return; // the modal owns the keyboard
      if (e.key === "Escape") setPicking(null);
      const mod = e.metaKey || e.ctrlKey;
      const inField =
        e.target instanceof HTMLInputElement ||
        e.target instanceof HTMLSelectElement ||
        e.target instanceof HTMLTextAreaElement;
      if (mod && e.key.toLowerCase() === "z") {
        e.preventDefault();
        if (e.shiftKey) history.redo();
        else history.undo();
      } else if (mod && e.key.toLowerCase() === "y") {
        e.preventDefault();
        history.redo();
      } else if (mod && e.key.toLowerCase() === "e") {
        e.preventDefault();
        if (info) setExportOpen(true);
      } else if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        promptRef.current?.focus();
        promptRef.current?.select();
      } else if (mod && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void chooseFolder();
      } else if (!inField && (e.key === "Backspace" || e.key === "Delete") && selectedLight !== null) {
        e.preventDefault();
        const i = selectedLight;
        history.commit((a) => ({
          ...a,
          atmosphere: { ...a.atmosphere, lights: a.atmosphere.lights.filter((_, k) => k !== i) },
        }));
        setSelectedLight(null);
      } else if (!inField && e.key === "ArrowRight") {
        step(1);
      } else if (!inField && e.key === "ArrowLeft") {
        step(-1);
      } else if (e.key === "\\" && !e.repeat) {
        setBefore(true);
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.key === "\\") setBefore(false);
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [history.undo, history.redo, history.commit, chooseFolder, step, exportOpen, info, selectedLight]);

  const onResize = useCallback((w: number, h: number) => setViewSize({ w, h }), []);

  // ---- Render -------------------------------------------------------------------------

  if (!inTauri) {
    return (
      <div className="empty">
        <h1>EPIKOS RAW</h1>
        <p>This interface talks to the native engine. Start it with <code>npm run tauri dev</code>.</p>
      </div>
    );
  }

  return (
    <div className={`app${folder ? "" : " is-empty"}`}>
      <header className="topbar">
        <div className="brand">
          EPIKOS <span>RAW</span>
        </div>
        <button type="button" className="btn" onClick={() => void chooseFolder()} title="Open folder (⌘O)">
          Open folder…
        </button>
        <div className="topbar-title">
          {info ? (
            <>
              <strong>{info.name}</strong>
              <span>
                {info.make} {info.model} · {info.width}×{info.height}
              </span>
              {captureSummary(info.capture) && <span className="capture">{captureSummary(info.capture)}</span>}
            </>
          ) : (
            folder && <span>{folder}</span>
          )}
        </div>
        <div className="topbar-actions">
          <button type="button" className="btn icon" disabled={!history.canUndo} onClick={history.undo} title="Undo (⌘Z)">
            ↶
          </button>
          <button type="button" className="btn icon" disabled={!history.canRedo} onClick={history.redo} title="Redo (⇧⌘Z)">
            ↷
          </button>
          <button
            type="button"
            className={`btn${before ? " is-active" : ""}`}
            disabled={!info}
            onPointerDown={() => setBefore(true)}
            onPointerUp={() => setBefore(false)}
            onPointerLeave={() => setBefore(false)}
            title="Hold to compare with the unedited image (hold \)"
          >
            Before
          </button>
          <button
            type="button"
            className="btn primary-outline"
            disabled={!info}
            onClick={() => setExportOpen(true)}
            title="Export TIFF, layered PSD or enhanced DNG (⌘E)"
          >
            Export…
          </button>
          <SaveBadge status={save} />
        </div>
      </header>

      {exportOpen && info && (
        <ErrorBoundary area="the export dialog">
          <ExportDialog info={info} adjustments={adjustments} onClose={() => setExportOpen(false)} />
        </ErrorBoundary>
      )}

      {folder ? (
        <>
          <main className="stage">
            {info && (
              <ErrorBoundary area="the look prompt">
                <LookPromptBar ref={promptRef} path={info.path} adjustments={adjustments} commit={history.commit} />
              </ErrorBoundary>
            )}
            {files.length === 0 ? (
              <div className="viewer-status">No RAW, DNG, JPEG or PNG files in this folder.</div>
            ) : (
              <ErrorBoundary area="the viewer">
                <Viewer
                  preview={preview}
                  busy={busy}
                  error={openError ?? renderError}
                  loading={loading}
                  showingBefore={before}
                  overlay={depth.depth ?? masks.overlayMask}
                  onResize={onResize}
                  marker={lightMarker}
                  onPick={picking === "shafts" ? placeShaftLight : picking === "light" ? addLight : null}
                  lights={at.lights}
                  selectedLight={selectedLight}
                  onSelectLight={setSelectedLight}
                  onMoveLight={moveLight}
                  onMoveLightEnd={history.endEdit}
                  onLightDepth={nudgeLightDepth}
                  onDropLight={dropLight}
                  onLightPrepare={() => setLightDrag(true)}
                />
              </ErrorBoundary>
            )}
          </main>
          <aside className="panel">
            <Histogram preview={preview} />
            <div className="panel-scroll">
              <ErrorBoundary area="the side panel">
                {info && <SceneCard info={info} />}
                {info && (
                  <BatchSyncCard
                    info={info}
                    adjustments={adjustments}
                    group={group}
                    flushSave={flushSave}
                    onChanged={refreshFiles}
                  />
                )}
                {info && (
                  <StylePanel
                    styles={styles}
                    thumbs={thumbs}
                    adjustments={adjustments}
                    edit={history.edit}
                    endEdit={history.endEdit}
                    commit={history.commit}
                  />
                )}
                {info ? (
                  <StepsPanel
                    info={info}
                    adjustments={adjustments}
                    edit={history.edit}
                    endEdit={history.endEdit}
                    commit={history.commit}
                    masks={masks}
                    depth={depth}
                    histogram={preview?.histogram ?? null}
                    picking={picking}
                    setPicking={setPicking}
                    selectedLight={selectedLight}
                    setSelectedLight={setSelectedLight}
                  />
                ) : (
                  <p className="note pad">Select a photo to start editing.</p>
                )}
              </ErrorBoundary>
            </div>
          </aside>
          <footer className="strip">
            <Filmstrip files={files} story={story} selected={selected} onSelect={(p) => void select(p)} />
          </footer>
        </>
      ) : (
        <div className="welcome">
          <h1>
            EPIKOS <span>RAW</span>
          </h1>
          <p>Open a folder of RAW, DNG, JPEG or PNG files to begin.</p>
          <button type="button" className="btn primary" onClick={() => void chooseFolder()}>
            Open folder…
          </button>
          {folderError && <p className="error">{folderError}</p>}
          <p className="hint">
            Sony ARW · Canon CR3/CR2 · Nikon NEF · Fujifilm RAF · Leica DNG · Apple ProRAW · JPEG · PNG
          </p>
        </div>
      )}
    </div>
  );
}

function SaveBadge({ status }: { status: SaveStatus }) {
  switch (status.kind) {
    case "idle":
      return null;
    case "unsaved":
    case "saving":
      return <span className="save">Saving…</span>;
    case "saved":
      return status.warning ? (
        <span className="save is-warn" title={status.warning}>
          Saved (XMP skipped)
        </span>
      ) : (
        <span className="save is-ok">Saved</span>
      );
    case "error":
      return (
        <span className="save is-error" title={status.message}>
          Save failed
        </span>
      );
  }
}
