import { invoke } from "@tauri-apps/api/core";

import { IPC_PROTOCOL_VERSION, type FoundationError } from "./foundation";
import type { Note, NoteSummary } from "./notes";

export const MAX_TAG_NAME_CHARS = 64;
export const MAX_TAG_NAME_BYTES = 256;

export interface Tag {
  protocolVersion: typeof IPC_PROTOCOL_VERSION;
  id: string;
  name: string;
  isSeedDefault: boolean;
  createdAtMs: number;
  updatedAtMs: number;
}

interface PreparedTagName {
  name: string;
  normalizedName: string;
}

type TagsEnvelope =
  | { ok: true; tags: Tag[] }
  | { ok: false; error: FoundationError };

type TagEnvelope =
  | { ok: true; tag: Tag }
  | { ok: false; error: FoundationError };

type DeleteTagEnvelope =
  | { ok: true; deletedTagId: string }
  | { ok: false; error: FoundationError };

type MetadataEnvelope =
  | { ok: true; note: Note; clientChangeId: string }
  | { ok: false; error: FoundationError };

type RecentEnvelope =
  | { ok: true; notes: NoteSummary[] }
  | { ok: false; error: FoundationError };

export function prepareTagName(value: string): PreparedTagName {
  const name = value.trim();
  const normalizedName = name.normalize("NFKC").toLowerCase();
  if (!withinTagNameBudget(name) || !withinTagNameBudget(normalizedName)) {
    throw validationError();
  }
  return { name, normalizedName };
}

export async function listTags(): Promise<Tag[]> {
  const response = await invoke<TagsEnvelope>("list_tags", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION },
  });
  if (!response.ok) throw response.error;
  return response.tags;
}

export async function listTagsForNote(noteId: string): Promise<Tag[]> {
  const response = await invoke<TagsEnvelope>("list_tags_for_note", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, noteId },
  });
  if (!response.ok) throw response.error;
  return response.tags;
}

export async function createTag(name: string): Promise<Tag> {
  const prepared = prepareTagName(name);
  const response = await invoke<TagEnvelope>("create_tag", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, name: prepared.name },
  });
  if (!response.ok) throw response.error;
  return response.tag;
}

export async function renameTag(tagId: string, name: string): Promise<Tag> {
  const prepared = prepareTagName(name);
  const response = await invoke<TagEnvelope>("rename_tag", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, tagId, name: prepared.name },
  });
  if (!response.ok) throw response.error;
  return response.tag;
}

export async function deleteTag(tagId: string): Promise<void> {
  const response = await invoke<DeleteTagEnvelope>("delete_tag", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, tagId },
  });
  if (!response.ok) throw response.error;
  if (response.deletedTagId !== tagId) {
    throw new Error("The Tag deletion acknowledgement did not match this request.");
  }
}

export function assignTag(noteId: string, tagId: string, baseRevision: number): Promise<Note> {
  return mutateNoteTag("assign_tag", noteId, tagId, baseRevision);
}

export function removeTag(noteId: string, tagId: string, baseRevision: number): Promise<Note> {
  return mutateNoteTag("remove_tag", noteId, tagId, baseRevision);
}

export function setNotePinned(
  noteId: string,
  isPinned: boolean,
  baseRevision: number,
): Promise<Note> {
  return mutateMetadata("set_note_pinned", {
    noteId,
    isPinned,
    baseRevision,
  });
}

export async function listRecentNotes(): Promise<NoteSummary[]> {
  const response = await invoke<RecentEnvelope>("list_recent_notes", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION },
  });
  if (!response.ok) throw response.error;
  return response.notes;
}

async function mutateNoteTag(
  command: "assign_tag" | "remove_tag",
  noteId: string,
  tagId: string,
  baseRevision: number,
): Promise<Note> {
  return mutateMetadata(command, { noteId, tagId, baseRevision });
}

async function mutateMetadata(
  command: "assign_tag" | "remove_tag" | "set_note_pinned",
  request: Record<string, string | number | boolean>,
): Promise<Note> {
  const clientChangeId = crypto.randomUUID();
  const response = await invoke<MetadataEnvelope>(command, {
    request: {
      protocolVersion: IPC_PROTOCOL_VERSION,
      ...request,
      clientChangeId,
    },
  });
  if (!response.ok) throw response.error;
  if (response.clientChangeId !== clientChangeId) {
    throw new Error("The metadata acknowledgement did not match this request.");
  }
  return response.note;
}

function withinTagNameBudget(value: string): boolean {
  return value.length > 0
    && Array.from(value).length <= MAX_TAG_NAME_CHARS
    && new TextEncoder().encode(value).byteLength <= MAX_TAG_NAME_BYTES;
}

function validationError(): FoundationError {
  return {
    code: "VALIDATION_FAILED",
    message: "The Tag name did not match the supported format.",
    recoverable: true,
  };
}

interface PinIntentCoordinatorOptions {
  initialPinned: boolean;
  flush: () => Promise<boolean>;
  save: (nextPinned: boolean) => Promise<Note>;
  apply: (saved: Note) => void;
  onBusyChange?: (busy: boolean) => void;
  onError?: (error: unknown) => void;
}

export class PinIntentCoordinator {
  private confirmed: boolean;
  private desired: boolean;
  private running?: Promise<void>;

  constructor(private readonly options: PinIntentCoordinatorOptions) {
    this.confirmed = options.initialPinned;
    this.desired = options.initialPinned;
  }

  get desiredPinned(): boolean {
    return this.desired;
  }

  get isBusy(): boolean {
    return this.running !== undefined;
  }

  waitForIdle(): Promise<void> {
    return this.running ?? Promise.resolve();
  }

  request(nextPinned: boolean): Promise<void> {
    this.desired = nextPinned;
    if (!this.running) {
      const run = this.drain();
      this.running = run;
      const clear = () => {
        if (this.running === run) this.running = undefined;
      };
      void run.then(clear, clear);
    }
    return this.running ?? Promise.resolve();
  }

  private async drain(): Promise<void> {
    this.options.onBusyChange?.(true);
    try {
      if (!await this.options.flush()) {
        throw new Error("The current edit could not be saved before changing Pin.");
      }
      while (this.desired !== this.confirmed) {
        const intended = this.desired;
        const saved = await this.options.save(intended);
        if (saved.isPinned !== intended) {
          throw new Error("The Pin acknowledgement did not match this request.");
        }
        this.confirmed = saved.isPinned;
        this.options.apply(saved);
      }
    } catch (error: unknown) {
      this.desired = this.confirmed;
      this.options.onError?.(error);
    } finally {
      this.options.onBusyChange?.(false);
    }
  }
}
