import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MAX_SEARCH_QUERY_CHARS, searchNotes } from "./search";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

describe("B06 typed Search transport", () => {
  beforeEach(() => invokeMock.mockReset());

  it("trims the query and invokes only the typed search command", async () => {
    invokeMock.mockResolvedValue({ ok: true, hits: [] });

    await expect(searchNotes("  天气 Search %_\"  ")).resolves.toEqual([]);
    expect(invokeMock).toHaveBeenCalledWith("search_notes", {
      request: { protocolVersion: 1, query: "天气 Search %_\"" },
    });
  });

  it("returns empty locally for whitespace and never crosses IPC", async () => {
    await expect(searchNotes(" \t\r\n ")).resolves.toEqual([]);
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("rejects character, UTF-8 byte, and control limits before IPC", async () => {
    for (const query of [
      "a".repeat(MAX_SEARCH_QUERY_CHARS + 1),
      "界".repeat(MAX_SEARCH_QUERY_CHARS + 1),
      "safe\u0000unsafe",
    ]) {
      await expect(searchNotes(query)).rejects.toMatchObject({ code: "VALIDATION_FAILED" });
    }
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("maps the controlled backend error without exposing a raw exception", async () => {
    invokeMock.mockResolvedValue({
      ok: false,
      error: { code: "VALIDATION_FAILED", message: "The request was not valid.", recoverable: true },
    });

    await expect(searchNotes("NEAR(foo)")).rejects.toMatchObject({
      code: "VALIDATION_FAILED",
      message: "The request was not valid.",
    });
  });
});
