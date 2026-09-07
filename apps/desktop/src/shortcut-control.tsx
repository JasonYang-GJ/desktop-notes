import { useState } from "react";

import { isFoundationError } from "./foundation";
import {
  getShortcutConfig,
  setShortcutConfig,
  type ShortcutStatus,
} from "./quick-capture";

export function ShortcutControl() {
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<ShortcutStatus>();
  const [shortcut, setShortcut] = useState("Ctrl+Alt+N");
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string>();

  async function toggle() {
    if (open) {
      setOpen(false);
      return;
    }
    setOpen(true);
    setWorking(true);
    setError(undefined);
    try {
      const current = await getShortcutConfig();
      setStatus(current);
      setShortcut(current.configuredShortcut);
    } catch {
      setError("无法加载快捷键设置。");
    } finally {
      setWorking(false);
    }
  }

  async function save() {
    setWorking(true);
    setError(undefined);
    try {
      const updated = await setShortcutConfig(shortcut);
      setStatus(updated);
      setShortcut(updated.configuredShortcut);
    } catch (caught: unknown) {
      setError(
        isFoundationError(caught) && caught.code === "SHORTCUT_CONFLICT"
          ? "该快捷键已被其他应用占用，之前的快捷键仍然有效。"
          : isFoundationError(caught) && caught.code === "SHORTCUT_INVALID"
            ? "快捷键无效。请使用至少两个修饰键，以及一个受支持的字母、数字或功能键。"
            : "无法更改快捷键，之前的快捷键仍然有效。",
      );
    } finally {
      setWorking(false);
    }
  }

  return (
    <div className="shortcut-control">
      <button
        type="button"
        className="shortcut-trigger"
        aria-expanded={open}
        aria-controls="shortcut-settings"
        onClick={() => void toggle()}
      >
        快速笔记
      </button>
      {open && (
        <section id="shortcut-settings" className="shortcut-popover" aria-label="快速笔记快捷键设置">
          <label>
            <span>全局快捷键</span>
            <input
              aria-label="全局快捷键"
              value={shortcut}
              disabled={working}
              onChange={(event) => setShortcut(event.target.value)}
              placeholder="Ctrl+Alt+N"
            />
          </label>
          <button type="button" disabled={working} onClick={() => void save()}>
            {working ? "正在检查…" : "应用"}
          </button>
          {status && (
            <small role="status">
              当前生效：{status.activeShortcut ?? "不可用"}
              {status.registrationError && ` · 错误代码 ${status.registrationError}`}
            </small>
          )}
          {error && <p role="alert">{error}</p>}
        </section>
      )}
    </div>
  );
}
