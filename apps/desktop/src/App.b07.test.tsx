import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CalendarWorkspace } from "./App";
import type { FoundationStatus } from "./foundation";
import { clearImageMemoryCache } from "./images";
import type { Note } from "./notes";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const noteId = "73000000-0000-4000-8000-000000000001";
const assetId = "73000000-0000-4000-8000-000000000002";
const status: FoundationStatus = {
  protocolVersion: 1,
  ready: true,
  view: "today_normal_empty",
  databaseState: "reopened",
  schemaVersion: 5,
  encryption: "sqlcipher",
};

describe("B07 editor image lifecycle", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    clearImageMemoryCache();
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("73000000-0000-4000-8000-000000000003");
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("pastes an imageRef, renders the encrypted import result, and autosaves it", async () => {
    const initial = note({ type: "doc", content: [{ type: "paragraph" }] }, 1);
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [initial] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: initial };
      if (command === "list_tags" || command === "list_tags_for_note") return { ok: true, tags: [] };
      if (command === "import_image_asset") return { ok: true, image: imageResponse() };
      if (command === "update_note_content") {
        const request = (args as { request: { bodyJson: Note["bodyJson"]; clientChangeId: string } }).request;
        return {
          ok: true,
          clientChangeId: request.clientChangeId,
          note: note(request.bodyJson, 2),
        };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /B07 image note/ }));
    const body = await screen.findByLabelText("笔记正文");
    const file = {
      type: "image/png",
      size: 4,
      arrayBuffer: async () => new Uint8Array([1, 2, 3, 4]).buffer,
    } as File;
    fireEvent.paste(body, {
      clipboardData: {
        items: [{ kind: "file", type: "image/png", getAsFile: () => file }],
        getData: () => "",
      },
    });

    await waitFor(() => expect(document.querySelector(`.image-ref[data-asset-id="${assetId}"] img`))
      .toHaveAttribute("src", "data:image/png;base64,AQIDBA=="));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "update_note_content",
      expect.objectContaining({
        request: expect.objectContaining({
          bodyJson: expect.objectContaining({
            content: expect.arrayContaining([
              expect.objectContaining({
                content: expect.arrayContaining([
                  { type: "imageRef", attrs: { asset_id: assetId } },
                ]),
              }),
            ]),
          }),
          baseRevision: 1,
        }),
      }),
    ), { timeout: 2_500 });
    expect(await screen.findByText("已保存")).toBeVisible();
  });

  it("loads a persisted imageRef through the note-scoped read command", async () => {
    const restarted = note({
      type: "doc",
      content: [{
        type: "paragraph",
        content: [
          { type: "text", text: "before" },
          { type: "imageRef", attrs: { asset_id: assetId, display_width: 320 } },
          { type: "text", text: "after" },
        ],
      }],
    }, 2);
    invokeMock.mockImplementation(async (command) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [restarted] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: restarted };
      if (command === "list_tags" || command === "list_tags_for_note") return { ok: true, tags: [] };
      if (command === "read_image_asset") return { ok: true, image: imageResponse() };
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /B07 image note/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("read_image_asset", {
      request: { protocolVersion: 1, noteId, assetId },
    }));
    const image = document.querySelector(`.image-ref[data-asset-id="${assetId}"] img`);
    await waitFor(() => expect(image).toHaveAttribute("src", "data:image/png;base64,AQIDBA=="));
    expect(image).toHaveStyle({ width: "320px" });
    expect(screen.getByLabelText("笔记正文")).toHaveTextContent("beforeafter");
  });
});

function note(bodyJson: Note["bodyJson"], revision: number): Note {
  return {
    protocolVersion: 1,
    id: noteId,
    noteDate: "2026-09-06",
    title: "B07 image note",
    bodyJson,
    bodyFormat: "tiptap-json",
    bodySchemaVersion: 1,
    bodyText: "",
    contentHash: "0".repeat(64),
    isPinned: false,
    createdAtMs: 1_780_000_000_000,
    updatedAtMs: 1_780_000_000_000 + revision,
    revision,
  };
}

function imageResponse() {
  return {
    protocolVersion: 1,
    assetId,
    mediaType: "image/png",
    pixelWidth: 37,
    pixelHeight: 23,
    dataBase64: "AQIDBA==",
  };
}
