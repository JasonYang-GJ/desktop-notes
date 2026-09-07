import { invoke } from "@tauri-apps/api/core";

import { IPC_PROTOCOL_VERSION, type FoundationError } from "./foundation";
import { canonicalRichTextDocument } from "./rich-text/contract";
import {
  RichTextMigrationRegistry,
  type RichTextLoadResult,
  type StoredRichTextEnvelope,
} from "./rich-text/migration";
import type { RichTextDocument } from "./rich-text/types";

export const BODY_FORMAT = "tiptap-json" as const;
export const BODY_SCHEMA_VERSION = 1 as const;

export type TiptapDocument = RichTextDocument;

export const EMPTY_NOTE_BODY: TiptapDocument = {
  type: "doc",
  content: [{ type: "paragraph" }],
};

export interface Note {
  protocolVersion: typeof IPC_PROTOCOL_VERSION;
  id: string;
  noteDate: string;
  title: string;
  bodyJson: TiptapDocument;
  bodyFormat: typeof BODY_FORMAT;
  bodySchemaVersion: typeof BODY_SCHEMA_VERSION;
  bodyText: string;
  contentHash: string;
  isPinned: boolean;
  createdAtMs: number;
  updatedAtMs: number;
  revision: number;
}

export interface NoteSummary {
  protocolVersion: typeof IPC_PROTOCOL_VERSION;
  id: string;
  noteDate: string;
  title: string;
  isPinned: boolean;
  updatedAtMs: number;
  revision: number;
}

export interface DateCount {
  noteDate: string;
  count: number;
}

export interface DateChangeUndoToken {
  tokenId: string;
  expiresAtMs: number;
}

export interface DateChangeReceipt {
  note: Note;
  previousDate: string;
  undoToken: DateChangeUndoToken;
}

type NoteEnvelope =
  | { ok: true; note: Note; clientChangeId?: string }
  | { ok: false; error: FoundationError };

type NotesEnvelope =
  | { ok: true; notes: NoteSummary[] }
  | { ok: false; error: FoundationError };

type NoteDeletedEnvelope =
  | { ok: true; deletedNoteId: string; clientChangeId: string }
  | { ok: false; error: FoundationError };

type DateCountsEnvelope =
  | { ok: true; counts: DateCount[] }
  | { ok: false; error: FoundationError };

type DateChangedEnvelope =
  | {
      ok: true;
      note: Note;
      previousDate: string;
      undoToken: DateChangeUndoToken;
      clientChangeId: string;
    }
  | { ok: false; error: FoundationError };

type DateUndoEnvelope =
  | { ok: true; note: Note; clientChangeId: string }
  | { ok: false; error: FoundationError };

export interface NoteUpdateDraft {
  noteId: string;
  noteDate: string;
  title: string;
  bodyJson: TiptapDocument;
  baseRevision: number;
  clientChangeId: string;
}

export async function createNote(noteDate: string): Promise<Note> {
  return createNoteFromDraft(noteDate, "", EMPTY_NOTE_BODY);
}

export async function createNoteFromDraft(
  noteDate: string,
  title: string,
  bodyJson: TiptapDocument,
): Promise<Note> {
  const canonicalBody = canonicalNoteDocument(bodyJson);
  const response = await invoke<NoteEnvelope>("create_note", {
    request: {
      protocolVersion: IPC_PROTOCOL_VERSION,
      noteDate,
      title,
      bodyFormat: BODY_FORMAT,
      bodySchemaVersion: BODY_SCHEMA_VERSION,
      bodyJson: canonicalBody,
    },
  });
  return noteFrom(response);
}

export async function getNote(noteId: string): Promise<Note> {
  const response = await invoke<NoteEnvelope>("get_note", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, noteId },
  });
  return noteFrom(response);
}

export async function deleteNote(noteId: string, baseRevision: number): Promise<void> {
  const clientChangeId = crypto.randomUUID();
  const response = await invoke<NoteDeletedEnvelope>("delete_note", {
    request: {
      protocolVersion: IPC_PROTOCOL_VERSION,
      noteId,
      baseRevision,
      clientChangeId,
    },
  });
  if (!response.ok) throw response.error;
  if (response.deletedNoteId !== noteId || response.clientChangeId !== clientChangeId) {
    throw new Error("The delete acknowledgement did not match this request.");
  }
}

export async function listNotesForDate(noteDate: string): Promise<NoteSummary[]> {
  const response = await invoke<NotesEnvelope>("list_notes_for_date", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, noteDate },
  });
  if (!response.ok) throw response.error;
  return response.notes;
}

