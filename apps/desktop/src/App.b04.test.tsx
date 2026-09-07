import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CalendarWorkspace } from "./App";
import type { FoundationStatus } from "./foundation";
import type { Note } from "./notes";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const status: FoundationStatus = {
  protocolVersion: 1,
  ready: true,
  view: "today_normal_empty",
  databaseState: "reopened",
  schemaVersion: 2,
  encryption: "sqlcipher",
};

function note(id: string, title: string): Note {
  return {
    protocolVersion: 1,
    id,
    noteDate: "2026-09-06",
    title,
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
}

describe("B04 rich text and editor lifecycle", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("ec0ec3f0-c0ca-4f64-8ab4-350512a41b71");
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("blocks a Note switch when flushing the current dirty Note fails", async () => {
    const first = note("88f220fa-7083-4e16-8c7a-0980a3c92c54", "First note");
    const second = note("39aa93a1-e91c-4f0d-a622-b48fe89ddb16", "Second note");
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [first, second] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") {
        const noteId = (args as { request: { noteId: string } }).request.noteId;
        return { ok: true, note: noteId === first.id ? first : second };
      }
      if (command === "update_note_content") {
        return {
          ok: false,
          error: { code: "DISK_FULL", message: "Synthetic save failure", recoverable: true },
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /First note/ }));
    const title = await screen.findByRole("textbox", { name: "笔记标题" });
    fireEvent.change(title, { target: { value: "First note unsaved" } });
    fireEvent.click(screen.getByRole("button", { name: /Second note/ }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "update_note_content",
      expect.objectContaining({ request: expect.objectContaining({ title: "First note unsaved" }) }),
    ));
    expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue("First note unsaved");
    expect(screen.getByText("保存失败")).toBeVisible();
    expect(screen.getByText("当前笔记无法保存，因此仍保持打开。请重试。")).toBeVisible();
  });

  it("shows the D08 toolbar and routes HTML paste through the fail-closed sanitizer", async () => {
    const current = note("88f220fa-7083-4e16-8c7a-0980a3c92c54", "Rich note");
    invokeMock.mockImplementation(async (command) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [current] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: current };
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Rich note/ }));
    const body = await screen.findByLabelText("笔记正文");
    expect(screen.getByRole("toolbar", { name: "文字格式" })).toBeVisible();
    expect(screen.getByRole("button", { name: "任务清单" })).toBeVisible();

    fireEvent.paste(body, {
      clipboardData: {
        getData: (type: string) => type === "text/html"
          ? "<h2><strong>Safe pasted heading</strong></h2><p><a href=\"https://example.com\">Safe link</a></p>"
          : "Safe pasted heading Safe link",
      },
    });
    expect(await screen.findByRole("heading", { name: "Safe pasted heading" })).toBeVisible();
    const safeLink = screen.getByRole("link", { name: "Safe link" });
    expect(safeLink).toHaveAttribute("href", "https://example.com");
    expect(fireEvent.click(safeLink)).toBe(false);

    fireEvent.paste(body, {
      clipboardData: {
        getData: (type: string) => type === "text/html"
          ? "<p>官方文档确认，<code>Ling-3.0-flash-VL</code> 可以处理<strong>文本、图片和视频</strong>。</p>"
          : "官方文档确认，Ling-3.0-flash-VL 可以处理文本、图片和视频。",
      },
    });
    expect(body).toHaveTextContent("官方文档确认，Ling-3.0-flash-VL 可以处理文本、图片和视频。");
    expect(screen.getByText("已移除不受支持的粘贴样式，并保留文字内容。")).toBeVisible();

    fireEvent.paste(body, {
      clipboardData: {
        getData: (type: string) => type === "text/html"
          ? "<script>alert('blocked')</script><p>Do not insert</p>"
          : "Do not insert",
      },
    });
    expect(screen.getByText("已拦截不安全的粘贴内容。")).toBeVisible();
    expect(body).not.toHaveTextContent("Do not insert");
  });

  it("flushes A successfully before loading B", async () => {
    const first = note("88f220fa-7083-4e16-8c7a-0980a3c92c54", "First note");
    const second = note("39aa93a1-e91c-4f0d-a622-b48fe89ddb16", "Second note");
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [first, second] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") {
        const noteId = (args as { request: { noteId: string } }).request.noteId;
        return { ok: true, note: noteId === first.id ? first : second };
      }
      if (command === "update_note_content") {
        const request = (args as { request: { clientChangeId: string; title: string } }).request;
        return {
          ok: true,
          note: { ...first, title: request.title, revision: 2 },
          clientChangeId: request.clientChangeId,
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /First note/ }));
    fireEvent.change(await screen.findByRole("textbox", { name: "笔记标题" }), {
      target: { value: "First saved before switch" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Second note/ }));

    await waitFor(() => expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Second note"));
    const saveIndex = invokeMock.mock.calls.findIndex(([command]) => command === "update_note_content");
    const secondOpenIndex = invokeMock.mock.calls.findIndex(([command, args]) => command === "get_note"
      && (args as { request: { noteId: string } }).request.noteId === second.id);
    expect(saveIndex).toBeGreaterThan(-1);
    expect(saveIndex).toBeLessThan(secondOpenIndex);
  });

  it("keeps Ctrl+S failure visible as Error and cancels browser Save Page", async () => {
    const current = note("88f220fa-7083-4e16-8c7a-0980a3c92c54", "Save failure note");
    invokeMock.mockImplementation(async (command) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [current] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: current };
      if (command === "update_note_content") {
        return { ok: false, error: { code: "DISK_FULL", message: "Synthetic", recoverable: true } };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Save failure note/ }));
    const title = await screen.findByRole("textbox", { name: "笔记标题" });
    fireEvent.change(title, { target: { value: "Keep this draft" } });
    const shortcut = new KeyboardEvent("keydown", {
      key: "s",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    expect(window.dispatchEvent(shortcut)).toBe(false);
    expect(await screen.findByText("保存失败")).toBeVisible();
    expect(title).toHaveValue("Keep this draft");
  });

  it("reports a controlled recovery error without rewriting a future rich-text envelope", async () => {
    const future = { ...note("88f220fa-7083-4e16-8c7a-0980a3c92c54", "Future note"), bodySchemaVersion: 2 };
    invokeMock.mockImplementation(async (command) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [future] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: future };
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Future note/ }));
    expect(await screen.findByText(
      "这条笔记含有不受支持或损坏的富文本；已保存的正文未被改动。",
    )).toBeVisible();
    expect(screen.queryByRole("textbox", { name: "笔记标题" })).not.toBeInTheDocument();
  });
});
