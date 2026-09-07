import { describe, expect, it, vi } from "vitest";

import {
  installWindowSaveLifecycle,
  type DesktopWindowLike,
  type LifecycleEditor,
} from "./window-save-lifecycle";

function fakeWindow() {
  let close: ((event: { preventDefault: () => void }) => void | Promise<void>) | undefined;
  let focus: ((event: { payload: boolean }) => void | Promise<void>) | undefined;
  let resize: (() => void | Promise<void>) | undefined;
  const desktop: DesktopWindowLike = {
    onCloseRequested: vi.fn(async (handler) => { close = handler; return () => undefined; }),
    onFocusChanged: vi.fn(async (handler) => { focus = handler; return () => undefined; }),
    onResized: vi.fn(async (handler) => { resize = handler; return () => undefined; }),
    isMinimized: vi.fn().mockResolvedValue(true),
    destroy: vi.fn().mockResolvedValue(undefined),
  };
  return {
    desktop,
    close: (event: { preventDefault: () => void }) => close?.(event),
    focus: (focused: boolean) => focus?.({ payload: focused }),
    resize: () => resize?.(),
  };
}

function editor(flushResult: boolean): LifecycleEditor {
  return {
    hasUnsavedChanges: () => true,
    lockInteraction: vi.fn(() => vi.fn()),
    flush: vi.fn().mockResolvedValue(flushResult),
  };
}

describe("B04 window save lifecycle", () => {
  it("prevents close until a dirty Note flushes, then destroys the real window", async () => {
    const target = fakeWindow();
    const current = editor(true);
    const onFailure = vi.fn();
    const lifecycle = installWindowSaveLifecycle({
      desktopWindow: target.desktop,
      getEditor: () => current,
      onFailure,
    });
    await lifecycle.ready;
    const preventDefault = vi.fn();
    await target.close({ preventDefault });
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(current.flush).toHaveBeenCalledOnce();
    expect(target.desktop.destroy).toHaveBeenCalledOnce();
    expect(onFailure).not.toHaveBeenCalled();
    lifecycle.dispose();
  });

  it("blocks close and keeps the editor available when persistence fails", async () => {
    const target = fakeWindow();
    const current = editor(false);
    const onFailure = vi.fn();
    const lifecycle = installWindowSaveLifecycle({
      desktopWindow: target.desktop,
      getEditor: () => current,
      onFailure,
    });
    await lifecycle.ready;
    const preventDefault = vi.fn();
    await target.close({ preventDefault });
    expect(preventDefault).toHaveBeenCalledOnce();
    expect(target.desktop.destroy).not.toHaveBeenCalled();
    expect(onFailure).toHaveBeenCalledWith("无法保存当前笔记，窗口已保持打开。请重试。");
    lifecycle.dispose();
  });

  it("attempts a flush on focus loss and minimize state changes", async () => {
    const target = fakeWindow();
    const current = editor(true);
    const lifecycle = installWindowSaveLifecycle({
      desktopWindow: target.desktop,
      getEditor: () => current,
      onFailure: () => undefined,
    });
    await lifecycle.ready;
    await target.focus(false);
    await target.resize();
    await vi.waitFor(() => expect(current.flush).toHaveBeenCalledTimes(2));
    lifecycle.dispose();
  });
});
