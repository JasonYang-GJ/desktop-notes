export interface LifecycleEditor {
  hasUnsavedChanges: () => boolean;
  lockInteraction: () => () => void;
  flush: () => Promise<boolean>;
}

type Unlisten = () => void;

export interface DesktopWindowLike {
  onCloseRequested: (
    handler: (event: { preventDefault: () => void }) => void | Promise<void>,
  ) => Promise<Unlisten>;
  onFocusChanged: (
    handler: (event: { payload: boolean }) => void | Promise<void>,
  ) => Promise<Unlisten>;
  onResized: (
    handler: (event?: { payload: unknown }) => void | Promise<void>,
  ) => Promise<Unlisten>;
  isMinimized: () => Promise<boolean>;
  destroy: () => Promise<void>;
}

export interface WindowSaveLifecycle {
  ready: Promise<void>;
  dispose: () => void;
}

export function installWindowSaveLifecycle({
  desktopWindow,
  getEditor,
  onFailure,
  browserWindow = window,
  browserDocument = document,
}: {
  desktopWindow?: DesktopWindowLike;
  getEditor: () => LifecycleEditor | undefined;
  onFailure: (message: string) => void;
  browserWindow?: Window;
  browserDocument?: Document;
}): WindowSaveLifecycle {
  let disposed = false;
  let allowUnload = false;
  let closing = false;
  let regularFlush: Promise<boolean> | undefined;
  const unlisteners: Unlisten[] = [];

  const flushForStateChange = (): Promise<boolean> => {
    if (regularFlush) return regularFlush;
    const current = getEditor();
    if (!current?.hasUnsavedChanges()) return Promise.resolve(true);
    const unlock = current.lockInteraction();
    regularFlush = current.flush()
      .then((saved) => {
        if (!saved) onFailure("无法保存当前笔记，编辑内容仍保持打开。请重试。");
        return saved;
      })
      .catch(() => {
        onFailure("无法保存当前笔记，编辑内容仍保持打开。请重试。");
        return false;
      })
      .finally(() => {
        unlock();
        regularFlush = undefined;
      });
    return regularFlush;
  };

  const beforeUnload = (event: BeforeUnloadEvent) => {
    if (allowUnload || !getEditor()?.hasUnsavedChanges()) return;
    event.preventDefault();
    event.returnValue = "";
  };
  const visibilityChanged = () => {
    if (browserDocument.visibilityState === "hidden") void flushForStateChange();
  };
  browserWindow.addEventListener("beforeunload", beforeUnload);
  browserDocument.addEventListener("visibilitychange", visibilityChanged);

  const ready = desktopWindow
    ? Promise.all([
        desktopWindow.onCloseRequested(async (event) => {
          event.preventDefault();
          if (closing) return;
          closing = true;
          const current = getEditor();
          const unlock = current?.lockInteraction();
          try {
            const saved = !current?.hasUnsavedChanges() || await current.flush();
            if (!saved) {
              onFailure("无法保存当前笔记，窗口已保持打开。请重试。");
              closing = false;
              unlock?.();
              return;
            }
            allowUnload = true;
            await desktopWindow.destroy();
          } catch {
            allowUnload = false;
            closing = false;
            unlock?.();
            onFailure("无法保存当前笔记，窗口已保持打开。请重试。");
          }
        }),
        desktopWindow.onFocusChanged(async ({ payload: focused }) => {
          if (!focused) await flushForStateChange();
        }),
        desktopWindow.onResized(async () => {
          if (await desktopWindow.isMinimized()) await flushForStateChange();
        }),
      ])
        .then((registered) => {
          if (disposed) registered.forEach((unlisten) => unlisten());
          else unlisteners.push(...registered);
        })
        .catch(() => {
          onFailure("无法安装窗口保存保护。存在未保存编辑时，窗口将阻止退出。");
        })
    : Promise.resolve();

  return {
    ready,
    dispose: () => {
      disposed = true;
      browserWindow.removeEventListener("beforeunload", beforeUnload);
      browserDocument.removeEventListener("visibilitychange", visibilityChanged);
      unlisteners.splice(0).forEach((unlisten) => unlisten());
    },
  };
}
