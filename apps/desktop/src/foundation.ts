import { invoke } from "@tauri-apps/api/core";

export const FOUNDATION_COMMAND = "get_foundation_status" as const;
export const IPC_PROTOCOL_VERSION = 1 as const;

export type DatabaseState = "fresh" | "reopened";

export interface FoundationStatus {
  protocolVersion: typeof IPC_PROTOCOL_VERSION;
  ready: true;
  view: "today_normal_empty";
  databaseState: DatabaseState;
  schemaVersion: number;
  encryption: "sqlcipher";
  backupStatus?: BackupStatus;
}

export interface BackupStatus {
  protocolVersion: typeof IPC_PROTOCOL_VERSION;
  state: "never" | "healthy" | "failed";
  lastSuccessAtMs: number | null;
  lastSuccessLocalDay: string | null;
  validGenerationCount: number;
  lastErrorCode: FoundationError["code"] | null;
}

export interface FoundationError {
  code:
    | "KEY_UNAVAILABLE"
    | "DATABASE_WRONG_KEY"
    | "DATABASE_CORRUPTED"
    | "DATABASE_SCHEMA_TOO_NEW"
    | "MIGRATION_FAILED"
    | "DATA_ROOT_UNAVAILABLE"
    | "DISK_FULL"
    | "WEBVIEW_UNAVAILABLE"
    | "VALIDATION_FAILED"
    | "NOTE_NOT_FOUND"
    | "REVISION_CONFLICT"
    | "UNDO_EXPIRED"
    | "TAG_NOT_FOUND"
    | "TAG_NAME_CONFLICT"
    | "IMAGE_REJECTED"
    | "IMAGE_TOO_LARGE"
    | "ASSET_NOT_FOUND"
    | "ASSET_CORRUPTED"
    | "SHORTCUT_INVALID"
    | "SHORTCUT_CONFLICT"
    | "CAPTURE_SESSION_MISMATCH"
    | "BACKUP_BUSY"
    | "BACKUP_FAILED"
    | "BACKUP_CORRUPTED"
    | "BACKUP_UNSUPPORTED"
    | "INTERNAL_ERROR";
  message: string;
  recoverable: boolean;
}

export type FoundationEnvelope =
  | { ok: true; status: FoundationStatus }
  | { ok: false; error: FoundationError };

export function readFoundationStatus(): Promise<FoundationEnvelope> {
  return invoke<FoundationEnvelope>(FOUNDATION_COMMAND, {
    request: { protocolVersion: IPC_PROTOCOL_VERSION },
  });
}

export function isFoundationError(value: unknown): value is FoundationError {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<FoundationError>;
  return (
    typeof candidate.code === "string" &&
    typeof candidate.message === "string" &&
    typeof candidate.recoverable === "boolean"
  );
}
