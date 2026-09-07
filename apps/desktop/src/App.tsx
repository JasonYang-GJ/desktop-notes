import { EditorContent, useEditor } from "@tiptap/react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  type CSSProperties,
  type ClipboardEvent as ReactClipboardEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { richTextEditorExtensions } from "./basic-editor";
import { CalendarView, NOTE_DRAG_MIME } from "./calendar-view";
import { formatSelectedDate, monthFromDate } from "./calendar";
import {
  type BackupStatus,
  type FoundationError,
  type FoundationStatus,
  isFoundationError,
  readFoundationStatus,
} from "./foundation";
import { clearImageMemoryCache, imageDisplayUrl, importPastedImage } from "./images";
import {
  canonicalNoteDocument,
  changeNoteDate,
  createNote,
  deleteNote,
  getNote,
  isRichTextRecoveryError,
  listNoteCountsForMonth,
  listNotesForDate,
  localDateToday,
  sameBasicDocument,
  type DateChangeReceipt,
  type DateChangeUndoToken,
  type Note,
  type NoteSummary,
  type TiptapDocument,
  undoNoteDateChange,
  updateNoteContent,
} from "./notes";
import {
  assignTag,
  createTag,
  deleteTag,
  listRecentNotes,
  listTags,
  listTagsForNote,
  PinIntentCoordinator,
  removeTag,
  renameTag,
  setNotePinned,
  type Tag,
} from "./organization";
import { RichTextToolbar } from "./rich-text-toolbar";
import { QuickCaptureDialog } from "./quick-capture-dialog";
import { takeQuickCapture, type QuickCaptureSession } from "./quick-capture";
import { MAX_SEARCH_QUERY_CHARS, searchNotes, type SearchHit } from "./search";
import { validateRichTextDocument } from "./rich-text/contract";
import { sanitizePastedHtmlForPersistence } from "./rich-text/sanitizer";
import { SaveCoordinator, type EditableNoteDraft, type SaveState } from "./save-coordinator";
import { TagControls } from "./tag-controls";
import { ShortcutControl } from "./shortcut-control";
import {
  installWindowSaveLifecycle,
  type DesktopWindowLike,
} from "./window-save-lifecycle";
import { useWindowShell, type ThemeMode, type VisualState } from "./window-state";
import "./styles.css";

type StartupState =
  | { phase: "loading" }
  | { phase: "ready"; status: FoundationStatus }
  | { phase: "error"; error: FoundationError };

const ERROR_TITLES: Record<FoundationError["code"], string> = {
  KEY_UNAVAILABLE: "本地密钥不可用",
  DATABASE_WRONG_KEY: "数据库密钥不匹配",
  DATABASE_CORRUPTED: "数据库需要处理",
  DATABASE_SCHEMA_TOO_NEW: "检测到更新版本的数据库",
  MIGRATION_FAILED: "数据库升级已停止",
  DATA_ROOT_UNAVAILABLE: "本地存储不可用",
  DISK_FULL: "存储空间已满",
  WEBVIEW_UNAVAILABLE: "桌面界面不可用",
  VALIDATION_FAILED: "笔记格式不符合要求",
  NOTE_NOT_FOUND: "找不到笔记",
  REVISION_CONFLICT: "笔记已在其他位置更改",
  UNDO_EXPIRED: "撤销期限已过",
  TAG_NOT_FOUND: "找不到标签",
  TAG_NAME_CONFLICT: "标签名称已存在",
  IMAGE_REJECTED: "图片格式不受支持",
  IMAGE_TOO_LARGE: "图片过大",
  ASSET_NOT_FOUND: "找不到图片",
  ASSET_CORRUPTED: "无法验证图片",
  SHORTCUT_INVALID: "快捷键无效",
  SHORTCUT_CONFLICT: "快捷键已被占用",
  CAPTURE_SESSION_MISMATCH: "快速笔记会话已结束",
  BACKUP_BUSY: "备份正在进行",
  BACKUP_FAILED: "备份需要处理",
  BACKUP_CORRUPTED: "无法验证备份",
  BACKUP_UNSUPPORTED: "不支持此备份版本",
  INTERNAL_ERROR: "桌面笔记无法启动",
};

const SAFE_FALLBACK: FoundationError = {
  code: "INTERNAL_ERROR",
  message: "桌面笔记无法完成安全的本地启动。",
  recoverable: false,
};

export function App() {
  const [startup, setStartup] = useState<StartupState>({ phase: "loading" });
  const windowShell = useWindowShell();

  useEffect(() => {
    let active = true;
    async function loadFoundation() {
      try {
        const response = await readFoundationStatus();
        if (active) {
          setStartup(
            response.ok
              ? { phase: "ready", status: response.status }
              : { phase: "error", error: response.error },
          );
        }
      } catch (error: unknown) {
        if (active) {
          setStartup({
            phase: "error",
            error: isFoundationError(error) ? error : SAFE_FALLBACK,
          });
        }
      }
    }
    void loadFoundation();
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return undefined;
    let active = true;
    let removeListener: (() => void) | undefined;
    void listen<BackupStatus>("desktop-notes://backup-status", (event) => {
      if (!active) return;
      setStartup((current) =>
        current.phase === "ready"
          ? {
              phase: "ready",
              status: { ...current.status, backupStatus: event.payload },
            }
          : current,
      );
    })
      .then((unlisten) => {
        if (active) removeListener = unlisten;
        else unlisten();
      })
      .catch(() => undefined);
    return () => {
      active = false;
      removeListener?.();
    };
  }, []);

  return (
    <div
      className={`app-frame window-${windowShell.session.visualState}`}
      data-theme={windowShell.session.theme}
      data-material={windowShell.session.material}
    >
      <header className="topbar">
        <span className="wordmark">桌面笔记</span>
        <div className="topbar-actions">
          {startup.phase === "ready" && (
            <WindowControls
              visualState={windowShell.session.visualState}
              theme={windowShell.session.theme}
              onVisualState={windowShell.setVisualState}
              onTheme={windowShell.setTheme}
            />
          )}
          {startup.phase === "ready" && <ShortcutControl />}
          <span className="security-mark" aria-label="本地加密存储">
            <span className="security-dot" aria-hidden="true" />
            本地加密
          </span>
        </div>
      </header>

      <main className="workspace" aria-live="polite">
        {startup.phase === "loading" && <LoadingState />}
        {startup.phase === "ready" && (
          <CalendarWorkspace
            status={startup.status}
            visualState={windowShell.session.visualState}
            onVisualState={windowShell.setVisualState}
          />
        )}
        {startup.phase === "error" && <ErrorState error={startup.error} />}
      </main>
      {windowShell.error && <p className="window-shell-error" role="alert">{windowShell.error}</p>}
    </div>
  );
}

function WindowControls({
  visualState,
  theme,
  onVisualState,
  onTheme,
}: {
  visualState: VisualState;
  theme: ThemeMode;
  onVisualState: (state: VisualState) => void;
  onTheme: (theme: ThemeMode) => void;
}) {
  return (
    <div className="window-controls">
      <div className="state-switch" aria-label="窗口视图">
        {(["collapsed", "normal", "expanded"] as const).map((state) => (
          <button
            type="button"
            key={state}
            aria-label={`${visualStateLabel(state)}视图`}
            aria-pressed={visualState === state}
            onClick={() => onVisualState(state)}
          >
            {state === "collapsed" ? "—" : state === "normal" ? "▦" : "▥"}
          </button>
        ))}
      </div>
      <label className="theme-control">
        <span>主题</span>
        <select value={theme} onChange={(event) => onTheme(event.target.value as ThemeMode)}>
          <option value="system">跟随系统</option>
          <option value="light">浅色</option>
          <option value="dark">深色</option>
        </select>
      </label>
    </div>
  );
}

