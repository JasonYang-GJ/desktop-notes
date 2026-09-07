import { invoke } from "@tauri-apps/api/core";
import { IPC_PROTOCOL_VERSION, type FoundationError } from "./foundation";

export interface SearchHit {
  id: string;
  noteDate: string;
  title: string;
  snippet: string;
  matchingTags: SearchMatchTag[];
  updatedAtMs: number;
  revision: number;
}

export interface SearchMatchTag {
  id: string;
  name: string;
}

type Envelope = { ok: true; hits: SearchHit[] } | { ok: false; error: FoundationError };
export const MAX_SEARCH_QUERY_CHARS = 256;
export const MAX_SEARCH_QUERY_BYTES = 1024;

export async function searchNotes(query: string): Promise<SearchHit[]> {
  const normalized = query.trim();
  if (normalized.length === 0) return [];
  if (
    new TextEncoder().encode(normalized).length > MAX_SEARCH_QUERY_BYTES
    || [...normalized].length > MAX_SEARCH_QUERY_CHARS
    || [...normalized].some((character) => {
      const codePoint = character.codePointAt(0) ?? 0;
      return codePoint <= 0x1f || codePoint === 0x7f;
    })
  ) {
    throw {
      code: "VALIDATION_FAILED",
      message: "Search query is invalid.",
      recoverable: true,
    } satisfies FoundationError;
  }
  const response = await invoke<Envelope>("search_notes", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, query: normalized },
  });
  if (!response.ok) throw response.error;
  return response.hits;
}
