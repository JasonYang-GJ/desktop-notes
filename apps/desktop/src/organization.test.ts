import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createTag,
  prepareTagName,
  renameTag,
  setNotePinned,
  PinIntentCoordinator,
} from "./organization";
import type { Note } from "./notes";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

function note(isPinned: boolean, revision: number): Note {
  return {
    protocolVersion: 1,
    id: "0e67ecfd-17f2-48f0-83c1-c10921016dbb",
    noteDate: "2026-09-06",
    title: "B05",
    bodyJson: { type: "doc", content: [{ type: "paragraph" }] },
    bodyFormat: "tiptap-json",
    bodySchemaVersion: 1,
    bodyText: "",
    contentHash: "0".repeat(64),
    isPinned,
    createdAtMs: 100,
    updatedAtMs: 100 + revision,
    revision,
  };
}

describe("B05 organization frontend boundary", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("0ed50682-84b2-4022-9869-c2a426d90586");
  });

  it("trims, NFKC-normalizes for uniqueness, and rejects empty or oversized names before IPC", async () => {
    expect(prepareTagName("  中文 Tag  ")).toEqual({
      name: "中文 Tag",
      normalizedName: "中文 tag",
    });
    expect(prepareTagName("ＰＲＯＪＥＣＴ").normalizedName).toBe("project");
    expect(() => prepareTagName("   ")).toThrow("supported format");
    expect(() => prepareTagName("x".repeat(65))).toThrow("supported format");

    await expect(createTag(" ")).rejects.toMatchObject({ code: "VALIDATION_FAILED" });
    await expect(renameTag("aa303a9a-fc89-4586-8f2c-73a556e5ed5e", "x".repeat(65)))
      .rejects.toMatchObject({ code: "VALIDATION_FAILED" });
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("rejects a metadata acknowledgement with the wrong client identity", async () => {
    invokeMock.mockResolvedValue({
      ok: true,
      note: note(true, 2),
      clientChangeId: "4c209297-3c0e-46f5-b5bd-9be95b17fd7a",
    });

    await expect(setNotePinned(note(false, 1).id, true, 1))
      .rejects.toThrow("did not match");
  });

  it("serializes rapid Pin intents so an older response cannot roll back the final state", async () => {
    const resolvers: Array<(value: Note) => void> = [];
    const saves: boolean[] = [];
    const applied: boolean[] = [];
    let confirmed = false;
    let revision = 1;
    const coordinator = new PinIntentCoordinator({
      initialPinned: false,
      flush: async () => true,
      save: (nextPinned) => {
        saves.push(nextPinned);
        return new Promise<Note>((resolve) => resolvers.push(resolve));
      },
      apply: (saved) => {
        confirmed = saved.isPinned;
        revision = saved.revision;
        applied.push(saved.isPinned);
      },
    });

    const first = coordinator.request(true);
    await vi.waitFor(() => expect(saves).toEqual([true]));
    void coordinator.request(false);
    resolvers.shift()?.(note(true, ++revision));
    await vi.waitFor(() => expect(saves).toEqual([true, false]));
    void coordinator.request(true);
    resolvers.shift()?.(note(false, ++revision));
    await vi.waitFor(() => expect(saves).toEqual([true, false, true]));
    resolvers.shift()?.(note(true, ++revision));
    await first;

    expect(applied).toEqual([true, false, true]);
    expect(confirmed).toBe(true);
    expect(coordinator.desiredPinned).toBe(true);
  });
});