function LoadingState() {
  return (
    <section className="day-panel loading-panel" aria-label="正在安全启动">
      <p className="eyebrow">正在打开你的本地空间</p>
      <div className="loading-line loading-line-large" />
      <div className="loading-line" />
    </section>
  );
}

interface UndoNotice {
  noteId: string;
  targetDate: string;
  token: DateChangeUndoToken;
}

interface SelectedDateMove {
  noteId: string;
  lockInteraction: () => () => void;
  flush: () => Promise<boolean>;
  hasUnsavedChanges: () => boolean;
  move: (newDate: string) => Promise<boolean>;
  isInteractionLocked: () => boolean;
}

interface PreparedEditorTransition {
  transitionId: number;
  complete: () => void;
}

interface CalendarResizeStart {
  pointerId: number;
  startX: number;
  startWidth: number;
}

const DEFAULT_CALENDAR_PANE_WIDTH = 272;
const MIN_CALENDAR_PANE_WIDTH = 220;
const MAX_CALENDAR_PANE_WIDTH = 520;

function calendarPaneReservedWidth(viewportWidth: number, visualState: VisualState): number {
  if (viewportWidth <= 700) return 0;
  if (viewportWidth <= 1040) return 9 + 280;
  if (visualState === "expanded") return 9 + 268 + 360;
  return 9 + 300;
}

export function clampCalendarPaneWidth(
  proposed: number,
  workspaceWidth: number,
  visualState: VisualState = "normal",
  viewportWidth: number = workspaceWidth,
): number {
  const availableMaximum = Math.max(
    MIN_CALENDAR_PANE_WIDTH,
    Math.min(
      MAX_CALENDAR_PANE_WIDTH,
      workspaceWidth - calendarPaneReservedWidth(viewportWidth, visualState),
    ),
  );
  return Math.min(availableMaximum, Math.max(MIN_CALENDAR_PANE_WIDTH, proposed));
}