export async function updateNoteContent(draft: NoteUpdateDraft): Promise<Note> {
  const bodyJson = canonicalNoteDocument(draft.bodyJson);
  const canonicalJson = JSON.stringify(bodyJson);
  const contentHash = await sha256Hex(
    `desktop-notes-note-content-v1\0${draft.noteDate}\0${draft.title}\0${canonicalJson}`,
  );
  const response = await invoke<NoteEnvelope>("update_note_content", {
    request: {
      protocolVersion: IPC_PROTOCOL_VERSION,
      noteId: draft.noteId,
      title: draft.title,
      bodyFormat: BODY_FORMAT,
      bodySchemaVersion: BODY_SCHEMA_VERSION,
      bodyJson,
      baseRevision: draft.baseRevision,
      clientChangeId: draft.clientChangeId,
      contentHash,
    },
  });
  if (!response.ok) throw response.error;
  if (response.clientChangeId !== draft.clientChangeId) {
    throw new Error("The save acknowledgement did not match this edit.");
  }
  return response.note;
}

export async function listNoteCountsForMonth(month: string): Promise<DateCount[]> {
  const response = await invoke<DateCountsEnvelope>("list_note_counts_for_month", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, month },
  });
  if (!response.ok) throw response.error;
  return response.counts;
}

export async function changeNoteDate(
  noteId: string,
  newNoteDate: string,
  baseRevision: number,
): Promise<DateChangeReceipt> {
  const clientChangeId = crypto.randomUUID();
  const response = await invoke<DateChangedEnvelope>("change_note_date", {
    request: {
      protocolVersion: IPC_PROTOCOL_VERSION,
      noteId,
      newNoteDate,
      baseRevision,
      clientChangeId,
    },
  });
  if (!response.ok) throw response.error;
  if (response.clientChangeId !== clientChangeId) {
    throw new Error("The date-change acknowledgement did not match this request.");
  }
  return {
    note: response.note,
    previousDate: response.previousDate,
    undoToken: response.undoToken,
  };
}

export async function undoNoteDateChange(tokenId: string): Promise<Note> {
  const clientChangeId = crypto.randomUUID();
  const response = await invoke<DateUndoEnvelope>("undo_note_date_change", {
    request: {
      protocolVersion: IPC_PROTOCOL_VERSION,
      tokenId,
      clientChangeId,
    },
  });
  if (!response.ok) throw response.error;
  if (response.clientChangeId !== clientChangeId) {
    throw new Error("The date-undo acknowledgement did not match this request.");
  }
  return response.note;
}

export function canonicalNoteDocument(value: TiptapDocument): TiptapDocument {
  return canonicalRichTextDocument(value);
}

export const canonicalBasicDocument = canonicalNoteDocument;

export function sameBasicDocument(left: TiptapDocument, right: TiptapDocument): boolean {
  return JSON.stringify(canonicalNoteDocument(left)) === JSON.stringify(canonicalNoteDocument(right));
}

export function localDateToday(now = new Date()): string {
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function noteFrom(response: NoteEnvelope): Note {
  if (!response.ok) throw response.error;
  const note = response.note;
  const envelope: StoredRichTextEnvelope = {
    body_schema_version: note.bodySchemaVersion,
    body_json: note.bodyJson,
  };
  const loaded = richTextMigrations.load(envelope);
  if (loaded.mode === "read-only") throw new RichTextRecoveryError(loaded);
  try {
    return {
      ...note,
      bodySchemaVersion: BODY_SCHEMA_VERSION,
      bodyJson: canonicalNoteDocument(loaded.envelope.body_json as TiptapDocument),
    };
  } catch {
    throw new RichTextRecoveryError({
      mode: "read-only",
      original_envelope: structuredClone(envelope),
      original_preserved: true,
      applied: loaded.applied,
      error_code: "RICH_TEXT_MIGRATION_FAILED",
      error_detail: "Stored rich-text content failed the approved v1 schema.",
    });
  }
}

const richTextMigrations = new RichTextMigrationRegistry(BODY_SCHEMA_VERSION);

export class RichTextRecoveryError extends Error {
  readonly code: "RICH_TEXT_VERSION_UNSUPPORTED" | "RICH_TEXT_MIGRATION_FAILED";
  readonly originalEnvelope: StoredRichTextEnvelope;

  constructor(result: Extract<RichTextLoadResult, { mode: "read-only" }>) {
    super(result.error_detail);
    this.name = "RichTextRecoveryError";
    this.code = result.error_code;
    this.originalEnvelope = structuredClone(result.original_envelope);
  }
}

export const isRichTextRecoveryError = (value: unknown): value is RichTextRecoveryError =>
  value instanceof RichTextRecoveryError;

async function sha256Hex(value: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}
