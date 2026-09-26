import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  inTauri,
  learnedStyles,
  listFiles,
  listLuts,
  listPresets,
  listFolder,
  onFileDrag,
  openImage,
  pickFolder,
  pickPhoto,
  saveDocument,
  storyArc,
  storyArcFiles,
} from "./api";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { ExportDialog } from "./components/ExportDialog";
import { shutter } from "./format";
import { Filmstrip } from "./components/Filmstrip";
import { Histogram } from "./components/Histogram";
import type { LightKind } from "./components/LightPalette";
import { LookPromptBar } from "./components/LookPromptBar";
import { BatchSyncCard, SceneCard } from "./components/ScenePanel";
import { StepsPanel } from "./components/StepsPanel";
import { StylePanel } from "./components/StylePanel";
import { HistoryPanel, LeftSidebar } from "./components/LeftSidebar";
import { LearnedLibrary } from "./components/LearnedLibrary";
import { SettingsDialog } from "./components/SettingsDialog";
import { loadAiSettings, storeAiSettings, type AiSettings } from "./settings";
import type { BrushSettings } from "./components/ManualMaskTool";
import { MentorPanel } from "./components/MentorPanel";
import { describeChange } from "./historyLabel";
import { Viewer } from "./components/Viewer";
import { useHistory } from "./hooks/useHistory";
import { depthAt, useDepth } from "./hooks/useDepth";
import { useMasks } from "./hooks/useMasks";
import { usePreview } from "./hooks/usePreview";
import { useStyles } from "./hooks/useStyles";
import {
  cropIsActive,
  defaultAdjustments,
  defaultCrop,
  defaultLight,
  type Adjustments,
  type FileEntry,
  type ExportFormat,
  type ImageInfo,
  type LearnedStyle,
  type LutInfo,
  type Preset,
  type PickTarget,
  type StyleInfo,
  type StoryArc,
} from "./types";

const AUTOSAVE_MS = 600;
const MAX_PREVIEW_SIDE = 4096;
/** Long side of the preview rendered while a control is dragged. */
const DRAG_PROXY_SIDE = 960;
const MENTOR_TARGET_KEY = "epikos.mentor.target";

/** What the filmstrip shows: a folder, or photos opened one by one (Open Photo, a drop). */
type Source = { kind: "folder"; dir: string } | { kind: "files"; paths: string[] };

const listSource = (s: Source) => (s.kind === "folder" ? listFolder(s.dir) : listFiles(s.paths));
const storySource = (s: Source) => (s.kind === "folder" ? storyArc(s.dir) : storyArcFiles(s.paths));

type SaveStatus =
  | { kind: "idle" }
  | { kind: "unsaved" }
  | { kind: "saving" }
  | { kind: "saved"; warning: string | null }
  | { kind: "error"; message: string };