export function CalendarWorkspace({
  status,
  today: todayOverride,
  visualState = "normal",
  onVisualState,
}: {
  status: FoundationStatus;
  today?: string;
  visualState?: VisualState;
  onVisualState?: (state: VisualState) => void;
}) {
  const today = useMemo(() => todayOverride ?? localDateToday(), [todayOverride]);
  const [selectedDate, setSelectedDate] = useState(today);
  const [viewMode, setViewMode] = useState<"calendar" | "recent">("calendar");
  const [visibleMonth, setVisibleMonth] = useState(() => monthFromDate(today));
  const [notes, setNotes] = useState<NoteSummary[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchHits, setSearchHits] = useState<SearchHit[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string>();
  const [counts, setCounts] = useState<ReadonlyMap<string, number>>(new Map());
  const [selected, setSelected] = useState<Note>();
  const [loadingNotes, setLoadingNotes] = useState(true);
  const [noteError, setNoteError] = useState<string>();
  const [creating, setCreating] = useState(false);
  const [transitioning, setTransitioning] = useState(false);
  const [undoing, setUndoing] = useState(false);
  const [undoNotice, setUndoNotice] = useState<UndoNotice>();
  const [refreshVersion, setRefreshVersion] = useState(0);
  const [editorEpoch, setEditorEpoch] = useState(0);
  const [quickCapture, setQuickCapture] = useState<QuickCaptureSession>();
  const [quickCaptureNotice, setQuickCaptureNotice] = useState<string>();
  const [noteNotice, setNoteNotice] = useState<string>();
  const [calendarPaneWidth, setCalendarPaneWidth] = useState(DEFAULT_CALENDAR_PANE_WIDTH);
  const [resizingCalendar, setResizingCalendar] = useState(false);
  const selectedDateMoveRef = useRef<SelectedDateMove | undefined>(undefined);
  const undoTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const undoingRef = useRef(false);
  const listRequestRef = useRef(0);
  const editorTransitionRef = useRef(0);
  const editorReplacementRef = useRef(false);
  const captureRequestRef = useRef(false);
  const captureQueuedRef = useRef(false);
  const workspaceContentRef = useRef<HTMLDivElement>(null);
  const calendarResizeRef = useRef<CalendarResizeStart | undefined>(undefined);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return undefined;
    let active = true;
    let removeListener: (() => void) | undefined;
    const receiveCapture = async () => {
      captureQueuedRef.current = true;
      if (captureRequestRef.current) return;
      captureRequestRef.current = true;
      try {
        while (active && captureQueuedRef.current) {
          captureQueuedRef.current = false;
          const session = await takeQuickCapture();
          if (active && session) {
            setQuickCapture((current) => current ?? session);
            setQuickCaptureNotice(undefined);
            captureQueuedRef.current = false;
            break;
          }
        }
      } catch {
        if (active) setQuickCaptureNotice("无法安全打开快速笔记。");
      } finally {
        captureRequestRef.current = false;
      }
    };
    void listen("desktop-notes://quick-capture-available", () => {
      void receiveCapture();
    }).then((unlisten) => {
      if (!active) {
        unlisten();
        return;
      }
      removeListener = unlisten;
      void receiveCapture();
    }).catch(() => {
      if (active) setQuickCaptureNotice("快速笔记事件暂时无法送达。");
    });
    return () => {
      active = false;
      removeListener?.();
    };
  }, []);

  useEffect(() => {
    const requestId = ++listRequestRef.current;
    let active = true;
    setLoadingNotes(true);
    const listing = viewMode === "recent"
      ? listRecentNotes()
      : listNotesForDate(selectedDate);
    void listing
      .then((items) => {
        if (active && requestId === listRequestRef.current) setNotes(items);
      })
      .catch(() => {
        if (active && requestId === listRequestRef.current) {
          setNoteError(viewMode === "recent"
            ? "无法加载最近笔记。"
            : "无法加载这一天的笔记。");
        }
      })
      .finally(() => {
        if (active && requestId === listRequestRef.current) setLoadingNotes(false);
      });
    return () => {
      active = false;
    };
  }, [refreshVersion, selectedDate, viewMode]);

  useEffect(() => {
    let active = true;
    const normalized = searchQuery.trim();
    if (!normalized) {
      setSearchHits([]);
      setSearchError(undefined);
      setSearching(false);
      return undefined;
    }
    setSearching(true);
    setSearchError(undefined);
    const timer = window.setTimeout(() => {
      void searchNotes(normalized)
        .then((hits) => {
          if (active) setSearchHits(hits);
        })
        .catch(() => {
          if (active) {
            setSearchHits([]);
            setSearchError("无法完成搜索。");
          }
        })
        .finally(() => {
          if (active) setSearching(false);
        });
    }, 200);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [refreshVersion, searchQuery]);

  useEffect(() => {
    let active = true;
    void listNoteCountsForMonth(visibleMonth)
      .then((items) => {
        if (active) setCounts(new Map(items.map((item) => [item.noteDate, item.count])));
      })
      .catch(() => {
        if (active) setNoteError("无法加载日历中的笔记数量。");
      });
    return () => {
      active = false;
    };
  }, [refreshVersion, visibleMonth]);

  useEffect(() => () => {
    if (undoTimerRef.current !== undefined) clearTimeout(undoTimerRef.current);
  }, []);

  useEffect(() => {
    const workspace = workspaceContentRef.current;
    if (!workspace || visualState === "collapsed") return undefined;
    const constrainWidth = () => {
      const width = workspace.getBoundingClientRect().width || workspace.clientWidth || 1180;
      setCalendarPaneWidth((current) => clampCalendarPaneWidth(
        current,
        width,
        visualState,
        window.innerWidth,
      ));
    };
    constrainWidth();
    const ResizeObserverConstructor = (window as unknown as {
      ResizeObserver?: typeof ResizeObserver;
    }).ResizeObserver;
    const observer = ResizeObserverConstructor
      ? new ResizeObserverConstructor(constrainWidth)
      : undefined;
    observer?.observe(workspace);
    window.addEventListener("resize", constrainWidth);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", constrainWidth);
    };
  }, [visualState]);

  useEffect(() => {
    const desktopWindow = "__TAURI_INTERNALS__" in window
      ? getCurrentWindow() as unknown as DesktopWindowLike
      : undefined;
    const lifecycle = installWindowSaveLifecycle({
      desktopWindow,
      getEditor: () => selectedDateMoveRef.current,
      onFailure: setNoteError,
    });
    return lifecycle.dispose;
  }, []);

  const dismissUndoNotice = useCallback(() => {
    setUndoNotice(undefined);
    if (undoTimerRef.current !== undefined) {
      clearTimeout(undoTimerRef.current);
      undoTimerRef.current = undefined;
    }
  }, []);

  const beginEditorTransition = useCallback(() => {
    const transitionId = ++editorTransitionRef.current;
    dismissUndoNotice();
    setNoteNotice(undefined);
    return transitionId;
  }, [dismissUndoNotice]);

  const calendarResizeMaximum = useCallback(() => {
    const width = workspaceContentRef.current?.getBoundingClientRect().width
      || workspaceContentRef.current?.clientWidth
      || 1180;
    return clampCalendarPaneWidth(Number.POSITIVE_INFINITY, width, visualState, window.innerWidth);
  }, [visualState]);

  function beginCalendarResize(event: ReactPointerEvent<HTMLDivElement>) {
    if (event.button !== 0 || visualState === "collapsed") return;
    calendarResizeRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: calendarPaneWidth,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setResizingCalendar(true);
    event.preventDefault();
  }

  function resizeCalendar(event: ReactPointerEvent<HTMLDivElement>) {
    const start = calendarResizeRef.current;
    if (!start || start.pointerId !== event.pointerId) return;
    setCalendarPaneWidth(clampCalendarPaneWidth(
      start.startWidth + event.clientX - start.startX,
      workspaceContentRef.current?.getBoundingClientRect().width
        || workspaceContentRef.current?.clientWidth
        || 1180,
      visualState,
      window.innerWidth,
    ));
  }

  function finishCalendarResize(event: ReactPointerEvent<HTMLDivElement>) {
    if (calendarResizeRef.current?.pointerId !== event.pointerId) return;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    calendarResizeRef.current = undefined;
    setResizingCalendar(false);
  }

  function resizeCalendarWithKeyboard(event: ReactKeyboardEvent<HTMLDivElement>) {
    const step = event.shiftKey ? 32 : 16;
    let next: number | undefined;
    if (event.key === "ArrowLeft") next = calendarPaneWidth - step;
    if (event.key === "ArrowRight") next = calendarPaneWidth + step;
    if (event.key === "Home") next = MIN_CALENDAR_PANE_WIDTH;
    if (event.key === "End") next = calendarResizeMaximum();
    if (next === undefined) return;
    event.preventDefault();
    setCalendarPaneWidth(clampCalendarPaneWidth(
      next,
      workspaceContentRef.current?.getBoundingClientRect().width
        || workspaceContentRef.current?.clientWidth
        || 1180,
      visualState,
      window.innerWidth,
    ));
  }

  const prepareEditorTransition = useCallback(async (): Promise<PreparedEditorTransition | undefined> => {
    if (undoingRef.current || editorReplacementRef.current) return undefined;
    editorReplacementRef.current = true;
    setTransitioning(true);
    const currentEditor = selectedDateMoveRef.current;
    if (currentEditor?.isInteractionLocked()) {
      editorReplacementRef.current = false;
      setTransitioning(false);
      return undefined;
    }
    const unlockInteraction = currentEditor?.lockInteraction();
    const complete = () => {
      unlockInteraction?.();
      editorReplacementRef.current = false;
      setTransitioning(false);
    };
    if (currentEditor && !await currentEditor.flush()) {
      setNoteError("当前笔记无法保存，因此仍保持打开。请重试。");
      complete();
      return undefined;
    }
    return { transitionId: beginEditorTransition(), complete };
  }, [beginEditorTransition]);

  async function chooseDate(date: string) {
    if (undoingRef.current || editorReplacementRef.current) return;
    if (date === selectedDate && viewMode === "calendar") return;
    const prepared = await prepareEditorTransition();
    if (!prepared) return;
    try {
      setSelectedDate(date);
      setViewMode("calendar");
      setVisibleMonth(monthFromDate(date));
      setSelected(undefined);
      setNoteError(undefined);
    } finally {
      prepared.complete();
    }
  }

  async function showRecent() {
    if (viewMode === "recent" || undoingRef.current || editorReplacementRef.current) return;
    const prepared = await prepareEditorTransition();
    if (!prepared) return;
    try {
      setViewMode("recent");
      setSelected(undefined);
      setNoteError(undefined);
    } finally {
      prepared.complete();
    }
  }

  async function addNote() {
    if (undoingRef.current || editorReplacementRef.current) return;
    const prepared = await prepareEditorTransition();
    if (!prepared) return;
    const transitionId = prepared.transitionId;
    setCreating(true);
    setNoteError(undefined);
    try {
      const note = await createNote(selectedDate);
      if (transitionId !== editorTransitionRef.current || undoingRef.current) return;
      setNotes((current) => [summaryFrom(note), ...current]);
      setSelected(note);
      setRefreshVersion((version) => version + 1);
    } catch {
      if (transitionId === editorTransitionRef.current && !undoingRef.current) {
        setNoteError("无法新建笔记。");
      }
    } finally {
      setCreating(false);
      prepared.complete();
    }
  }

  async function openNote(noteId: string) {
    if (undoingRef.current || editorReplacementRef.current || selected?.id === noteId) return;
    const prepared = await prepareEditorTransition();
    if (!prepared) return;
    const transitionId = prepared.transitionId;
    setNoteError(undefined);
    try {
      const note = await getNote(noteId);
      if (transitionId === editorTransitionRef.current && !undoingRef.current) {
        setSelected(note);
      }
    } catch (error: unknown) {
      if (transitionId === editorTransitionRef.current && !undoingRef.current) {
        setNoteError(
          isRichTextRecoveryError(error)
            ? "这条笔记含有不受支持或损坏的富文本；已保存的正文未被改动。"
            : "无法打开所选笔记。",
        );
      }
    } finally {
      prepared.complete();
    }
  }

  const updateSummary = useCallback((
    id: string,
    title: string,
    revision: number,
    updatedAtMs = Date.now(),
  ) => {
    setNotes((current) => sortVisibleNotes(current.map((note) => (
      note.id === id ? { ...note, title, revision, updatedAtMs } : note
    )), viewMode));
  }, [viewMode]);

  const applyOrganizationChange = useCallback((saved: Note) => {
    setSelected((current) => current?.id === saved.id ? saved : current);
    setNotes((current) => sortVisibleNotes(current.map((note) => (
      note.id === saved.id ? summaryFrom(saved) : note
    )), viewMode));
  }, [viewMode]);

  const applyNoteDeletion = useCallback((noteId: string) => {
    dismissUndoNotice();
    setSelected((current) => current?.id === noteId ? undefined : current);
    setNotes((current) => current.filter((note) => note.id !== noteId));
    setSearchHits((current) => current.filter((hit) => hit.id !== noteId));
    setRefreshVersion((version) => version + 1);
    setNoteError(undefined);
    setNoteNotice("笔记已删除。");
  }, [dismissUndoNotice]);

  const applyDateChange = useCallback((receipt: DateChangeReceipt, transitionId: number) => {
    setRefreshVersion((version) => version + 1);
    if (transitionId !== editorTransitionRef.current || undoingRef.current) return;
    setSelected(receipt.note);
    setSelectedDate(receipt.note.noteDate);
    setVisibleMonth(monthFromDate(receipt.note.noteDate));
    setEditorEpoch((epoch) => epoch + 1);
    setUndoNotice({
      noteId: receipt.note.id,
      targetDate: receipt.note.noteDate,
      token: receipt.undoToken,
    });
    if (undoTimerRef.current !== undefined) clearTimeout(undoTimerRef.current);
    undoTimerRef.current = setTimeout(() => {
      setUndoNotice((current) => (
        current?.token.tokenId === receipt.undoToken.tokenId ? undefined : current
      ));
      undoTimerRef.current = undefined;
    }, Math.max(0, receipt.undoToken.expiresAtMs - Date.now()));
  }, []);

  const registerSelectedDateMove = useCallback((move?: SelectedDateMove) => {
    selectedDateMoveRef.current = move;
  }, []);

  async function moveNoteToDate(noteId: string, newDate: string) {
    if (undoingRef.current || editorReplacementRef.current) return;
    const summary = notes.find((item) => item.id === noteId);
    if (!summary || summary.noteDate === newDate) return;
    setNoteError(undefined);
    let transitionId: number | undefined;
    let prepared: PreparedEditorTransition | undefined;
    try {
      if (selectedDateMoveRef.current?.noteId === noteId) {
        await selectedDateMoveRef.current.move(newDate);
        return;
      }
      prepared = await prepareEditorTransition();
      if (!prepared) return;
      transitionId = prepared.transitionId;
      const receipt = await changeNoteDate(noteId, newDate, summary.revision);
      applyDateChange(receipt, transitionId);
    } catch {
      if (transitionId === undefined
        || (transitionId === editorTransitionRef.current && !undoingRef.current)) {
        setNoteError("无法更改笔记日期。");
      }
    } finally {
      prepared?.complete();
    }
  }

  async function undoDateChange() {
    if (!undoNotice || undoingRef.current) return;
    const notice = undoNotice;
    undoingRef.current = true;
    ++editorTransitionRef.current;
    setUndoing(true);
    setNoteError(undefined);
    const selectedDateMove = selectedDateMoveRef.current;
    const unlockInteraction = selectedDateMove?.lockInteraction();
    try {
      if (selectedDateMove && !await selectedDateMove.flush()) {
        setNoteError("请先保存当前编辑，再撤销日期更改。");
        return;
      }
      const restored = await undoNoteDateChange(notice.token.tokenId);
      setSelected(restored);
      setSelectedDate(restored.noteDate);
      setVisibleMonth(monthFromDate(restored.noteDate));
      setEditorEpoch((epoch) => epoch + 1);
      setRefreshVersion((version) => version + 1);
      dismissUndoNotice();
    } catch (error: unknown) {
      dismissUndoNotice();
      setNoteError(
        isFoundationError(error) && error.code === "REVISION_CONFLICT"
          ? "笔记内容已经变化，无法再撤销日期更改。"
          : "无法撤销日期更改。",
      );
    } finally {
      unlockInteraction?.();
      undoingRef.current = false;
      setUndoing(false);
    }
  }

  const selectedHeading = selectedDate === today
    ? "今天"
    : new Intl.DateTimeFormat("zh-CN", { month: "long", day: "numeric" })
      .format(dateAtNoon(selectedDate));
  const searchActive = searchQuery.trim().length > 0;

  const todayCount = counts.get(today) ?? (selectedDate === today && viewMode === "calendar" ? notes.length : 0);

  return (
    <section className={`day-panel note-workspace calendar-workspace is-${visualState}`}>
      <div className="collapsed-summary" aria-hidden={visualState !== "collapsed"}>
        <span className="collapsed-rule" aria-hidden="true" />
        <div>
          <p>{formatSelectedDate(today)}</p>
          <strong>今天</strong>
        </div>
        <span className="collapsed-count">{todayCount} 条笔记</span>
        <button type="button" aria-label="打开标准视图" onClick={() => onVisualState?.("normal")}>打开</button>
      </div>
      <div
        className="workspace-content"
        aria-hidden={visualState === "collapsed"}
        ref={workspaceContentRef}
        style={{ "--calendar-pane-width": `${calendarPaneWidth}px` } as CSSProperties}
      >
      <CalendarView
        month={visibleMonth}
        selectedDate={selectedDate}
        today={today}
        counts={counts}
        onMonthChange={setVisibleMonth}
        onSelectDate={(date) => void chooseDate(date)}
        onDropNote={(noteId, date) => void moveNoteToDate(noteId, date)}
      />
      <div
        className={resizingCalendar ? "calendar-notes-resizer is-resizing" : "calendar-notes-resizer"}
        role="separator"
        aria-label="调整日历和笔记列表宽度"
        aria-orientation="vertical"
        aria-valuemin={MIN_CALENDAR_PANE_WIDTH}
        aria-valuemax={calendarResizeMaximum()}
        aria-valuenow={calendarPaneWidth}
        tabIndex={visualState === "collapsed" ? -1 : 0}
        title="拖动调整日历宽度，双击恢复默认宽度"
        onPointerDown={beginCalendarResize}
        onPointerMove={resizeCalendar}
        onPointerUp={finishCalendarResize}
        onPointerCancel={finishCalendarResize}
        onKeyDown={resizeCalendarWithKeyboard}
        onDoubleClick={() => setCalendarPaneWidth(DEFAULT_CALENDAR_PANE_WIDTH)}
      />
      <aside className="notes-rail">
        <label className="search-entry">
          <span>搜索笔记</span>
          <input
            aria-label="搜索笔记"
            autoComplete="off"
            maxLength={MAX_SEARCH_QUERY_CHARS}
            value={searchQuery}
            onChange={(event) => setSearchQuery(event.target.value)}
            placeholder="标题、正文或标签"
          />
        </label>
        {searchActive ? (
          <nav className="search-results" aria-label="搜索结果" aria-busy={searching}>
            {searching && <p className="rail-message">正在搜索…</p>}
            {!searching && searchError && <p className="note-error" role="alert">{searchError}</p>}
            {!searching && !searchError && searchHits.length === 0 && (
              <p className="rail-message">没有匹配的笔记。</p>
            )}
            {!searching && !searchError && searchHits.map((hit) => (
              <button
                type="button"
                className={selected?.id === hit.id ? "note-row search-hit selected" : "note-row search-hit"}
                key={hit.id}
                disabled={undoing || transitioning}
                onClick={() => void openNote(hit.id)}
              >
                <strong>{highlightText(hit.title || "无标题", searchQuery)}</strong>
                {hit.snippet && <span className="search-snippet">{highlightText(hit.snippet, searchQuery)}</span>}
                <small>
                  {formatShortDate(hit.noteDate)}
                  {hit.matchingTags.length > 0 && (
                    <span className="search-tags">
                      {" · "}
                      {hit.matchingTags.map((tag, index) => (
                        <span key={tag.id}>
                          {index > 0 && ", "}
                          {highlightText(tag.name, searchQuery)}
                        </span>
                      ))}
                    </span>
                  )}
                </small>
              </button>
            ))}
          </nav>
        ) : <>
          <div className="view-switch" aria-label="笔记视图">
          <button
            type="button"
            aria-pressed={viewMode === "calendar"}
            disabled={undoing || transitioning}
            onClick={() => void chooseDate(selectedDate)}
          >
            日历
          </button>
          <button
            type="button"
            aria-pressed={viewMode === "recent"}
            disabled={undoing || transitioning}
            onClick={() => void showRecent()}
          >
            最近
          </button>
        </div>
        <div className="day-heading">
          <div>
            <p className="eyebrow">{viewMode === "recent" ? "所有日期" : formatSelectedDate(selectedDate)}</p>
            <h1>{viewMode === "recent" ? "最近笔记" : selectedHeading}</h1>
          </div>
          <span className="view-state">{visualStateLabel(visualState)}</span>
        </div>

        {viewMode === "calendar" ? (
          <button
            className="new-note"
            type="button"
            aria-label="新建笔记"
            disabled={creating || undoing || transitioning}
            onClick={() => void addNote()}
          >
            <span aria-hidden="true">+</span>
            {creating ? "正在创建…" : "新建笔记"}
          </button>
        ) : <div className="recent-spacer" />}

        <nav className="note-list" aria-label={viewMode === "recent" ? "最近笔记" : `${selectedHeading}的笔记`}>
          {loadingNotes && <p className="rail-message">正在加载笔记…</p>}
          {!loadingNotes && notes.length === 0 && (
            <p className="rail-message">{viewMode === "recent" ? "还没有最近笔记。" : "这一天还没有笔记。"}</p>
          )}
          {notes.map((note) => (
            <button
              className={selected?.id === note.id ? "note-row selected" : "note-row"}
              type="button"
              key={note.id}
              disabled={undoing || transitioning}
              draggable={!undoing && !transitioning}
              onDragStart={(event) => {
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData(NOTE_DRAG_MIME, note.id);
              }}
              onClick={() => void openNote(note.id)}
            >
              <span className="note-row-title">
                <strong>{note.title || "无标题"}</strong>
                {note.isPinned && <span className="pin-badge">已置顶</span>}
              </span>
              <small>
                {viewMode === "recent" && `${formatShortDate(note.noteDate)} · `}
                修订版本 {note.revision}
              </small>
            </button>
          ))}
        </nav>
        </>}

        {noteError && <p className="note-error" role="alert">{noteError}</p>}
        {noteNotice && <p className="note-success" role="status">{noteNotice}</p>}
        {undoNotice && (
          <div className="undo-notice" role="status">
            <span>已移动到 {formatShortDate(undoNotice.targetDate)}。</span>
            <button
              type="button"
              aria-label="撤销日期更改"
              disabled={undoing}
              onClick={() => void undoDateChange()}
            >
              {undoing ? "正在撤销…" : "撤销"}
            </button>
          </div>
        )}
        <FoundationFooter status={status} />
      </aside>

      <div className="editor-pane">
        {selected ? (
          <NoteEditor
            key={`${selected.id}:${editorEpoch}`}
            note={selected}
            onSummaryChange={updateSummary}
            onDateChanged={applyDateChange}
            onDateChangeStarted={beginEditorTransition}
            registerDateMove={registerSelectedDateMove}
            onOrganizationChanged={applyOrganizationChange}
            onDeleted={applyNoteDeletion}
          />
        ) : (
          <div className="editor-empty">
            <span className="empty-rule" aria-hidden="true" />
            <p className="empty-title">给今天留一处安静角落。</p>
            <p className="empty-copy">新建一条笔记，或从左侧选择一条。</p>
          </div>
        )}
      </div>
      </div>
      {quickCaptureNotice && (
        <p className="capture-launch-error" role="alert">{quickCaptureNotice}</p>
      )}
      {quickCapture && (
        <QuickCaptureDialog
          key={quickCapture.id}
          session={quickCapture}
          defaultDate={today}
          onCancelled={() => setQuickCapture(undefined)}
          onSaved={(_note, warning) => {
            setQuickCapture(undefined);
            setRefreshVersion((version) => version + 1);
            setQuickCaptureNotice(warning ?? "快速笔记已保存。");
          }}
        />
      )}
    </section>
  );
}

