import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

describe("B02 Today and basic Note editor regression under B03", () => {
  beforeEach(() => invokeMock.mockReset());
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("shows Today with the B02 create control and no later-batch controls", async () => {
    invokeMock.mockResolvedValueOnce({
      ok: true,
      status: {
        protocolVersion: 1,
        ready: true,
        view: "today_normal_empty",
        databaseState: "fresh",
        schemaVersion: 2,
        encryption: "sqlcipher",
      },
    });
    invokeMock.mockResolvedValueOnce({ ok: true, notes: [] });
    invokeMock.mockResolvedValueOnce({ ok: true, counts: [] });

    render(<App />);

    expect(await screen.findByRole("heading", { name: "今天" })).toBeVisible();
    expect(screen.getByText("标准")).toBeVisible();
    expect(screen.getByRole("button", { name: "新建笔记" })).toBeVisible();
    expect(await screen.findByText("这一天还没有笔记。")).toBeVisible();
    expect(screen.getByText("已在此设备上加密")).toBeVisible();
    expect(screen.getByRole("complementary", { name: "日历" })).toBeVisible();
    expect(screen.queryByText("标签")).not.toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("get_foundation_status", {
      request: { protocolVersion: 1 },
    });
  });

  it("shows a classified recoverable startup error without exposing internals", async () => {
    invokeMock.mockResolvedValue({
      ok: false,
      error: {
        code: "KEY_UNAVAILABLE",
        message: "The local encryption key is unavailable. Existing data was not replaced.",
        recoverable: true,
      },
    });

    render(<App />);

    expect(await screen.findByText("本地密钥不可用")).toBeVisible();
    expect(screen.getByText(/现有数据未被替换/)).toBeVisible();
    expect(document.body.textContent).not.toContain("keyring.json");
  });

  it("maps Ctrl+S to an immediate typed save for the selected Note", async () => {
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("ec0ec3f0-c0ca-4f64-8ab4-350512a41b71");
    const note = {
      protocolVersion: 1,
      id: "88f220fa-7083-4e16-8c7a-0980a3c92c54",
      noteDate: "2026-09-06",
      title: "B02 Test Note",
      bodyJson: { type: "doc", content: [{ type: "paragraph" }] },
      bodyFormat: "tiptap-json",
      bodySchemaVersion: 1,
      bodyText: "",
      contentHash: "0".repeat(64),
      isPinned: false,
      createdAtMs: 1_780_000_000_000,
      updatedAtMs: 1_780_000_000_000,
      revision: 1,
    };
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "get_foundation_status") return {
        ok: true,
        status: {
          protocolVersion: 1,
          ready: true,
          view: "today_normal_empty",
          databaseState: "reopened",
          schemaVersion: 2,
          encryption: "sqlcipher",
        },
      };
      if (command === "list_notes_for_date") return { ok: true, notes: [note] };
      if (command === "list_note_counts_for_month") {
        return { ok: true, counts: [{ noteDate: note.noteDate, count: 1 }] };
      }
      if (command === "get_note") return { ok: true, note };
      if (command === "list_tags" || command === "list_tags_for_note") {
        return { ok: true, tags: [] };
      }
      if (command === "update_note_content") {
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
        ok: true,
        note: { ...note, title: "Immediate", revision: 2 },
          clientChangeId: request.clientChangeId,
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /B02 Test Note/ }));
    const title = await screen.findByRole("textbox", { name: "笔记标题" });
    fireEvent.change(title, { target: { value: "Immediate" } });
    expect(screen.getByText("未保存")).toBeVisible();

    fireEvent.keyDown(window, { key: "s", ctrlKey: true });

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "update_note_content",
      expect.objectContaining({
        request: expect.objectContaining({
          noteId: note.id,
          baseRevision: 1,
          title: "Immediate",
        }),
      }),
    ));
    expect(await screen.findByText("已保存", {}, { timeout: 3_000 })).toBeVisible();
  });
});
