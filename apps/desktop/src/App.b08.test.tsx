import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CalendarWorkspace } from "./App";
import type { FoundationStatus } from "./foundation";
import type { Note } from "./notes";

const eventHarness = vi.hoisted(() => ({ handler: undefined as (() => void) | undefined }));

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_event: string, handler: () => void) => {
    eventHarness.handler = handler;
    return () => undefined;
  }),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onCloseRequested: async () => () => undefined,
    onFocusChanged: async () => () => undefined,
    onResized: async () => () => undefined,
    isMinimized: async () => false,
    destroy: async () => undefined,
  }),
}));

const invokeMock = vi.mocked(invoke);
const existingId = "84000000-0000-4000-8000-000000000001";
const captureId = "84000000-0000-4000-8000-000000000002";
const status: FoundationStatus = {
  protocolVersion: 1,
  ready: true,
  view: "today_normal_empty",
  databaseState: "reopened",
  schemaVersion: 6,
  encryption: "sqlcipher",
};

describe("B08 workspace capture integration", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    eventHarness.handler = undefined;
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      value: {},
      configurable: true,
    });
  });

  afterEach(() => {
    cleanup();
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  });

  it("coalesces rapid delivery and keeps the already-dirty editor mounted unchanged", async () => {
    let captureTake = 0;
    const existing = note();
    invokeMock.mockImplementation(async (command) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [existing] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: existing };
      if (command === "list_tags" || command === "list_tags_for_note") return { ok: true, tags: [] };
      if (command === "take_quick_capture") {
        captureTake += 1;
        return captureTake === 1 ? { ok: true, session: null } : {
          ok: true,
          session: {
            protocolVersion: 1,
            id: captureId,
            contentType: "text",
            text: "Rapid capture text",
            dataBase64: null,
            mediaType: null,
            acquisitionAttempts: 1,
          },
        };
      }
      if (command === "finish_quick_capture") return { ok: true };
      if (command === "update_note_content") throw new Error("autosave is outside this assertion");
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-07" />);
    fireEvent.click(await screen.findByRole("button", { name: /Existing dirty Note/ }));
    const existingTitle = await screen.findByLabelText("笔记标题");
    fireEvent.change(existingTitle, { target: { value: "Unsaved current edit" } });
    await waitFor(() => expect(eventHarness.handler).toBeTypeOf("function"));
    eventHarness.handler?.();
    eventHarness.handler?.();

    expect(await screen.findByRole("dialog", { name: "快速笔记" })).toBeVisible();
    expect(screen.getByLabelText("快速笔记正文")).toHaveTextContent("Rapid capture text");
    expect(screen.getByLabelText("笔记标题")).toHaveValue("Unsaved current edit");
    expect(screen.getAllByRole("dialog", { name: "快速笔记" })).toHaveLength(1);
    expect(captureTake).toBe(2);

    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "快速笔记" }))
      .not.toBeInTheDocument());
    expect(screen.getByLabelText("笔记标题")).toHaveValue("Unsaved current edit");
    expect(invokeMock.mock.calls.some(([command]) => command === "create_note")).toBe(false);
  });
});

function note(): Note {
  return {
    protocolVersion: 1,
    id: existingId,
    noteDate: "2026-09-07",
    title: "Existing dirty Note",
    bodyJson: {
      type: "doc",
      content: [{ type: "paragraph", content: [{ type: "text", text: "Existing body" }] }],
    },
    bodyFormat: "tiptap-json",
    bodySchemaVersion: 1,
    bodyText: "Existing body",
    contentHash: "0".repeat(64),
    isPinned: false,
    createdAtMs: 1_788_000_000_000,
    updatedAtMs: 1_788_000_000_000,
    revision: 1,
  };
}
