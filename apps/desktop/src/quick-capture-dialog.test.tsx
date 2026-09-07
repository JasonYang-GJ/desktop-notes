import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { QuickCaptureDialog } from "./quick-capture-dialog";
import type { Note } from "./notes";
import type { QuickCaptureSession } from "./quick-capture";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const sessionId = "82000000-0000-4000-8000-000000000001";
const noteId = "82000000-0000-4000-8000-000000000002";
const assetId = "82000000-0000-4000-8000-000000000003";
const tagId = "82000000-0000-4000-8000-000000000004";

describe("B08 Quick Capture review dialog", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("82000000-0000-4000-8000-000000000005");
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("reviews text, editable title/date/tags, then creates exactly one deliberate Note", async () => {
    const onSaved = vi.fn();
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_tags") return { ok: true, tags: [tag()] };
      if (command === "create_note") {
        const request = (args as { request: {
          noteDate: string;
          title: string;
          bodyJson: Note["bodyJson"];
        } }).request;
        return { ok: true, note: note(request.noteDate, request.title, request.bodyJson, 1) };
      }
      if (command === "assign_tag") {
        const request = (args as { request: { baseRevision: number; clientChangeId: string } }).request;
        return {
          ok: true,
          clientChangeId: request.clientChangeId,
          note: note("2026-09-08", "Edited title", textBody("Captured body"), 2),
        };
      }
      if (command === "finish_quick_capture") return { ok: true };
      throw new Error(`Unexpected command ${command}`);
    });

    render(
      <QuickCaptureDialog
        session={textSession()}
        defaultDate="2026-09-07"
        onSaved={onSaved}
        onCancelled={vi.fn()}
      />,
    );
    expect(await screen.findByLabelText("快速笔记正文")).toHaveTextContent("Captured body");
    expect(screen.getByLabelText("快速笔记标题")).toHaveValue("Captured body");
    fireEvent.change(screen.getByLabelText("快速笔记标题"), { target: { value: "Edited title" } });
    fireEvent.change(screen.getByLabelText("快速笔记日期"), { target: { value: "2026-09-08" } });
    fireEvent.click(await screen.findByRole("checkbox", { name: "Work" }));
    fireEvent.click(screen.getByRole("button", { name: "保存快速笔记" }));

    await waitFor(() => expect(onSaved).toHaveBeenCalledTimes(1));
    expect(invokeMock).toHaveBeenCalledWith("create_note", {
      request: expect.objectContaining({
        protocolVersion: 1,
        noteDate: "2026-09-08",
        title: "Edited title",
        bodyJson: textBody("Captured body"),
      }),
    });
    expect(invokeMock).toHaveBeenCalledWith("assign_tag", {
      request: expect.objectContaining({ noteId, tagId, baseRevision: 1 }),
    });
    expect(invokeMock).toHaveBeenCalledWith("finish_quick_capture", {
      request: { protocolVersion: 1, sessionId },
    });
  });

  it("stages a captured image through B07 and cancel removes it without creating a Note", async () => {
    const onCancelled = vi.fn();
    invokeMock.mockImplementation(async (command) => {
      if (command === "list_tags") return { ok: true, tags: [] };
      if (command === "import_image_asset") return {
        ok: true,
        image: {
          protocolVersion: 1,
          assetId,
          mediaType: "image/png",
          pixelWidth: 2,
          pixelHeight: 2,
          dataBase64: "AQIDBA==",
        },
      };
      if (command === "discard_image_asset" || command === "finish_quick_capture") {
        return { ok: true };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(
      <QuickCaptureDialog
        session={imageSession()}
        defaultDate="2026-09-07"
        onSaved={vi.fn()}
        onCancelled={onCancelled}
      />,
    );
    await waitFor(() => expect(document.querySelector(`.image-ref[data-asset-id="${assetId}"]`))
      .toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "取消" }));

    await waitFor(() => expect(onCancelled).toHaveBeenCalledTimes(1));
    expect(invokeMock).toHaveBeenCalledWith("discard_image_asset", {
      request: { protocolVersion: 1, clientImportId: sessionId },
    });
    expect(invokeMock.mock.calls.some(([command]) => command === "create_note")).toBe(false);
  });

  it("opens an empty clipboard as an unsaved blank Note and cancel leaves no residue", async () => {
    const onCancelled = vi.fn();
    invokeMock.mockImplementation(async (command) => {
      if (command === "list_tags") return { ok: true, tags: [] };
      if (command === "finish_quick_capture") return { ok: true };
      throw new Error(`Unexpected command ${command}`);
    });
    render(
      <QuickCaptureDialog
        session={{ ...textSession(), contentType: "empty", text: null }}
        defaultDate="2026-09-07"
        onSaved={vi.fn()}
        onCancelled={onCancelled}
      />,
    );
    expect(await screen.findByText(/剪贴板为空/i)).toBeVisible();
    expect(screen.getByLabelText("快速笔记标题")).toHaveValue("");
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() => expect(onCancelled).toHaveBeenCalledTimes(1));
    expect(invokeMock.mock.calls.some(([command]) => command === "create_note")).toBe(false);
    expect(invokeMock.mock.calls.some(([command]) => command === "discard_image_asset")).toBe(false);
  });

  it("keeps the in-memory review open when deliberate save fails", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "list_tags") return { ok: true, tags: [] };
      if (command === "create_note") throw new Error("synthetic failure");
      throw new Error(`Unexpected command ${command}`);
    });
    render(
      <QuickCaptureDialog
        session={textSession()}
        defaultDate="2026-09-07"
        onSaved={vi.fn()}
        onCancelled={vi.fn()}
      />,
    );
    fireEvent.click(await screen.findByRole("button", { name: "保存快速笔记" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("当前内容仍保持打开");
    expect(screen.getByLabelText("快速笔记正文")).toHaveTextContent("Captured body");
  });
});

function textSession(): QuickCaptureSession {
  return {
    protocolVersion: 1,
    id: sessionId,
    contentType: "text",
    text: "Captured body",
    dataBase64: null,
    mediaType: null,
    acquisitionAttempts: 1,
  };
}

function imageSession(): QuickCaptureSession {
  return {
    ...textSession(),
    contentType: "image",
    text: null,
    dataBase64: "AQIDBA==",
    mediaType: "image/png",
  };
}

function textBody(text: string): Note["bodyJson"] {
  return { type: "doc", content: [{ type: "paragraph", content: [{ type: "text", text }] }] };
}

function note(noteDate: string, title: string, bodyJson: Note["bodyJson"], revision: number): Note {
  return {
    protocolVersion: 1,
    id: noteId,
    noteDate,
    title,
    bodyJson,
    bodyFormat: "tiptap-json",
    bodySchemaVersion: 1,
    bodyText: "Captured body",
    contentHash: "0".repeat(64),
    isPinned: false,
    createdAtMs: 1_788_000_000_000,
    updatedAtMs: 1_788_000_000_000 + revision,
    revision,
  };
}

function tag() {
  return {
    protocolVersion: 1 as const,
    id: tagId,
    name: "Work",
    isSeedDefault: true,
    createdAtMs: 1,
    updatedAtMs: 1,
  };
}
