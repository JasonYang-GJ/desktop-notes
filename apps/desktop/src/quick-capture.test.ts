import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  appendImageToDocument,
  capturedTextDocument,
  finishQuickCapture,
  getShortcutConfig,
  localSuggestedTitle,
  setShortcutConfig,
  takeQuickCapture,
} from "./quick-capture";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const sessionId = "81000000-0000-4000-8000-000000000001";
const assetId = "81000000-0000-4000-8000-000000000002";

describe("B08 quick-capture client contract", () => {
  beforeEach(() => invokeMock.mockReset());

  it("builds deterministic local title and safe paragraphs without external work", () => {
    expect(localSuggestedTitle("\r\n  第一条有意义的句子。后续不会进标题\n第二行"))
      .toBe("第一条有意义的句子。");
    const prepared = capturedTextDocument("first\r\nsecond\u0000\u0007");
    expect(prepared.shortened).toBe(false);
    expect(prepared.document).toEqual({
      type: "doc",
      content: [
        { type: "paragraph", content: [{ type: "text", text: "first" }] },
        { type: "paragraph", content: [{ type: "text", text: "second" }] },
      ],
    });
  });

  it("appends only an asset reference, never captured base64", () => {
    const document = appendImageToDocument(
      capturedTextDocument("caption").document,
      assetId,
    );
    expect(document.content).toEqual([
      { type: "paragraph", content: [{ type: "text", text: "caption" }] },
      { type: "paragraph", content: [{ type: "imageRef", attrs: { asset_id: assetId } }] },
    ]);
    expect(JSON.stringify(document)).not.toContain("base64");
  });

  it("takes at most one pending session and validates text-plus-image payloads", async () => {
    invokeMock.mockResolvedValueOnce({
      ok: true,
      session: {
        protocolVersion: 1,
        id: sessionId,
        contentType: "text_and_image",
        text: "caption",
        dataBase64: "AQIDBA==",
        mediaType: "image/png",
        acquisitionAttempts: 1,
      },
    }).mockResolvedValueOnce({ ok: true, session: null });

    await expect(takeQuickCapture()).resolves.toMatchObject({
      id: sessionId,
      contentType: "text_and_image",
    });
    await expect(takeQuickCapture()).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenNthCalledWith(1, "take_quick_capture", {
      request: { protocolVersion: 1 },
    });
  });

  it("uses typed commands for session completion and shortcut reconfiguration", async () => {
    const status = {
      protocolVersion: 1,
      configuredShortcut: "Ctrl+Alt+Q",
      activeShortcut: "Ctrl+Alt+Q",
      registrationError: null,
    };
    invokeMock
      .mockResolvedValueOnce({ ok: true, status })
      .mockResolvedValueOnce({ ok: true, status })
      .mockResolvedValueOnce({ ok: true });

    await expect(getShortcutConfig()).resolves.toEqual(status);
    await expect(setShortcutConfig("Ctrl+Alt+Q")).resolves.toEqual(status);
    await expect(finishQuickCapture(sessionId)).resolves.toBeUndefined();
    expect(invokeMock).toHaveBeenNthCalledWith(2, "set_shortcut_config", {
      request: { protocolVersion: 1, shortcut: "Ctrl+Alt+Q" },
    });
    expect(invokeMock).toHaveBeenNthCalledWith(3, "finish_quick_capture", {
      request: { protocolVersion: 1, sessionId },
    });
  });
});
