import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App, clampCalendarPaneWidth } from "./App";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);
function firePointer(
  element: Element,
  type: "pointerdown" | "pointermove" | "pointerup",
  values: { button?: number; pointerId: number; clientX: number },
) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperties(event, {
    button: { value: values.button ?? 0 },
    pointerId: { value: values.pointerId },
    clientX: { value: values.clientX },
  });
  fireEvent(element, event);
}

const note = {
  protocolVersion: 1,
  id: "d1213338-686e-46a9-aa1e-79f3db61d15c",
  noteDate: "2026-09-07",
  title: "待删除笔记",
  bodyJson: { type: "doc", content: [{ type: "paragraph" }] },
  bodyFormat: "tiptap-json",
  bodySchemaVersion: 1,
  bodyText: "",
  contentHash: "0".repeat(64),
  isPinned: false,
  createdAtMs: 1_789_000_000_000,
  updatedAtMs: 1_789_000_000_000,
  revision: 1,
};

describe("note deletion and adjustable calendar divider", () => {
  beforeEach(() => invokeMock.mockReset());

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("requires confirmation, soft-deletes the selected Note, and refreshes active views", async () => {
    let deleted = false;
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("9bd32b76-87dc-4203-b2c5-6fe8701c89a7");
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "get_foundation_status") return {
        ok: true,
        status: {
          protocolVersion: 1,
          ready: true,
          view: "today_normal_empty",
          databaseState: "reopened",
          schemaVersion: 7,
          encryption: "sqlcipher",
        },
      };
      if (command === "list_notes_for_date") return { ok: true, notes: deleted ? [] : [note] };
      if (command === "list_note_counts_for_month") return {
        ok: true,
        counts: deleted ? [] : [{ noteDate: note.noteDate, count: 1 }],
      };
      if (command === "get_note") return { ok: true, note };
      if (command === "list_tags" || command === "list_tags_for_note") return { ok: true, tags: [] };
      if (command === "delete_note") {
        deleted = true;
        const request = (args as { request: { clientChangeId: string } }).request;
        return {
          ok: true,
          deletedNoteId: note.id,
          clientChangeId: request.clientChangeId,
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /待删除笔记/ }));
    fireEvent.click(await screen.findByRole("button", { name: "删除笔记" }));

    expect(screen.getByRole("button", { name: "确认删除笔记" })).toBeVisible();
    expect(invokeMock.mock.calls.some(([command]) => command === "delete_note")).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "取消删除笔记" }));
    expect(screen.queryByRole("button", { name: "确认删除笔记" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "删除笔记" }));
    fireEvent.click(screen.getByRole("button", { name: "确认删除笔记" }));

    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("delete_note", {
      request: {
        protocolVersion: 1,
        noteId: note.id,
        baseRevision: 1,
        clientChangeId: "9bd32b76-87dc-4203-b2c5-6fe8701c89a7",
      },
    }));
    expect(await screen.findByText("笔记已删除。")).toBeVisible();
    expect(screen.queryByRole("textbox", { name: "笔记标题" })).not.toBeInTheDocument();
  });

  it("moves the calendar divider with pointer and keyboard controls", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "get_foundation_status") return {
        ok: true,
        status: {
          protocolVersion: 1,
          ready: true,
          view: "today_normal_empty",
          databaseState: "reopened",
          schemaVersion: 7,
          encryption: "sqlcipher",
        },
      };
      if (command === "list_notes_for_date") return { ok: true, notes: [] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      throw new Error(`Unexpected command ${command}`);
    });

    render(<App />);
    const divider = await screen.findByRole("separator", { name: "调整日历和笔记列表宽度" });
    const workspace = document.querySelector<HTMLElement>(".workspace-content");
    expect(workspace).not.toBeNull();
    Object.defineProperty(workspace, "getBoundingClientRect", {
      value: () => ({ width: 1180 }),
    });

    expect(workspace?.style.getPropertyValue("--calendar-pane-width")).toBe("272px");
    firePointer(divider, "pointerdown", { pointerId: 4, clientX: 272 });
    firePointer(divider, "pointermove", { pointerId: 4, clientX: 400 });
    firePointer(divider, "pointerup", { pointerId: 4, clientX: 400 });
    expect(workspace?.style.getPropertyValue("--calendar-pane-width")).toBe("400px");

    fireEvent.keyDown(divider, { key: "ArrowLeft" });
    expect(workspace?.style.getPropertyValue("--calendar-pane-width")).toBe("384px");
    fireEvent.keyDown(divider, { key: "End" });
    expect(workspace?.style.getPropertyValue("--calendar-pane-width")).toBe("520px");
    fireEvent.doubleClick(divider);
    expect(workspace?.style.getPropertyValue("--calendar-pane-width")).toBe("272px");
  });

  it("reserves enough space for the notes and editor panes in every responsive mode", () => {
    expect(clampCalendarPaneWidth(520, 1000, "expanded", 1070)).toBe(363);
    expect(clampCalendarPaneWidth(520, 1000, "normal", 1070)).toBe(520);
    expect(clampCalendarPaneWidth(520, 1000, "expanded", 1040)).toBe(520);
    expect(clampCalendarPaneWidth(520, 680, "normal", 720)).toBe(391);
    expect(clampCalendarPaneWidth(520, 680, "normal", 700)).toBe(520);
  });
});
