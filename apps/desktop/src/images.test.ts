import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  clearImageMemoryCache,
  imageDisplayUrl,
  importPastedImage,
  MAX_IMAGE_INPUT_BYTES,
} from "./images";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const assetId = "72000000-0000-4000-8000-000000000001";

describe("B07 image IPC client", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    clearImageMemoryCache();
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("72000000-0000-4000-8000-000000000002");
  });

  afterEach(() => vi.restoreAllMocks());

  it("sends a bounded approved clipboard image and keeps display bytes in memory", async () => {
    invokeMock.mockResolvedValue({
      ok: true,
      image: imageResponse("AQIDBA=="),
    });
    const file = fakeFile("image/png", new Uint8Array([1, 2, 3, 4]));

    const imported = await importPastedImage("72000000-0000-4000-8000-000000000099", file);
    expect(imported.assetId).toBe(assetId);
    expect(invokeMock).toHaveBeenCalledWith("import_image_asset", {
      request: {
        protocolVersion: 1,
        clientImportId: "72000000-0000-4000-8000-000000000002",
        mediaType: "image/png",
        source: "clipboard_screenshot",
        dataBase64: "AQIDBA==",
      },
    });
    await expect(imageDisplayUrl("72000000-0000-4000-8000-000000000099", assetId))
      .resolves.toBe("data:image/png;base64,AQIDBA==");
    expect(invokeMock).toHaveBeenCalledTimes(1);
  });

  it("loads a committed asset through the note-scoped read command after restart", async () => {
    invokeMock.mockResolvedValue({ ok: true, image: imageResponse("iVBORw0KGgo=") });
    await expect(imageDisplayUrl("72000000-0000-4000-8000-000000000099", assetId))
      .resolves.toBe("data:image/png;base64,iVBORw0KGgo=");
    expect(invokeMock).toHaveBeenCalledWith("read_image_asset", {
      request: {
        protocolVersion: 1,
        noteId: "72000000-0000-4000-8000-000000000099",
        assetId,
      },
    });
  });

  it("does not repopulate decrypted display cache after its Note context is cleared", async () => {
    let resolveFirst: ((value: unknown) => void) | undefined;
    invokeMock.mockImplementationOnce(() => new Promise((resolve) => {
      resolveFirst = resolve;
    }));
    const pending = imageDisplayUrl("72000000-0000-4000-8000-000000000099", assetId);
    clearImageMemoryCache();
    resolveFirst?.({ ok: true, image: imageResponse("iVBORw0KGgo=") });
    await expect(pending).resolves.toBe("data:image/png;base64,iVBORw0KGgo=");

    invokeMock.mockResolvedValueOnce({ ok: true, image: imageResponse("iVBORw0KGgo=") });
    await imageDisplayUrl("72000000-0000-4000-8000-000000000099", assetId);
    expect(invokeMock).toHaveBeenCalledTimes(2);
  });

  it("rejects unsupported and oversized clipboard files before IPC", async () => {
    await expect(importPastedImage("72000000-0000-4000-8000-000000000099", fakeFile("image/svg+xml", new Uint8Array([1]))))
      .rejects.toMatchObject({ code: "IMAGE_REJECTED" });
    await expect(importPastedImage("72000000-0000-4000-8000-000000000099", {
      type: "image/png",
      size: MAX_IMAGE_INPUT_BYTES + 1,
      arrayBuffer: async () => new ArrayBuffer(0),
    } as File)).rejects.toMatchObject({ code: "IMAGE_TOO_LARGE" });
    expect(invokeMock).not.toHaveBeenCalled();
  });
});

function fakeFile(type: string, bytes: Uint8Array): File {
  return {
    type,
    size: bytes.byteLength,
    arrayBuffer: async () => bytes.buffer.slice(0),
  } as File;
}

function imageResponse(dataBase64: string) {
  return {
    protocolVersion: 1,
    assetId,
    mediaType: "image/png",
    pixelWidth: 37,
    pixelHeight: 23,
    dataBase64,
  };
}