export default function App() {
  const [source, setSource] = useState<Source | null>(null);
  const [files, setFiles] = useState<FileEntry[]>([]);
  const [story, setStory] = useState<StoryArc | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);
  /** Files are being dragged over the window. */
  const [dragging, setDragging] = useState(false);
  // An open failure over a running session is a passing notice, not a state.
  useEffect(() => {
    if (!source || !folderError) return;
    const t = window.setTimeout(() => setFolderError(null), 8000);
    return () => window.clearTimeout(t);
  }, [source, folderError]);
  const [selected, setSelected] = useState<string | null>(null);
  const [info, setInfo] = useState<ImageInfo | null>(null);
  const [loading, setLoading] = useState(false);
  const [openError, setOpenError] = useState<string | null>(null);
  const [viewSize, setViewSize] = useState({ w: 0, h: 0 });
  const [before, setBefore] = useState(false);
  const [save, setSave] = useState<SaveStatus>({ kind: "idle" });
  const [exportOpen, setExportOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [aiSettings, setAiSettingsState] = useState<AiSettings>(loadAiSettings);
  const setAiSettings = (s: AiSettings) => {
    setAiSettingsState(s);
    storeAiSettings(s);
  };
  /** The last finished export of the open photo, for the toolbar. */
  const [lastExport, setLastExport] = useState<{ path: string; label: string } | null>(null);

  const stylesRef = useRef<StyleInfo[]>([]);
  const history = useHistory<Adjustments>(defaultAdjustments(), (b, a) => describeChange(b, a, stylesRef.current));
  const adjustments = history.value;
  // The unedited image in the same framing (lens, straighten), so Before and the
  // split view line up with the edit.
  const lensKey = JSON.stringify([adjustments.lens, adjustments.crop]);
  const beforeAdjustments = useMemo(
    () => ({ ...defaultAdjustments(), lens: adjustments.lens, crop: adjustments.crop }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [lensKey],
  );
  /** The crop tool is open: the image shows the whole straightened frame. */
  const [cropping, setCropping] = useState(false);
  /** The hand-drawn mask edited on the image, and the brush for new strokes. */
  const [manualActiveRaw, setManualActive] = useState<number | null>(null);
  const manualActive = manualActiveRaw !== null && manualActiveRaw < adjustments.manual.length ? manualActiveRaw : null;
  const [brush, setBrush] = useState<BrushSettings>({ size: 0.05, feather: 50, flow: 100, erase: false });
  /** A style or LUT previewed while the pointer rests on its card. */
  const [hoverAdjustments, setHoverAdjustments] = useState<Adjustments | null>(null);
  /** Split view divider (0–1 across), or null when off. */
  const [split, setSplit] = useState<number | null>(null);
  const [leftOpen, setLeftOpen] = useState(() => {
    try {
      return localStorage.getItem("epikos.sidebar.open") !== "false";
    } catch {
      return true;
    }
  });
  const toggleLeft = () =>
    setLeftOpen((o) => {
      try {
        localStorage.setItem("epikos.sidebar.open", String(!o));
      } catch {
        // Remembering the panel is a convenience only.
      }
      return !o;
    });
  const [luts, setLuts] = useState<LutInfo[]>([]);
  const refreshLuts = useCallback(() => {
    listLuts().then(setLuts, () => setLuts([]));
  }, []);
  useEffect(() => {
    if (inTauri) refreshLuts();
  }, [refreshLuts]);
  // Learned styles and presets (~/.epikos), and the AI Mentor's starting target.
  const [learned, setLearned] = useState<LearnedStyle[]>([]);
  const [presets, setPresets] = useState<Preset[]>([]);
  const refreshLibrary = useCallback(() => {
    learnedStyles().then(setLearned, () => setLearned([]));
    listPresets().then(setPresets, () => setPresets([]));
  }, []);
  useEffect(() => refreshLibrary(), [refreshLibrary]);
  const [mentorTarget, setMentorTargetState] = useState<string | null>(() => {
    try {
      return localStorage.getItem(MENTOR_TARGET_KEY);
    } catch {
      return null;
    }
  });
  const setMentorTarget = useCallback((id: string | null) => {
    setMentorTargetState(id);
    try {
      if (id) localStorage.setItem(MENTOR_TARGET_KEY, id);
      else localStorage.removeItem(MENTOR_TARGET_KEY);
    } catch {
      // Remembering the target is a convenience only.
    }
  }, []);
  // A deleted (or unknown) learned style falls back to Editorial.
  const activeTarget = mentorTarget && learned.some((s) => s.id === mentorTarget) ? mentorTarget : null;

  // While a slider or handle is dragged, render a lighter proxy (long side at most
  // DRAG_PROXY_SIDE device px) so feedback keeps up with the pointer; the full
  // resolution follows as soon as the drag ends. The browser scales the proxy up on
  // the GPU, so the image never jumps in size.
  const full = [Math.min(viewSize.w, MAX_PREVIEW_SIDE), Math.min(viewSize.h, MAX_PREVIEW_SIDE)] as const;
  const proxy = Math.min(1, DRAG_PROXY_SIDE / Math.max(1, full[0], full[1]));
  const previewSize = (
    history.dragging ? [Math.max(1, Math.round(full[0] * proxy)), Math.max(1, Math.round(full[1] * proxy))] : full
  ) as readonly [number, number];
  const shown = before ? beforeAdjustments : (hoverAdjustments ?? adjustments);
  const uncropped = useMemo(() => ({ ...shown, crop: defaultCrop() }), [shown]);
  const { preview, error: renderError, busy } = usePreview(
    info?.path ?? null,
    info ? (cropping ? uncropped : shown) : null,
    ...previewSize,
  );
  const { preview: beforePreview } = usePreview(
    split !== null && !cropping ? (info?.path ?? null) : null,
    info && split !== null ? beforeAdjustments : null,
    ...previewSize,
  );
  // The part of the frame on screen, and its size in actual pixels.
  const shownCrop = !cropping && cropIsActive(shown.crop) ? shown.crop : null;
  const pixelSize = info
    ? { w: info.width * (shownCrop?.width ?? 1), h: info.height * (shownCrop?.height ?? 1) }
    : null;
  useEffect(() => setHoverAdjustments(null), [info?.path]);

  const masks = useMasks(info?.path ?? null, adjustments);
  const { styles, thumbs } = useStyles(info?.path ?? null, adjustments);
  stylesRef.current = styles;
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
  /** Open a folder or a set of photos as the session; `first` is selected if listed. */
  const openSource = useCallback(
    async (next: Source, first?: string) => {
      await flushSave();
      setFolderError(null);
      try {
        const entries = await listSource(next);
        if (entries.length === 0 && next.kind === "files") {
          setFolderError(
            "None of those files is a photo EPIKOS RAW can open (RAW, DNG, JPEG, PNG or TIFF).",
          );
          return;
        }
        const token = ++folderToken.current;
        setSource(next);
        setFiles(entries);
        setStory(null);
        setInfo(null);
        setSelected(null);
        const pick = entries.find((e) => e.path === first) ?? entries[0];
        if (pick) void select(pick.path);
        // Story-arc grouping reads only previews and EXIF, so it's quick; the strip
        // regroups when it lands. A failure just leaves the plain strip.
        storySource(next).then(
          (arc) => token === folderToken.current && setStory(arc),
          (e) => console.warn("story arc:", e),
        );
      } catch (e) {
        setFolderError(String(e));
      }
    },
    [flushSave, select],
  );

  const chooseFolder = useCallback(async () => {
    const dir = await pickFolder();
    if (dir) await openSource({ kind: "folder", dir });
  }, [openSource]);

  const choosePhoto = useCallback(async () => {
    const path = await pickPhoto();
    if (path) await openSource({ kind: "files", paths: [path] }, path);
  }, [openSource]);

  // Photos (or folders) dropped anywhere on the window open as a session; unsupported
  // files among them are skipped.
  useEffect(() => {
    if (!inTauri) return;
    let unlisten: (() => void) | undefined;
    let alive = true;
    onFileDrag((e) => {
      if (e.type === "over") setDragging(true);
      else if (e.type === "leave") setDragging(false);
      else {
        setDragging(false);
        if (e.paths.length > 0) void openSource({ kind: "files", paths: e.paths }, e.paths[0]);
      }
    }).then(
      (u) => (alive ? (unlisten = u) : u()),
      (err) => console.warn("drag and drop unavailable:", err),
    );
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [openSource]);

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
    if (!source) return;
    listSource(source).then(setFiles, (e) => console.warn("refresh folder:", e));
  }, [source]);

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
    setCropping(false);
    setManualActive(null);
    setLastExport(null);
  }, [info?.path]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (exportOpen) return; // the modal owns the keyboard
      if (e.key === "Escape") {
        setPicking(null);
        setCropping(false);
        setManualActive(null);
      }
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
        if (e.shiftKey) void choosePhoto();
        else void chooseFolder();
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
      } else if (!inField && !mod && e.key.toLowerCase() === "c" && info) {
        setCropping((c) => !c);
      } else if (!inField && !mod && e.key.toLowerCase() === "y") {
        setSplit((v) => (v === null ? 0.5 : null));
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
  }, [history.undo, history.redo, history.commit, chooseFolder, choosePhoto, step, exportOpen, info, selectedLight]);

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
    <div className={`app${source ? "" : " is-empty"}${leftOpen ? "" : " left-collapsed"}`}>
      <header className="topbar">
        <div className="brand">
          EPIKOS <span>RAW</span>
        </div>
        {source && (
          <button
            type="button"
            className={`btn icon${leftOpen ? " is-active" : ""}`}
            onClick={toggleLeft}
            title={leftOpen ? "Hide the left panel" : "Show Presets & Styles, AI Mentor and History"}
            aria-pressed={leftOpen}
          >
            ◧
          </button>
        )}
        <button type="button" className="btn" onClick={() => void chooseFolder()} title="Open folder (⌘O)">
          Open folder…
        </button>
        <button type="button" className="btn" onClick={() => void choosePhoto()} title="Open photo (⇧⌘O)">
          Open photo…
        </button>
        <div className="topbar-title">
          {info ? (
            <>
              <strong>{info.name}</strong>
              <CaptureBadges info={info} />
            </>
          ) : (
            source && <span>{sourceLabel(source, files.length)}</span>
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
            className={`btn${split !== null ? " is-active" : ""}`}
            disabled={!info}
            onClick={() => setSplit((v) => (v === null ? 0.5 : null))}
            title="Split view: drag the divider to wipe between before and after (Y)"
            aria-pressed={split !== null}
          >
            Split
          </button>
          <button
            type="button"
            className="btn primary-outline"
            disabled={!info}
            onClick={() => setExportOpen(true)}
            title="Export TIFF, layered PSD, enhanced DNG, JPEG or PNG (⌘E)"
          >
            Export…
          </button>
          {lastExport && (
            <span className="export-status" title={lastExport.path}>
              <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden>
                <path d="M2.5 6.3l2.3 2.3 4.7-5" fill="none" stroke="currentColor" strokeWidth="1.5" />
              </svg>
              Exported {lastExport.label}
            </span>
          )}
          <button type="button" className="btn icon" onClick={() => setSettingsOpen(true)} title="Settings" aria-label="Settings">
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden>
              <circle cx="8" cy="8" r="2.4" fill="none" stroke="currentColor" strokeWidth="1.3" />
              <path
                d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"
                stroke="currentColor"
                strokeWidth="1.3"
                strokeLinecap="round"
              />
            </svg>
          </button>
          <SaveBadge status={save} />
        </div>
      </header>

      {settingsOpen && (
        <ErrorBoundary area="settings">
          <SettingsDialog settings={aiSettings} onChange={setAiSettings} onClose={() => setSettingsOpen(false)} />
        </ErrorBoundary>
      )}
      {exportOpen && info && (
        <ErrorBoundary area="the export dialog">
          <ExportDialog
            info={info}
            adjustments={adjustments}
            onClose={() => setExportOpen(false)}
            onExported={(r) => setLastExport({ path: r.path, label: exportLabel(r.format) })}
          />
        </ErrorBoundary>
      )}

      {source ? (
        <>
          <main className="stage">
            {info && (
              <ErrorBoundary area="the look prompt">
                <LookPromptBar
                  ref={promptRef}
                  path={info.path}
                  adjustments={adjustments}
                  commit={history.commit}
                  ai={aiSettings}
                />
              </ErrorBoundary>
            )}
            {files.length === 0 ? (
              <div className="viewer-status">No RAW, DNG, JPEG, PNG or TIFF files in this folder.</div>
            ) : (
              <ErrorBoundary area="the viewer">
                <Viewer
                  preview={preview}
                  busy={busy}
                  error={openError ?? renderError}
                  loading={loading}
                  showingBefore={before}
                  before={beforePreview}
                  split={split}
                  onSplit={setSplit}
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
                  frame={shownCrop}
                  pixelSize={pixelSize}
                  manualTool={
                    manualActive !== null && !cropping
                      ? {
                          shape: adjustments.manual[manualActive].shape,
                          brush,
                          onChange: (shape) =>
                            history.edit((a) => ({
                              ...a,
                              manual: a.manual.map((m, k) => (k === manualActive ? { ...m, shape } : m)),
                            })),
                          onEnd: history.endEdit,
                        }
                      : null
                  }
                  cropTool={
                    cropping
                      ? {
                          crop: adjustments.crop,
                          rotation: adjustments.lens.rotation,
                          onChange: (crop) => history.edit((a) => ({ ...a, crop })),
                          onRotate: (rotation) => history.edit((a) => ({ ...a, lens: { ...a.lens, rotation } })),
                          onEnd: history.endEdit,
                          onReset: () =>
                            history.commit((a) => ({
                              ...a,
                              crop: { ...defaultCrop(), aspect: a.crop.aspect },
                              lens: { ...a.lens, rotation: 0 },
                            })),
                          onDone: () => setCropping(false),
                        }
                      : null
                  }
                />
              </ErrorBoundary>
            )}
          </main>
          {leftOpen && (
            <ErrorBoundary area="the left panel">
              <LeftSidebar
                content={{
                  styles: info ? (
                    <StylePanel
                      styles={styles}
                      thumbs={thumbs}
                      adjustments={adjustments}
                      edit={history.edit}
                      endEdit={history.endEdit}
                      commit={history.commit}
                      onPreview={setHoverAdjustments}
                      luts={luts}
                      onLutsChanged={refreshLuts}
                      library={
                        <LearnedLibrary
                          info={info}
                          adjustments={adjustments}
                          commit={history.commit}
                          learned={learned}
                          presets={presets}
                          onChanged={refreshLibrary}
                          mentorTarget={activeTarget}
                          setMentorTarget={setMentorTarget}
                        />
                      }
                    />
                  ) : (
                    <p className="note pad">Select a photo to browse styles.</p>
                  ),
                  mentor: info ? (
                    <MentorPanel
                      info={info}
                      adjustments={adjustments}
                      commit={history.commit}
                      target={activeTarget}
                      targets={learned}
                      setTarget={setMentorTarget}
                    />
                  ) : (
                    <p className="note pad">Select a photo for the mentor&apos;s reading.</p>
                  ),
                  history: <HistoryPanel steps={history.steps} index={history.index} goTo={history.goTo} />,
                }}
              />
            </ErrorBoundary>
          )}
          <aside className="panel">
            <Histogram preview={preview} />
            <div className="panel-scroll">
              <ErrorBoundary area="the side panel">
                {info && <SceneCard info={info} preview={preview} />}
                {info && (
                  <BatchSyncCard
                    info={info}
                    adjustments={adjustments}
                    group={group}
                    flushSave={flushSave}
                    onChanged={refreshFiles}
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
                    cropping={cropping}
                    setCropping={setCropping}
                    manualActive={manualActive}
                    setManualActive={setManualActive}
                    brush={brush}
                    setBrush={setBrush}
                  />
                ) : (
                  <p className="note pad">Select a photo to start editing.</p>
                )}
              </ErrorBoundary>
            </div>
          </aside>
          <footer className="strip">
            <Filmstrip
              files={files}
              story={story}
              selected={selected}
              onSelect={(p) => void select(p)}
              masksLoaded={info && Object.keys(masks.masks).length > 0 ? info.path : null}
            />
          </footer>
        </>
      ) : (
        <div className="welcome">
          <h1>
            EPIKOS <span>RAW</span>
          </h1>
          <p>Open a folder or a photo to begin, or drop photos anywhere on this window.</p>
          <div className="welcome-actions">
            <button type="button" className="btn primary" onClick={() => void chooseFolder()}>
              Open folder…
            </button>
            <button type="button" className="btn primary-outline" onClick={() => void choosePhoto()}>
              Open photo…
            </button>
          </div>
          {folderError && <p className="error">{folderError}</p>}
          <p className="hint">ARW · CR3/CR2 · NEF · RAF · DNG · Apple ProRAW · JPEG · PNG · TIFF</p>
        </div>
      )}
      {source && folderError && (
        <div className="notice" role="alert">
          <span>{folderError}</span>
          <button type="button" className="btn icon" onClick={() => setFolderError(null)} aria-label="Dismiss">
            ×
          </button>
        </div>
      )}
      {dragging && (
        <div className="drop-overlay" aria-hidden>
          <div>
            <strong>Drop to open</strong>
            <span>RAW, DNG, JPEG, PNG or TIFF (photos or folders)</span>
          </div>
        </div>
      )}
    </div>
  );
}