function NoteEditor({
  note,
  onSummaryChange,
  onDateChanged,
  onDateChangeStarted,
  registerDateMove,
  onOrganizationChanged,
  onDeleted,
}: {
  note: Note;
  onSummaryChange: (id: string, title: string, revision: number, updatedAtMs?: number) => void;
  onDateChanged: (receipt: DateChangeReceipt, transitionId: number) => void;
  onDateChangeStarted: () => number;
  registerDateMove: (move?: SelectedDateMove) => void;
  onOrganizationChanged: (saved: Note) => void;
  onDeleted: (noteId: string) => void;
}) {
  const [title, setTitle] = useState(note.title);
  const [noteDate, setNoteDate] = useState(note.noteDate);
  const [revision, setRevision] = useState(note.revision);
  const [saveState, setSaveState] = useState<SaveState>("Saved");
  const [dateChanging, setDateChanging] = useState(false);
  const [interactionLocked, setInteractionLocked] = useState(false);
  const [dateError, setDateError] = useState<string>();
  const [editorFeedback, setEditorFeedback] = useState<string>();
  const [tags, setTags] = useState<Tag[]>([]);
  const [assignedTagIds, setAssignedTagIds] = useState<ReadonlySet<string>>(new Set());
  const [organizationBusy, setOrganizationBusy] = useState(false);
  const [pinBusy, setPinBusy] = useState(false);
  const [displayedPinned, setDisplayedPinned] = useState(note.isPinned);
  const [organizationError, setOrganizationError] = useState<string>();
  const [deleteConfirmation, setDeleteConfirmation] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string>();
  const interactionLockedRef = useRef(false);
  const interactionLockCountRef = useRef(0);
  const confirmedPinnedRef = useRef(note.isPinned);
  const pinUnlockRef = useRef<(() => void) | undefined>(undefined);
  const organizationPendingRef = useRef<Promise<boolean> | undefined>(undefined);
  const imageImportPendingRef = useRef<Promise<boolean> | undefined>(undefined);
  const latestContentSaveRef = useRef<Note | undefined>(undefined);
  const draftRef = useRef<EditableNoteDraft>({
    noteId: note.id,
    noteDate: note.noteDate,
    title: note.title,
    bodyJson: note.bodyJson,
    baseRevision: note.revision,
  });
  const coordinator = useMemo(() => new SaveCoordinator(
    async (draft, clientChangeId) => {
      const saved = await updateNoteContent({ ...draft, clientChangeId });
      latestContentSaveRef.current = saved;
      return { revision: saved.revision };
    },
    setSaveState,
    (nextRevision) => {
      draftRef.current = { ...draftRef.current, baseRevision: nextRevision };
      setRevision(nextRevision);
      onSummaryChange(
        note.id,
        draftRef.current.title,
        nextRevision,
        latestContentSaveRef.current?.updatedAtMs,
      );
      latestContentSaveRef.current = undefined;
    },
  ), [note.id, onSummaryChange]);
  const editorExtensions = useMemo(
    () => richTextEditorExtensions((assetId) => imageDisplayUrl(note.id, assetId)),
    [note.id],
  );

  useEffect(() => () => clearImageMemoryCache(), [note.id]);
  useEffect(() => () => coordinator.dispose(), [coordinator]);
  useEffect(() => {
    function saveShortcut(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        if (!interactionLockedRef.current) void coordinator.flush();
      }
    }
    window.addEventListener("keydown", saveShortcut);
    return () => window.removeEventListener("keydown", saveShortcut);
  }, [coordinator]);

  const editor = useEditor({
    extensions: editorExtensions,
    content: note.bodyJson,
    immediatelyRender: false,
    editorProps: {
      attributes: {
        class: "basic-editor",
        "aria-label": "笔记正文",
      },
    },
    onUpdate: ({ editor: currentEditor }) => {
      const bodyJson = canonicalNoteDocument(currentEditor.getJSON() as TiptapDocument);
      if (sameBasicDocument(bodyJson, draftRef.current.bodyJson)) return;
      draftRef.current = { ...draftRef.current, bodyJson };
      coordinator.changed(draftRef.current);
    },
  });

  useEffect(() => {
    editor?.setEditable(!dateChanging && !interactionLocked);
  }, [dateChanging, editor, interactionLocked]);

  const lockInteraction = useCallback(() => {
    interactionLockCountRef.current += 1;
    if (interactionLockCountRef.current === 1) {
      interactionLockedRef.current = true;
      editor?.setEditable(false);
      setInteractionLocked(true);
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      interactionLockCountRef.current = Math.max(0, interactionLockCountRef.current - 1);
      if (interactionLockCountRef.current !== 0) return;
      interactionLockedRef.current = false;
      editor?.setEditable(!dateChanging);
      setInteractionLocked(false);
    };
  }, [dateChanging, editor]);

  useEffect(() => {
    let active = true;
    void Promise.all([listTags(), listTagsForNote(note.id)])
      .then(([catalog, assigned]) => {
        if (!active) return;
        setTags(sortTags(catalog));
        setAssignedTagIds(new Set(assigned.map((tag) => tag.id)));
      })
      .catch(() => {
        if (active) setOrganizationError("无法加载标签。");
      });
    return () => {
      active = false;
    };
  }, [note.id]);

  const applyMetadata = useCallback((saved: Note) => {
    draftRef.current = { ...draftRef.current, baseRevision: saved.revision };
    coordinator.rebase(saved.noteDate, saved.revision);
    confirmedPinnedRef.current = saved.isPinned;
    setRevision(saved.revision);
    onOrganizationChanged(saved);
  }, [coordinator, onOrganizationChanged]);

  const pinCoordinator = useMemo(() => new PinIntentCoordinator({
    // Date-transition state can recreate this coordinator without remounting
    // the editor. Seed it from the latest confirmed backend state so a Pin
    // followed by a date move cannot invert the next intent.
    initialPinned: confirmedPinnedRef.current,
    flush: () => coordinator.flush(),
    save: (nextPinned) => setNotePinned(
      note.id,
      nextPinned,
      draftRef.current.baseRevision,
    ),
    apply: applyMetadata,
    onBusyChange: (busy) => {
      setPinBusy(busy);
      if (busy) {
        pinUnlockRef.current = lockInteraction();
      } else {
        pinUnlockRef.current?.();
        pinUnlockRef.current = undefined;
        setDisplayedPinned(confirmedPinnedRef.current);
      }
    },
    onError: (error) => {
      setOrganizationError(
        isFoundationError(error) && error.code === "REVISION_CONFLICT"
          ? "笔记已经变化，无法更新置顶状态。"
          : "无法保存置顶状态。",
      );
    },
  }), [applyMetadata, coordinator, lockInteraction, note.id]);

  function trackOrganization(operation: () => Promise<boolean>): Promise<boolean> {
    if (organizationPendingRef.current) return organizationPendingRef.current;
    const pending = operation();
    organizationPendingRef.current = pending;
    const clear = () => {
      if (organizationPendingRef.current === pending) organizationPendingRef.current = undefined;
    };
    void pending.then(clear, clear);
    return pending;
  }

  async function changeTagAssignment(tag: Tag, assigned: boolean): Promise<boolean> {
    if (interactionLockedRef.current) return false;
    const unlock = lockInteraction();
    setOrganizationBusy(true);
    setOrganizationError(undefined);
    try {
      if (!await coordinator.flush()) {
        setOrganizationError("请先保存当前编辑，再更改标签。");
        return false;
      }
      const saved = assigned
        ? await removeTag(note.id, tag.id, draftRef.current.baseRevision)
        : await assignTag(note.id, tag.id, draftRef.current.baseRevision);
      applyMetadata(saved);
      setAssignedTagIds((current) => {
        const next = new Set(current);
        if (assigned) next.delete(tag.id);
        else next.add(tag.id);
        return next;
      });
      return true;
    } catch (error: unknown) {
      setOrganizationError(
        isFoundationError(error) && error.code === "REVISION_CONFLICT"
          ? "笔记已经变化，无法更新标签。"
          : "无法保存标签更改。",
      );
      return false;
    } finally {
      setOrganizationBusy(false);
      unlock();
    }
  }

  async function createCatalogTag(name: string): Promise<boolean> {
    setOrganizationBusy(true);
    setOrganizationError(undefined);
    try {
      const created = await createTag(name);
      setTags((current) => sortTags([...current, created]));
      return true;
    } catch (error: unknown) {
      setOrganizationError(tagMutationMessage(error));
      return false;
    } finally {
      setOrganizationBusy(false);
    }
  }

  async function renameCatalogTag(tag: Tag, name: string): Promise<boolean> {
    setOrganizationBusy(true);
    setOrganizationError(undefined);
    try {
      const renamed = await renameTag(tag.id, name);
      setTags((current) => sortTags(current.map((item) => item.id === tag.id ? renamed : item)));
      return true;
    } catch (error: unknown) {
      setOrganizationError(tagMutationMessage(error));
      return false;
    } finally {
      setOrganizationBusy(false);
    }
  }

  async function deleteCatalogTag(tag: Tag): Promise<boolean> {
    setOrganizationBusy(true);
    setOrganizationError(undefined);
    try {
      await deleteTag(tag.id);
      setTags((current) => current.filter((item) => item.id !== tag.id));
      setAssignedTagIds((current) => {
        const next = new Set(current);
        next.delete(tag.id);
        return next;
      });
      return true;
    } catch (error: unknown) {
      setOrganizationError(tagMutationMessage(error));
      return false;
    } finally {
      setOrganizationBusy(false);
    }
  }

  const flushAll = useCallback(async () => {
    await pinCoordinator.waitForIdle();
    if (organizationPendingRef.current) await organizationPendingRef.current;
    if (imageImportPendingRef.current) await imageImportPendingRef.current;
    return coordinator.flush();
  }, [coordinator, pinCoordinator]);

  async function confirmNoteDeletion() {
    if (interactionLockedRef.current) return;
    const unlock = lockInteraction();
    let deleted = false;
    setDeleting(true);
    setDeleteError(undefined);
    try {
      if (!await flushAll()) {
        setDeleteError("请先保存当前编辑，再删除笔记。");
        return;
      }
      await deleteNote(note.id, draftRef.current.baseRevision);
      deleted = true;
      setDeleting(false);
      unlock();
      onDeleted(note.id);
    } catch (error: unknown) {
      setDeleteError(
        isFoundationError(error) && error.code === "REVISION_CONFLICT"
          ? "笔记已经变化，请重新打开后再删除。"
          : "无法删除这条笔记。",
      );
    } finally {
      if (!deleted) {
        setDeleting(false);
        unlock();
      }
    }
  }

  const moveDate = useCallback(async (newDate: string) => {
    if (interactionLockedRef.current) return false;
    if (newDate === draftRef.current.noteDate) return true;
    const unlockInteraction = lockInteraction();
    const transitionId = onDateChangeStarted();
    setDateChanging(true);
    setDateError(undefined);
    try {
      if (!await coordinator.flush()) {
        setDateError("请先保存当前编辑，再更改日期。");
        return false;
      }
      const receipt = await changeNoteDate(
        note.id,
        newDate,
        draftRef.current.baseRevision,
      );
      draftRef.current = {
        ...draftRef.current,
        noteDate: receipt.note.noteDate,
        baseRevision: receipt.note.revision,
      };
      coordinator.rebase(receipt.note.noteDate, receipt.note.revision);
      setNoteDate(receipt.note.noteDate);
      setRevision(receipt.note.revision);
      onDateChanged(receipt, transitionId);
      return true;
    } catch (error: unknown) {
      setDateError(
        isFoundationError(error) && error.code === "REVISION_CONFLICT"
          ? "笔记已经变化，无法更新日期。"
          : "无法更改笔记日期。",
      );
      return false;
    } finally {
      setDateChanging(false);
      unlockInteraction();
    }
  }, [coordinator, lockInteraction, note.id, onDateChanged, onDateChangeStarted]);

  useLayoutEffect(() => {
    registerDateMove({
      noteId: note.id,
      lockInteraction,
      flush: flushAll,
      hasUnsavedChanges: () => coordinator.hasUnsavedChanges()
        || pinCoordinator.isBusy
        || organizationPendingRef.current !== undefined
        || imageImportPendingRef.current !== undefined,
      move: moveDate,
      isInteractionLocked: () => interactionLockedRef.current,
    });
    return () => registerDateMove(undefined);
  }, [coordinator, flushAll, lockInteraction, moveDate, note.id, pinCoordinator, registerDateMove]);

  function changeTitle(nextTitle: string) {
    if (interactionLockedRef.current) return;
    setTitle(nextTitle);
    draftRef.current = { ...draftRef.current, title: nextTitle };
    coordinator.changed(draftRef.current);
  }

  function pasteRichText(event: ReactClipboardEvent<HTMLDivElement>) {
    const imageItem = Array.from(event.clipboardData.items ?? []).find(
      (item) => item.kind === "file" && item.type.startsWith("image/"),
    );
    if (imageItem) {
      event.preventDefault();
      event.stopPropagation();
      if (interactionLockedRef.current || !editor) return;
      const file = imageItem.getAsFile();
      if (!file) {
        setEditorFeedback("无法读取粘贴的图片。");
        return;
      }
      const unlock = lockInteraction();
      setEditorFeedback("正在加密粘贴的图片…");
      const pending = importPastedImage(note.id, file)
        .then((asset) => {
          editor.chain().focus().insertContent({
            type: "imageRef",
            attrs: { asset_id: asset.assetId },
          }).run();
          setEditorFeedback(undefined);
          return true;
        })
        .catch((error: unknown) => {
          setEditorFeedback(
            isFoundationError(error) && error.code === "IMAGE_TOO_LARGE"
              ? "粘贴的图片过大。"
              : "粘贴的图片已被拦截或无法加密。",
          );
          return false;
        })
        .finally(() => {
          if (imageImportPendingRef.current === pending) {
            imageImportPendingRef.current = undefined;
          }
          unlock();
        });
      imageImportPendingRef.current = pending;
      return;
    }
    const html = event.clipboardData.getData("text/html");
    if (!html || !editor) return;
    event.preventDefault();
    event.stopPropagation();
    if (interactionLockedRef.current) return;
    const result = sanitizePastedHtmlForPersistence(html, validateRichTextDocument);
    if (!result.accepted) {
      setEditorFeedback("已拦截不安全的粘贴内容。");
      return;
    }
    setEditorFeedback(
      result.disposition === "downgraded"
        ? "已移除不受支持的粘贴样式，并保留文字内容。"
        : undefined,
    );
    editor.chain().focus().insertContent(result.document.content).run();
  }

  function blockEmbeddedLinkNavigation(event: ReactMouseEvent<HTMLDivElement>) {
    const target = event.target instanceof Element ? event.target.closest("a[href]") : null;
    if (target) event.preventDefault();
  }

  return (
    <article className="note-editor">
      <div className="editor-topline">
        <button
          className={displayedPinned ? "pin-note pinned" : "pin-note"}
          type="button"
          aria-label={displayedPinned ? "取消置顶笔记" : "置顶笔记"}
          aria-busy={pinBusy}
          disabled={dateChanging || organizationBusy || (interactionLocked && !pinBusy)}
          onClick={() => {
            const nextPinned = !pinCoordinator.desiredPinned;
            setDisplayedPinned(nextPinned);
            setOrganizationError(undefined);
            void pinCoordinator.request(nextPinned).then(() => {
              setDisplayedPinned(pinCoordinator.desiredPinned);
            });
          }}
        >
          <span aria-hidden="true">⌖</span>
          {displayedPinned ? "已置顶" : "置顶"}
        </button>
        <span className={`save-state state-${saveState.toLowerCase()}`}>{saveStateLabel(saveState)}</span>
        <button
          className="save-now"
          type="button"
          disabled={dateChanging || interactionLocked}
          onClick={() => void coordinator.flush()}
        >
          保存
        </button>
        {deleteConfirmation ? (
          <div className="delete-note-confirmation" role="group" aria-label="确认删除笔记">
            <span>删除这篇笔记？</span>
            <button
              className="confirm-delete-note"
              type="button"
              aria-label="确认删除笔记"
              disabled={deleting || dateChanging || organizationBusy}
              onClick={() => void confirmNoteDeletion()}
            >
              {deleting ? "正在删除…" : "确认删除"}
            </button>
            <button
              className="cancel-delete-note"
              type="button"
              aria-label="取消删除笔记"
              disabled={deleting}
              onClick={() => {
                setDeleteConfirmation(false);
                setDeleteError(undefined);
              }}
            >
              取消
            </button>
          </div>
        ) : (
          <button
            className="delete-note"
            type="button"
            aria-label="删除笔记"
            disabled={dateChanging || organizationBusy || interactionLocked}
            onClick={() => {
              setDeleteConfirmation(true);
              setDeleteError(undefined);
            }}
          >
            删除
          </button>
        )}
      </div>
      <input
        className="title-input"
        aria-label="笔记标题"
        autoComplete="off"
        maxLength={500}
        placeholder="无标题"
        value={title}
        disabled={dateChanging || interactionLocked}
        onChange={(event) => changeTitle(event.target.value)}
      />
      <div className="date-binding">
        <label>
          <span>笔记日期</span>
          <input
            type="date"
            aria-label="笔记日期"
            autoComplete="off"
            value={noteDate}
            disabled={dateChanging || interactionLocked}
            onChange={(event) => void moveDate(event.target.value)}
          />
        </label>
        <span>ID {note.id}</span>
        <span>修订版本 {revision}</span>
      </div>
      <TagControls
        tags={tags}
        assignedIds={assignedTagIds}
        disabled={dateChanging || pinBusy || organizationBusy || interactionLocked}
        onToggle={(tag, assigned) => trackOrganization(() => changeTagAssignment(tag, assigned))}
        onCreate={(name) => trackOrganization(() => createCatalogTag(name))}
        onRename={(tag, name) => trackOrganization(() => renameCatalogTag(tag, name))}
        onDelete={(tag) => trackOrganization(() => deleteCatalogTag(tag))}
      />
      {organizationError && <p className="note-error organization-error" role="alert">{organizationError}</p>}
      {dateError && <p className="note-error" role="alert">{dateError}</p>}
      {deleteError && <p className="note-error" role="alert">{deleteError}</p>}
      {editor && (
        <RichTextToolbar
          editor={editor}
          disabled={dateChanging || interactionLocked}
          onFeedback={setEditorFeedback}
        />
      )}
      {editorFeedback && <p className="editor-feedback" role="alert">{editorFeedback}</p>}
      <div onPasteCapture={pasteRichText} onClickCapture={blockEmbeddedLinkNavigation}>
        <EditorContent editor={editor} />
      </div>
      <p className="editor-hint">停止输入 750 毫秒后自动保存 · Ctrl+S 立即保存 · Ctrl+Z 撤销本次编辑</p>
    </article>
  );
}

