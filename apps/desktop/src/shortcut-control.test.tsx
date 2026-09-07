import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ShortcutControl } from "./shortcut-control";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

describe("B08 minimum shortcut configuration entry", () => {
  beforeEach(() => invokeMock.mockReset());
  afterEach(cleanup);

  it("loads and atomically applies a configurable shortcut", async () => {
    invokeMock
      .mockResolvedValueOnce({ ok: true, status: status("Ctrl+Alt+N") })
      .mockResolvedValueOnce({ ok: true, status: status("Ctrl+Alt+Q") });
    render(<ShortcutControl />);
    fireEvent.click(screen.getByRole("button", { name: "快速笔记" }));
    const input = await screen.findByLabelText("全局快捷键");
    expect(input).toHaveValue("Ctrl+Alt+N");
    fireEvent.change(input, { target: { value: "Ctrl+Alt+Q" } });
    fireEvent.click(screen.getByRole("button", { name: "应用" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("当前生效：Ctrl+Alt+Q"));
    expect(invokeMock).toHaveBeenLastCalledWith("set_shortcut_config", {
      request: { protocolVersion: 1, shortcut: "Ctrl+Alt+Q" },
    });
  });

  it("shows a conflict and keeps the previous active shortcut visible", async () => {
    invokeMock
      .mockResolvedValueOnce({ ok: true, status: status("Ctrl+Alt+N") })
      .mockResolvedValueOnce({
        ok: false,
        error: {
          code: "SHORTCUT_CONFLICT",
          message: "That shortcut is unavailable.",
          recoverable: true,
        },
      });
    render(<ShortcutControl />);
    fireEvent.click(screen.getByRole("button", { name: "快速笔记" }));
    const input = await screen.findByLabelText("全局快捷键");
    fireEvent.change(input, { target: { value: "Ctrl+Alt+P" } });
    fireEvent.click(screen.getByRole("button", { name: "应用" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("之前的快捷键仍然有效");
    expect(screen.getByRole("status")).toHaveTextContent("当前生效：Ctrl+Alt+N");
  });
});

function status(shortcut: string) {
  return {
    protocolVersion: 1,
    configuredShortcut: shortcut,
    activeShortcut: shortcut,
    registrationError: null,
  };
}
