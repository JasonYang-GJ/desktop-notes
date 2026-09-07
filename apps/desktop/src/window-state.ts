import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";

export type VisualState = "collapsed" | "normal" | "expanded";
export type ThemeMode = "system" | "light" | "dark";
export type MaterialKind = "mica" | "desktop_acrylic" | "solid";

export interface WindowSession {
  protocolVersion: 1;
  visualState: VisualState;
  theme: ThemeMode;
  material: MaterialKind;
  materialReason: string;
  topmost: false;
}

type WindowEnvelope =
  | { ok: true; session: WindowSession }
  | { ok: false; error: { code: string; message: string; recoverable: boolean } };

const DEFAULT_SESSION: WindowSession = {
  protocolVersion: 1,
  visualState: "normal",
  theme: "system",
  material: "solid",
  materialReason: "web_preview",
  topmost: false,
};

const request = { protocolVersion: 1 } as const;

async function invokeWindow(command: string, payload: object = request): Promise<WindowSession> {
  const response = await invoke<WindowEnvelope>(command, { request: payload });
  if (!response.ok) throw new Error(response.error.message);
  return response.session;
}

export function useWindowShell() {
  const [session, setSession] = useState(DEFAULT_SESSION);
  const [error, setError] = useState<string>();
  const saveTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return undefined;
    let active = true;
    const removers: Array<() => void> = [];
    const scheduleSave = () => {
      if (saveTimer.current !== undefined) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        void invokeWindow("save_window_placement").catch(() => undefined);
      }, 240);
    };
    void invokeWindow("get_window_session")
      .then((value) => active && setSession(value))
      .catch(() => active && setError("无法加载窗口偏好设置。"));
    const currentWindow = getCurrentWindow();
    for (const subscribe of [currentWindow.onMoved.bind(currentWindow), currentWindow.onResized.bind(currentWindow), currentWindow.onScaleChanged.bind(currentWindow)]) {
      void subscribe(scheduleSave).then((remove) => {
        if (active) removers.push(remove);
        else remove();
      }).catch(() => undefined);
    }
    return () => {
      active = false;
      if (saveTimer.current !== undefined) clearTimeout(saveTimer.current);
      removers.forEach((remove) => remove());
    };
  }, []);

  const setVisualState = useCallback(async (visualState: VisualState) => {
    setError(undefined);
    setSession((current) => ({ ...current, visualState }));
    if (!("__TAURI_INTERNALS__" in window)) return;
    try {
      setSession(await invokeWindow("set_visual_state", { protocolVersion: 1, visualState }));
    } catch {
      setError("无法安全更改窗口大小。");
    }
  }, []);

  const setTheme = useCallback(async (theme: ThemeMode) => {
    setError(undefined);
    setSession((current) => ({ ...current, theme }));
    if (!("__TAURI_INTERNALS__" in window)) return;
    try {
      setSession(await invokeWindow("set_theme_mode", { protocolVersion: 1, theme }));
    } catch {
      setError("无法保存主题偏好设置。");
    }
  }, []);

  return { session, error, setVisualState, setTheme };
}