function FoundationFooter({ status }: { status: FoundationStatus }) {
  const backup = status.backupStatus;
  const backupLabel = backup?.state === "healthy"
    ? `备份 ${backup.lastSuccessLocalDay ?? "已验证"} · ${backup.validGenerationCount} 个有效版本`
    : backup?.state === "failed"
      ? "备份需要处理 · 仍可继续编辑"
      : "等待首次成功备份";
  return (
    <footer className="foundation-status">
      <span className="shield" aria-hidden="true">✓</span>
      <span>
        <strong>已在此设备上加密</strong>
        <small>
          安全存储已{status.databaseState === "fresh" ? "创建" : "重新打开"}
          {" · "}架构版本 {status.schemaVersion}
        </small>
        <small className={`backup-health backup-health-${backup?.state ?? "never"}`}>
          {backupLabel}
        </small>
      </span>
    </footer>
  );
}

function summaryFrom(note: Note): NoteSummary {
  return {
    protocolVersion: note.protocolVersion,
    id: note.id,
    noteDate: note.noteDate,
    title: note.title,
    isPinned: note.isPinned,
    updatedAtMs: note.updatedAtMs,
    revision: note.revision,
  };
}

function highlightText(text: string, query: string) {
  const needle = query.trim();
  if (!needle) return text;
  const escaped = needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const parts = text.split(new RegExp(`(${escaped})`, "giu"));
  return parts.map((part, index) => (
    index % 2 === 1
      ? <mark key={index}>{part}</mark>
      : <span key={index}>{part}</span>
  ));
}

