import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { readBackupStatus } from "./backup";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);

describe("B09 backup status IPC", () => {
  beforeEach(() => invokeMock.mockReset());

  it("reads the typed health status without exposing package paths", async () => {
    invokeMock.mockResolvedValue({
      ok: true,
      status: {
        protocolVersion: 1,
        state: "healthy",
        lastSuccessAtMs: 1_789_750_800_000,
        lastSuccessLocalDay: "2026-09-07",
        validGenerationCount: 3,
        lastErrorCode: null,
      },
    });

    const status = await readBackupStatus();

    expect(status.state).toBe("healthy");
    expect(status.validGenerationCount).toBe(3);
    expect(invokeMock).toHaveBeenCalledWith("get_backup_status", {
      request: { protocolVersion: 1 },
    });
    expect(JSON.stringify(status)).not.toContain("storagePath");
  });

  it("preserves a classified recoverable failure", async () => {
    const failure = {
      code: "BACKUP_FAILED" as const,
      message: "The automatic backup could not be completed.",
      recoverable: true,
    };
    invokeMock.mockResolvedValue({ ok: false, error: failure });
    await expect(readBackupStatus()).rejects.toEqual(failure);
  });
});
