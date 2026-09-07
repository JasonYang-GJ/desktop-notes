import { invoke } from "@tauri-apps/api/core";

import {
  IPC_PROTOCOL_VERSION,
  type BackupStatus,
  type FoundationError,
} from "./foundation";

type BackupEnvelope =
  | { ok: true; status: BackupStatus }
  | { ok: false; error: FoundationError };

export async function readBackupStatus(): Promise<BackupStatus> {
  const response = await invoke<BackupEnvelope>("get_backup_status", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION },
  });
  if (!response.ok) throw response.error;
  return response.status;
}
