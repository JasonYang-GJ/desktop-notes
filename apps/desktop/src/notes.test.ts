import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  MAX_BODY_JSON_BYTES,
  MAX_BODY_NODES,
} from "./rich-text/limits";
import {
  EMPTY_NOTE_BODY,
  changeNoteDate,
  createNote,
  deleteNote,
  listNoteCountsForMonth,
  localDateToday,
  sameBasicDocument,
  undoNoteDateChange,
  updateNoteContent,
} from "./notes";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

const storedNote = {
  protocolVersion: 1 as const,
  id: "b30256a2-a873-4589-aadc-d8e63fc7fb4d",
  noteDate: "2026-09-06",
  title: "",
  bodyJson: EMPTY_NOTE_BODY,
  bodyFormat: "tiptap-json" as const,
  bodySchemaVersion: 1 as const,
  bodyText: "",
  contentHash: "0".repeat(64),
  createdAtMs: 1,
  updatedAtMs: 1,
  revision: 1,
};

describe("typed B02 Note client", () => {
  beforeEach(() => invokeMock.mockReset());

  it("creates a Note for an explicit local date using Tiptap JSON v1", async () => {
    invokeMock.mockResolvedValue({ ok: true, note: storedNote });

    await createNote("2026-09-06");

    expect(invokeMock).toHaveBeenCalledWith("create_note", {
      request: {
        protocolVersion: 1,
        noteDate: "2026-09-06",
        title: "",
        bodyFormat: "tiptap-json",
        bodySchemaVersion: 1,
        bodyJson: EMPTY_NOTE_BODY,
      },
    });
  });

  it("sends revision, change identity, and the canonical content hash on update", async () => {
    invokeMock.mockResolvedValue({
      ok: true,
      note: { id: "b30256a2-a873-4589-aadc-d8e63fc7fb4d", revision: 2 },
      clientChangeId: "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71",
    });
    const body = {
      type: "doc" as const,
      content: [{
        type: "paragraph" as const,
        content: [{ type: "text" as const, text: "B02 synthetic body marker" }],
      }],
    };

    await updateNoteContent({
      noteId: "b30256a2-a873-4589-aadc-d8e63fc7fb4d",
      noteDate: "2026-09-06",
      title: "B02 Test Note",
      bodyJson: body,
      baseRevision: 1,
      clientChangeId: "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71",
    });

    expect(invokeMock).toHaveBeenCalledWith("update_note_content", {
      request: expect.objectContaining({
        noteId: "b30256a2-a873-4589-aadc-d8e63fc7fb4d",
        baseRevision: 1,
        clientChangeId: "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71",
        contentHash: "1e97795a07d98cfbe90b5be461050e6b085048e9f4b6134d0ae4e44ed9573914",
      }),
    });
  });

  it("deletes a Note through a revision-guarded typed request", async () => {
    const randomUuid = vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("a137ee02-e0db-42f2-818f-a8275ae59f11");
    invokeMock.mockResolvedValue({
      ok: true,
      deletedNoteId: storedNote.id,
      clientChangeId: "a137ee02-e0db-42f2-818f-a8275ae59f11",
    });

    await deleteNote(storedNote.id, 3);

    expect(invokeMock).toHaveBeenCalledWith("delete_note", {
      request: {
        protocolVersion: 1,
        noteId: storedNote.id,
        baseRevision: 3,
        clientChangeId: "a137ee02-e0db-42f2-818f-a8275ae59f11",
      },
    });
    randomUuid.mockRestore();
  });

  it("keeps approved rich-text nodes and marks in the canonical IPC payload", async () => {
    invokeMock.mockResolvedValue({
      ok: true,
      note: { id: "b30256a2-a873-4589-aadc-d8e63fc7fb4d", revision: 2 },
      clientChangeId: "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71",
    });
    const body = {
      type: "doc" as const,
      content: [
        {
          type: "heading" as const,
          attrs: { level: 2 as const },
          content: [{ type: "text" as const, text: "Important", marks: [{ type: "bold" as const }] }],
        },
        {
          type: "paragraph" as const,
          content: [{
            type: "text" as const,
            text: "Docs",
            marks: [{ type: "link" as const, attrs: { href: "https://example.com/docs" } }],
          }],
        },
      ],
    };

    await updateNoteContent({
      noteId: "b30256a2-a873-4589-aadc-d8e63fc7fb4d",
      noteDate: "2026-09-06",
      title: "B04 Test Note",
      bodyJson: body,
      baseRevision: 1,
      clientChangeId: "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71",
    });

    expect(invokeMock).toHaveBeenCalledWith("update_note_content", {
      request: expect.objectContaining({ bodyJson: body }),
    });
  });

  it("rejects oversized, over-node and over-depth bodies before typed IPC", async () => {
    const draft = {
      noteId: "b30256a2-a873-4589-aadc-d8e63fc7fb4d",
      noteDate: "2026-09-06",
      title: "B04 resource budget",
      bodyJson: EMPTY_NOTE_BODY,
      baseRevision: 1,
      clientChangeId: "ec0ec3f0-c0ca-4f64-8ab4-350512a41b71",
    };
    const oversizedBody = {
      type: "doc" as const,
      content: [{
        type: "paragraph" as const,
        content: [{ type: "text" as const, text: "x".repeat(MAX_BODY_JSON_BYTES) }],
      }],
    };
    await expect(updateNoteContent({ ...draft, bodyJson: oversizedBody })).rejects.toThrow(/resource limit/i);

    const overNodeBody = {
      type: "doc" as const,
      content: Array.from({ length: MAX_BODY_NODES }, () => ({ type: "paragraph" as const })),
    };
    await expect(updateNoteContent({ ...draft, bodyJson: overNodeBody })).rejects.toThrow(/resource limit/i);

    let nestedList: unknown = {
      type: "bulletList",
      content: [{ type: "listItem", content: [{ type: "paragraph" }] }],
    };
    for (let level = 1; level < 16; level += 1) {
      nestedList = {
        type: "bulletList",
        content: [{ type: "listItem", content: [{ type: "paragraph" }, nestedList] }],
      };
    }
    const overDepthBody = { type: "doc", content: [nestedList] } as typeof EMPTY_NOTE_BODY;
    await expect(updateNoteContent({ ...draft, bodyJson: overDepthBody })).rejects.toThrow(/resource limit/i);

    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("formats Today from local calendar fields rather than UTC midnight", () => {
    expect(localDateToday(new Date(2026, 8, 6, 0, 5))).toBe("2026-09-06");
  });

  it("treats a normalized editor echo as unchanged content", () => {
    expect(sameBasicDocument(
      { type: "doc", content: [{ type: "paragraph", content: [] }] },
      EMPTY_NOTE_BODY,
    )).toBe(true);
    expect(sameBasicDocument(
      EMPTY_NOTE_BODY,
      {
        type: "doc",
        content: [{ type: "paragraph", content: [{ type: "text", text: "changed" }] }],
      },
    )).toBe(false);
  });

  it("loads repository-backed month counts through a narrow typed command", async () => {
    invokeMock.mockResolvedValue({
      ok: true,
      counts: [{ noteDate: "2026-09-06", count: 2 }],
    });

    await expect(listNoteCountsForMonth("2026-09")).resolves.toEqual([
      { noteDate: "2026-09-06", count: 2 },
    ]);
    expect(invokeMock).toHaveBeenCalledWith("list_note_counts_for_month", {
      request: { protocolVersion: 1, month: "2026-09" },
    });
  });

  it("changes and undoes Note Date with revision and client identity", async () => {
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValueOnce("6089c54a-6a2a-4916-8a70-986cd12ec46f")
      .mockReturnValueOnce("7860c12d-c89b-477e-9bc7-d779d6cf3c58");
    const note = { id: "b30256a2-a873-4589-aadc-d8e63fc7fb4d", noteDate: "2026-09-10", revision: 2 };
    invokeMock
      .mockResolvedValueOnce({
        ok: true,
        note,
        previousDate: "2026-09-06",
        undoToken: {
          tokenId: "e52c6c84-e694-46f5-a982-846d9b68fb8b",
          expiresAtMs: 12_000,
        },
        clientChangeId: "6089c54a-6a2a-4916-8a70-986cd12ec46f",
      })
      .mockResolvedValueOnce({
        ok: true,
        note: { ...note, noteDate: "2026-09-06", revision: 3 },
        clientChangeId: "7860c12d-c89b-477e-9bc7-d779d6cf3c58",
      });

    const changed = await changeNoteDate(note.id, "2026-09-10", 1);
    expect(changed.previousDate).toBe("2026-09-06");
    expect(invokeMock).toHaveBeenNthCalledWith(1, "change_note_date", {
      request: {
        protocolVersion: 1,
        noteId: note.id,
        newNoteDate: "2026-09-10",
        baseRevision: 1,
        clientChangeId: "6089c54a-6a2a-4916-8a70-986cd12ec46f",
      },
    });

    await expect(undoNoteDateChange(changed.undoToken.tokenId)).resolves.toMatchObject({
      noteDate: "2026-09-06",
      revision: 3,
    });
    expect(invokeMock).toHaveBeenNthCalledWith(2, "undo_note_date_change", {
      request: {
        protocolVersion: 1,
        tokenId: "e52c6c84-e694-46f5-a982-846d9b68fb8b",
        clientChangeId: "7860c12d-c89b-477e-9bc7-d779d6cf3c58",
      },
    });
  });
});