function sortVisibleNotes(notes: NoteSummary[], viewMode: "calendar" | "recent"): NoteSummary[] {
  return [...notes].sort((left, right) => {
    if (viewMode === "calendar" && left.isPinned !== right.isPinned) {
      return left.isPinned ? -1 : 1;
    }
    // Modern JavaScript sort is stable, so exact timestamp ties retain the
    // repository's deterministic row order instead of inventing a second,
    // frontend-only public-ID ordering rule.
    return right.updatedAtMs - left.updatedAtMs;
  });
}

function sortTags(tags: Tag[]): Tag[] {
  return [...tags].sort((left, right) => left.name.localeCompare(right.name));
}

function visualStateLabel(state: VisualState): string {
  if (state === "collapsed") return "收起";
  if (state === "expanded") return "展开";
  return "标准";
}

function saveStateLabel(state: SaveState): string {
  if (state === "Dirty") return "未保存";
  if (state === "Saving") return "正在保存";
  if (state === "Error") return "保存失败";
  return "已保存";
}

function localizedErrorMessage(error: FoundationError): string {
  const messages: Record<FoundationError["code"], string> = {
    KEY_UNAVAILABLE: "本地加密密钥不可用。现有数据未被替换。",
    DATABASE_WRONG_KEY: "数据库密钥不匹配。现有数据未被替换。",
    DATABASE_CORRUPTED: "加密数据库可能已经损坏。",
    DATABASE_SCHEMA_TOO_NEW: "此加密数据库由更新版本的桌面笔记创建。",
    MIGRATION_FAILED: "无法安全升级加密数据库。",
    DATA_ROOT_UNAVAILABLE: "本地应用数据目录不可用。",
    DISK_FULL: "磁盘空间已满。",
    WEBVIEW_UNAVAILABLE: "桌面笔记需要 Microsoft Edge WebView2 Runtime。请安装或修复后重启应用。",
    VALIDATION_FAILED: "笔记请求的格式不受支持。",
    NOTE_NOT_FOUND: "找不到请求的笔记。",
    REVISION_CONFLICT: "保存期间笔记已发生变化，请重新加载最新版本。",
    UNDO_EXPIRED: "已无法安全撤销这次日期更改。",
    TAG_NOT_FOUND: "找不到请求的标签。",
    TAG_NAME_CONFLICT: "已有规范化名称相同的标签。",
    IMAGE_REJECTED: "仅支持 PNG、JPEG 或 WebP 图片。",
    IMAGE_TOO_LARGE: "图片超过安全大小限制。",
    ASSET_NOT_FOUND: "这条笔记无法使用该加密图片。",
    ASSET_CORRUPTED: "加密图片完整性校验失败。",
    SHORTCUT_INVALID: "请使用至少两个修饰键，以及一个受支持的字母、数字或功能键。",
    SHORTCUT_CONFLICT: "该快捷键不可用，之前的快捷键仍然有效。",
    CAPTURE_SESSION_MISMATCH: "快速笔记会话已失效。",
    BACKUP_BUSY: "自动备份正在运行。",
    BACKUP_FAILED: "自动备份未能完成，现有笔记和备份均未被替换。",
    BACKUP_CORRUPTED: "备份包完整性校验失败。",
    BACKUP_UNSUPPORTED: "不支持此备份包版本或保护配置。",
    INTERNAL_ERROR: "桌面笔记未能完成这项操作。",
  };
  return messages[error.code];
}

function tagMutationMessage(error: unknown): string {
  return isFoundationError(error) && error.code === "TAG_NAME_CONFLICT"
    ? "同名标签已存在。"
    : "无法保存标签更改。";
}

function dateAtNoon(date: string): Date {
  const [year, month, day] = date.split("-").map(Number);
  const value = new Date(0);
  value.setHours(12, 0, 0, 0);
  value.setFullYear(year, month - 1, day);
  return value;
}

function formatShortDate(date: string): string {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "long",
    day: "numeric",
    year: "numeric",
  }).format(dateAtNoon(date));
}

function ErrorState({ error }: { error: FoundationError }) {
  return (
    <section className="day-panel error-panel" role="alert">
      <p className="eyebrow">安全启动已停止</p>
      <h1>{ERROR_TITLES[error.code]}</h1>
      <p className="error-copy">{localizedErrorMessage(error)}</p>
      <p className="error-code">{error.code}</p>
    </section>
  );
}
