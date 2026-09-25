import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { inTauri, listFolder, openImage, pickFolder, saveDocument } from "./api";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { ExportDialog } from "./components/ExportDialog";
import { captureSummary } from "./format";
import { Filmstrip } from "./components/Filmstrip";
import { Histogram } from "./components/Histogram";
import { StepsPanel } from "./components/StepsPanel";
import { StylePanel } from "./components/StylePanel";
import { Viewer } from "./components/Viewer";
import { useHistory } from "./hooks/useHistory";
import { useDepth } from "./hooks/useDepth";
import { useMasks } from "./hooks/useMasks";
import { usePreview } from "./hooks/usePreview";
import { useStyles } from "./hooks/useStyles";
import { defaultAdjustments, type Adjustments, type FileEntry, type ImageInfo } from "./types";

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
  const depth = useDepth(info?.path ?? null, adjustments);
  const [picking, setPicking] = useState(false);
  const at = adjustments.atmosphere;
  const lightMarker =
    at.shafts > 0 && !at.shaftAuto ? { x: at.shaftX, y: at.shaftY } : null;
  const placeLight = useCallback(
    (x: number, y: number) => {
      history.commit((a) => ({
        ...a,
        atmosphere: { ...a.atmosphere, shaftAuto: false, shaftX: x, shaftY: y, shafts: a.atmosphere.shafts || 50 },
      }));
      setPicking(false);
    },
    [history.commit],
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

  const chooseFolder = useCallback(async () => {
    const dir = await pickFolder();
    if (!dir) return;
    await flushSave();
    setFolderError(null);
    try {
      const entries = await listFolder(dir);
      setFolder(dir);
      setFiles(entries);
      setInfo(null);
      setSelected(null);
      if (entries.length > 0) void select(entries[0].path);
    } catch (e) {
      setFolderError(String(e));
    }
  }, [flushSave, select]);

  const step = useCallback(
    (delta: number) => {
      if (files.length === 0) return;
      const i = files.findIndex((f) => f.path === selected);
      const next = files[Math.max(0, Math.min(files.length - 1, (i < 0 ? 0 : i) + delta))];
      if (next && next.path !== selected) void select(next.path);
    },
    [files, selected, select],
  );

  // ---- Keyboard -----------------------------------------------------------------------

  useEffect(() => setPicking(false), [info?.path]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (exportOpen) return; // the modal owns the keyboard
      if (e.key === "Escape") setPicking(false);
      const mod = e.metaKey || e.ctrlKey;
      const inField = e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement;
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
      } else if (mod && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void chooseFolder();
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
  }, [history.undo, history.redo, chooseFolder, step, exportOpen, info]);

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
            title="Export full-resolution 16-bit TIFF (⌘E)"
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
            {files.length === 0 ? (
              <div className="viewer-status">No RAW or DNG files in this folder.</div>
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
                  onPick={picking ? placeLight : null}
                />
              </ErrorBoundary>
            )}
          </main>
          <aside className="panel">
            <Histogram preview={preview} />
            <div className="panel-scroll">
              <ErrorBoundary area="the side panel">
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
                    picking={picking}
                    setPicking={setPicking}
                  />
                ) : (
                  <p className="note pad">Select a photo to start editing.</p>
                )}
              </ErrorBoundary>
            </div>
          </aside>
          <footer className="strip">
            <Filmstrip files={files} selected={selected} onSelect={(p) => void select(p)} />
          </footer>
        </>
      ) : (
        <div className="welcome">
          <h1>
            EPIKOS <span>RAW</span>
          </h1>
          <p>Open a folder of RAW or DNG files to begin.</p>
          <button type="button" className="btn primary" onClick={() => void chooseFolder()}>
            Open folder…
          </button>
          {folderError && <p className="error">{folderError}</p>}
          <p className="hint">Sony ARW · Canon CR3/CR2 · Nikon NEF · Fujifilm RAF · Leica DNG · Apple ProRAW</p>
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