/** Title-bar text for the session: the folder, the photo, or how many were opened. */
function sourceLabel(s: Source, count: number) {
  if (s.kind === "folder") return s.dir;
  if (s.paths.length === 1) return s.paths[0];
  return `${count} photo${count === 1 ? "" : "s"} opened`;
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

const exportLabel = (f: ExportFormat) =>
  ({ tiff: "TIFF", psd: "PSD", dng: "DNG", jpeg: "JPEG", png: "PNG" })[f];

/** Camera, lens and exposure as small badges in the toolbar. */
function CaptureBadges({ info }: { info: ImageInfo }) {
  const c = info.capture;
  const v = (r: [number, number] | null | undefined) => (r && r[1] ? r[0] / r[1] : 0);
  const items: { icon: React.ReactNode; text: string; title: string }[] = [];
  const camera = `${info.make} ${info.model}`.trim();
  if (camera) items.push({ icon: ICONS.camera, text: camera, title: `Camera · ${info.width}×${info.height}` });
  if (c.lensModel) items.push({ icon: ICONS.lens, text: c.lensModel, title: "Lens" });
  if (c.iso) items.push({ icon: null, text: `ISO ${c.iso}`, title: "ISO" });
  if (c.fNumber) items.push({ icon: null, text: `f/${Number(v(c.fNumber).toFixed(1))}`, title: "Aperture" });
  if (c.exposureTime) items.push({ icon: null, text: shutter(c.exposureTime), title: "Shutter speed" });
  if (c.focalLength) items.push({ icon: null, text: `${Number(v(c.focalLength).toFixed(0))} mm`, title: "Focal length" });
  return (
    <span className="capture-badges">
      {items.map((it) => (
        <span key={it.title} className="capture-badge" title={it.title}>
          {it.icon}
          {it.text}
        </span>
      ))}
    </span>
  );
}

const ICONS = {
  camera: (
    <svg viewBox="0 0 14 12" width="12" height="11" aria-hidden>
      <path d="M1.5 3.5h2.4l1.2-1.6h3.8l1.2 1.6h2.4v7h-11z" fill="none" stroke="currentColor" strokeWidth="1.1" />
      <circle cx="7" cy="6.8" r="2.1" fill="none" stroke="currentColor" strokeWidth="1.1" />
    </svg>
  ),
  lens: (
    <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden>
      <circle cx="6" cy="6" r="4.6" fill="none" stroke="currentColor" strokeWidth="1.1" />
      <circle cx="6" cy="6" r="2" fill="none" stroke="currentColor" strokeWidth="1.1" />
    </svg>
  ),
};
