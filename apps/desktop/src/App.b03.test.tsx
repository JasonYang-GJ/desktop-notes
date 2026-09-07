import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CalendarWorkspace } from "./App";
import { NOTE_DRAG_MIME } from "./calendar-view";
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

function note(overrides: Partial<Note> = {}): Note {
  return {
    protocolVersion: 1,
    id: "88f220fa-7083-4e16-8c7a-0980a3c92c54",
    noteDate: "2026-09-06",
    title: "Calendar note",
    bodyJson: { type: "doc", content: [{ type: "paragraph" }] },
    bodyFormat: "tiptap-json",
    bodySchemaVersion: 1,
    bodyText: "",
    contentHash: "0".repeat(64),
    createdAtMs: 1_780_000_000_000,
    updatedAtMs: 1_780_000_000_000,
    revision: 1,
    ...overrides,
    isPinned: overrides.isPinned ?? false,
  };
}

describe("B03 calendar and Note date integration", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("ec0ec3f0-c0ca-4f64-8ab4-350512a41b71");
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("starts on Today, selects another date, and creates the Note on that date", async () => {
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [] };
      if (command === "list_note_counts_for_month") {
        return { ok: true, counts: [{ noteDate: "2026-09-10", count: 2 }] };
      }
      if (command === "create_note") {
        const request = (args as { request: { noteDate: string } }).request;
        return { ok: true, note: note({ noteDate: request.noteDate, title: "" }) };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);

    expect(await screen.findByRole("heading", { name: "今天" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: /2026年9月10日.*2 条笔记/i }));
    expect(await screen.findByRole("heading", { name: "9月10日" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "新建笔记" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "create_note",
      expect.objectContaining({ request: expect.objectContaining({ noteDate: "2026-09-10" }) }),
    ));
  });

  it("flushes a pending edit before changing date and offers a revision-safe undo", async () => {
    const original = note();
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") {
        const requested = (args as { request: { noteDate: string } }).request.noteDate;
        return { ok: true, notes: requested === original.noteDate ? [original] : [] };
      }
      if (command === "list_note_counts_for_month") {
        return { ok: true, counts: [{ noteDate: original.noteDate, count: 1 }] };
      }
      if (command === "get_note") return { ok: true, note: original };
      if (command === "update_note_content") {
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
          ok: true,
          note: note({ title: "Saved before move", revision: 2 }),
          clientChangeId: request.clientChangeId,
        };
      }
      if (command === "change_note_date") {
        const request = (args as {
          request: { baseRevision: number; clientChangeId: string; newNoteDate: string };
        }).request;
        expect(request.baseRevision).toBe(2);
        return {
          ok: true,
          note: note({ title: "Saved before move", noteDate: request.newNoteDate, revision: 3 }),
          previousDate: original.noteDate,
          undoToken: { tokenId: "undo-token", expiresAtMs: Date.now() + 10_000 },
          clientChangeId: request.clientChangeId,
        };
      }
      if (command === "undo_note_date_change") {
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
          ok: true,
          note: note({ title: "Saved before move", revision: 4 }),
          clientChangeId: request.clientChangeId,
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Calendar note/ }));
    fireEvent.change(await screen.findByRole("textbox", { name: "笔记标题" }), {
      target: { value: "Saved before move" },
    });
    fireEvent.change(screen.getByLabelText("笔记日期"), { target: { value: "2026-09-10" } });

    expect(await screen.findByText(/已移动到 2026年9月10日/)).toBeVisible();
    expect(invokeMock.mock.calls.findIndex(([command]) => command === "update_note_content"))
      .toBeLessThan(invokeMock.mock.calls.findIndex(([command]) => command === "change_note_date"));
    fireEvent.click(screen.getByRole("button", { name: "撤销日期更改" }));
    await waitFor(() => expect(screen.getByLabelText("笔记日期")).toHaveValue("2026-09-06"));
  });

  it("flushes edits made after a date move before attempting undo", async () => {
    const original = note();
    const moved = note({ noteDate: "2026-09-10", revision: 2 });
    let savedAfterMove = false;
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") {
        const requested = (args as { request: { noteDate: string } }).request.noteDate;
        return { ok: true, notes: requested === original.noteDate ? [original] : [] };
      }
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: original };
      if (command === "change_note_date") {
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
          ok: true,
          note: moved,
          previousDate: original.noteDate,
          undoToken: { tokenId: "undo-after-edit", expiresAtMs: Date.now() + 10_000 },
          clientChangeId: request.clientChangeId,
        };
      }
      if (command === "update_note_content") {
        savedAfterMove = true;
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
          ok: true,
          note: note({ title: "Edit after move", noteDate: moved.noteDate, revision: 3 }),
          clientChangeId: request.clientChangeId,
        };
      }
      if (command === "undo_note_date_change") {
        if (!savedAfterMove) {
          const request = (args as { request: { clientChangeId: string } }).request;
          return {
            ok: true,
            note: note({ revision: 3 }),
            clientChangeId: request.clientChangeId,
          };
        }
        return {
          ok: false,
          error: {
            code: "REVISION_CONFLICT",
            message: "The Note changed before undo.",
            recoverable: true,
          },
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Calendar note/ }));
    fireEvent.change(await screen.findByLabelText("笔记日期"), { target: { value: "2026-09-10" } });
    expect(await screen.findByText(/已移动到 2026年9月10日/)).toBeVisible();
    const title = screen.getByRole("textbox", { name: "笔记标题" });
    fireEvent.change(title, { target: { value: "Edit after move" } });
    fireEvent.click(screen.getByRole("button", { name: "撤销日期更改" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "update_note_content",
      expect.objectContaining({ request: expect.objectContaining({ title: "Edit after move" }) }),
    ));
    expect(invokeMock.mock.calls.findIndex(([command]) => command === "update_note_content"))
      .toBeLessThan(invokeMock.mock.calls.findIndex(([command]) => command === "undo_note_date_change"));
    expect(title).toHaveValue("Edit after move");
    expect(await screen.findByText("笔记内容已经变化，无法再撤销日期更改。")).toBeVisible();
  });

  it("waits for an in-flight post-move save before attempting undo", async () => {
    const original = note();
    const moved = note({ noteDate: "2026-09-10", revision: 2 });
    let resolveSave!: (value: unknown) => void;
    const saveResponse = new Promise((resolve) => { resolveSave = resolve; });
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") {
        const requested = (args as { request: { noteDate: string } }).request.noteDate;
        return { ok: true, notes: requested === original.noteDate ? [original] : [] };
      }
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: original };
      if (command === "change_note_date") {
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
          ok: true,
          note: moved,
          previousDate: original.noteDate,
          undoToken: { tokenId: "undo-during-save", expiresAtMs: Date.now() + 10_000 },
          clientChangeId: request.clientChangeId,
        };
      }
      if (command === "update_note_content") return saveResponse;
      if (command === "undo_note_date_change") {
        return {
          ok: false,
          error: {
            code: "REVISION_CONFLICT",
            message: "The Note changed before undo.",
            recoverable: true,
          },
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Calendar note/ }));
    fireEvent.change(await screen.findByLabelText("笔记日期"), { target: { value: "2026-09-10" } });
    expect(await screen.findByText(/已移动到 2026年9月10日/)).toBeVisible();
    const title = screen.getByRole("textbox", { name: "笔记标题" });
    fireEvent.change(title, { target: { value: "In-flight edit" } });
    fireEvent.keyDown(window, { key: "s", ctrlKey: true });
    expect(await screen.findByText("正在保存")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "撤销日期更改" }));
    expect(invokeMock).not.toHaveBeenCalledWith("undo_note_date_change", expect.anything());

    resolveSave({
      ok: true,
      note: note({ title: "In-flight edit", noteDate: moved.noteDate, revision: 3 }),
      clientChangeId: "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71",
    });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "undo_note_date_change",
      expect.objectContaining({ request: expect.objectContaining({ tokenId: "undo-during-save" }) }),
    ));
    expect(title).toHaveValue("In-flight edit");
    expect(await screen.findByText("笔记内容已经变化，无法再撤销日期更改。")).toBeVisible();
  });

  it("locks the active editor while a date undo is in flight", async () => {
    const original = note();
    const moved = note({ noteDate: "2026-09-10", revision: 2 });
    let resolveUndo!: (value: unknown) => void;
    const undoResponse = new Promise((resolve) => { resolveUndo = resolve; });
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") {
        const requested = (args as { request: { noteDate: string } }).request.noteDate;
        return { ok: true, notes: requested === original.noteDate ? [original] : [] };
      }
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: original };
      if (command === "change_note_date") {
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
          ok: true,
          note: moved,
          previousDate: original.noteDate,
          undoToken: { tokenId: "undo-transition-lock", expiresAtMs: Date.now() + 10_000 },
          clientChangeId: request.clientChangeId,
        };
      }
      if (command === "undo_note_date_change") return undoResponse;
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Calendar note/ }));
    fireEvent.change(await screen.findByLabelText("笔记日期"), { target: { value: "2026-09-10" } });
    expect(await screen.findByText(/已移动到 2026年9月10日/)).toBeVisible();
    const title = screen.getByRole("textbox", { name: "笔记标题" });
    const body = await screen.findByLabelText("笔记正文");
    fireEvent.click(screen.getByRole("button", { name: "撤销日期更改" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "undo_note_date_change",
      expect.objectContaining({ request: expect.objectContaining({ tokenId: "undo-transition-lock" }) }),
    ));

    expect(title).toBeDisabled();
    expect(screen.getByLabelText("笔记日期")).toBeDisabled();
    expect(screen.getByRole("button", { name: "保存" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "新建笔记" })).toBeDisabled();
    expect(body).toHaveAttribute("contenteditable", "false");
    fireEvent.change(title, { target: { value: "Late edit must not enter the draft" } });
    fireEvent.click(screen.getByRole("button", { name: /2026年9月11日/i }));
    expect(title).toHaveValue(original.title);
    expect(screen.getByRole("heading", { name: "9月10日" })).toBeVisible();

    resolveUndo({
      ok: true,
      note: note({ revision: 3 }),
      clientChangeId: "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71",
    });
    await waitFor(() => expect(screen.getByLabelText("笔记日期")).toHaveValue(original.noteDate));
    expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue(original.title);
    expect(screen.getByRole("textbox", { name: "笔记标题" })).toBeEnabled();
  });

  it("dismisses a prior Note's date undo before opening another Note", async () => {
    const original = note();
    const moved = note({ noteDate: "2026-09-10", revision: 2 });
    const other = note({
      id: "39aa93a1-e91c-4f0d-a622-b48fe89ddb16",
      noteDate: moved.noteDate,
      title: "Another note",
    });
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") {
        const requested = (args as { request: { noteDate: string } }).request.noteDate;
        return { ok: true, notes: requested === moved.noteDate ? [moved, other] : [original] };
      }
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") {
        const request = (args as { request: { noteId: string } }).request;
        return { ok: true, note: request.noteId === other.id ? other : original };
      }
      if (command === "change_note_date") {
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
          ok: true,
          note: moved,
          previousDate: original.noteDate,
          undoToken: { tokenId: "undo-owned-by-original", expiresAtMs: Date.now() + 10_000 },
          clientChangeId: request.clientChangeId,
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Calendar note/ }));
    fireEvent.change(await screen.findByLabelText("笔记日期"), { target: { value: moved.noteDate } });
    expect(await screen.findByRole("button", { name: "撤销日期更改" })).toBeVisible();
    fireEvent.click(await screen.findByRole("button", { name: /Another note/ }));

    await waitFor(() => expect(screen.getByRole("textbox", { name: "笔记标题" }))
      .toHaveValue(other.title));
    expect(screen.queryByRole("button", { name: "撤销日期更改" })).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "笔记标题" }), {
      target: { value: "Another note pending edit" },
    });
    expect(screen.getByRole("textbox", { name: "笔记标题" }))
      .toHaveValue("Another note pending edit");
  });

  it("locks the active editor while another Note is opening", async () => {
    const original = note();
    const moved = note({ noteDate: "2026-09-10", revision: 2 });
    const other = note({
      id: "39aa93a1-e91c-4f0d-a622-b48fe89ddb16",
      title: "Late open response",
    });
    let resolveOther!: (value: unknown) => void;
    const otherResponse = new Promise((resolve) => { resolveOther = resolve; });
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") {
        const requested = (args as { request: { noteDate: string } }).request.noteDate;
        return { ok: true, notes: requested === original.noteDate ? [original, other] : [moved] };
      }
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") {
        const request = (args as { request: { noteId: string } }).request;
        return request.noteId === other.id ? otherResponse : { ok: true, note: original };
      }
      if (command === "change_note_date") {
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
          ok: true,
          note: moved,
          previousDate: original.noteDate,
          undoToken: { tokenId: "undo-after-late-open", expiresAtMs: Date.now() + 10_000 },
          clientChangeId: request.clientChangeId,
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Calendar note/ }));
    await screen.findByRole("textbox", { name: "笔记标题" });
    fireEvent.click(screen.getByRole("button", { name: /Late open response/ }));
    expect(screen.getByRole("textbox", { name: "笔记标题" })).toBeDisabled();
    expect(screen.getByLabelText("笔记日期")).toBeDisabled();
    fireEvent.change(screen.getByLabelText("笔记日期"), { target: { value: moved.noteDate } });
    expect(invokeMock).not.toHaveBeenCalledWith("change_note_date", expect.anything());

    resolveOther({ ok: true, note: other });
    await waitFor(() => expect(screen.getByRole("textbox", { name: "笔记标题" }))
      .toHaveValue(other.title));
    expect(screen.getByLabelText("笔记日期")).toHaveValue(other.noteDate);
  });

  it("moves a listed Note when it is dropped on another calendar date", async () => {
    const original = note();
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [original] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "change_note_date") {
        const request = (args as {
          request: { clientChangeId: string; newNoteDate: string };
        }).request;
        return {
          ok: true,
          note: note({ noteDate: request.newNoteDate, revision: 2 }),
          previousDate: original.noteDate,
          undoToken: { tokenId: "drag-undo", expiresAtMs: Date.now() + 10_000 },
          clientChangeId: request.clientChangeId,
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    const row = await screen.findByRole("button", { name: /Calendar note/ });
    const transfer = {
      setData: vi.fn(),
      getData: (type: string) => type === NOTE_DRAG_MIME ? original.id : "",
      effectAllowed: "none",
      dropEffect: "none",
    };
    fireEvent.dragStart(row, { dataTransfer: transfer });
    fireEvent.drop(screen.getByRole("button", { name: /2026年9月14日/i }), {
      dataTransfer: transfer,
    });

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "change_note_date",
      expect.objectContaining({
        request: expect.objectContaining({
          noteId: original.id,
          newNoteDate: "2026-09-14",
          baseRevision: 1,
        }),
      }),
    ));
  });
});
