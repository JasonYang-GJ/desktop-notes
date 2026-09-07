import { invoke } from "@tauri-apps/api/core";

import { IPC_PROTOCOL_VERSION, type FoundationError } from "./foundation";
import { canonicalNoteDocument, EMPTY_NOTE_BODY, type TiptapDocument } from "./notes";

export type QuickCaptureContentType =
  | "text"
  | "image"
  | "text_and_image"
  | "empty"
  | "unsupported"
  | "busy"
  | "failed";

export interface QuickCaptureSession {
  protocolVersion: typeof IPC_PROTOCOL_VERSION;
  id: string;
  contentType: QuickCaptureContentType;
  text: string | null;
  dataBase64: string | null;
  mediaType: "image/png" | null;
  acquisitionAttempts: number;
}

export interface ShortcutStatus {
  protocolVersion: typeof IPC_PROTOCOL_VERSION;
  configuredShortcut: string;
  activeShortcut: string | null;
  registrationError: FoundationError["code"] | null;
}

type CaptureEnvelope =
  | { ok: true; session: QuickCaptureSession | null }
  | { ok: false; error: FoundationError };
type ShortcutEnvelope =
  | { ok: true; status: ShortcutStatus }
  | { ok: false; error: FoundationError };
type ActionEnvelope = { ok: true } | { ok: false; error: FoundationError };

const SESSION_ID = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu;
const MAX_CAPTURE_PARAGRAPHS = 4_000;

export async function takeQuickCapture(): Promise<QuickCaptureSession | undefined> {
  const response = await invoke<CaptureEnvelope>("take_quick_capture", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION },
  });
  if (!response.ok) throw response.error;
  if (response.session === null) return undefined;
  validateSession(response.session);
  return response.session;
}

export async function finishQuickCapture(sessionId: string): Promise<void> {
  const response = await invoke<ActionEnvelope>("finish_quick_capture", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, sessionId },
  });
  if (!response.ok) throw response.error;
}

export async function getShortcutConfig(): Promise<ShortcutStatus> {
  const response = await invoke<ShortcutEnvelope>("get_shortcut_config", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION },
  });
  if (!response.ok) throw response.error;
  return response.status;
}

export async function setShortcutConfig(shortcut: string): Promise<ShortcutStatus> {
  const response = await invoke<ShortcutEnvelope>("set_shortcut_config", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, shortcut },
  });
  if (!response.ok) throw response.error;
  return response.status;
}

export function localSuggestedTitle(text: string): string {
  const meaningfulLine = sanitizeClipboardText(text)
    .split("\n")
    .map((line) => line.trim())
    .find(Boolean);
  if (!meaningfulLine) return "";
  const sentence = meaningfulLine.split(/(?<=[。！？.!?])\s*/u, 1)[0] ?? meaningfulLine;
  const compact = sentence.replace(/\s+/gu, " ").trim();
  const points = Array.from(compact);
  return points.length <= 80 ? compact : `${points.slice(0, 79).join("")}…`;
}

export function capturedTextDocument(raw: string): {
  document: TiptapDocument;
  shortened: boolean;
} {
  const sanitized = sanitizeClipboardText(raw);
  if (!sanitized) return { document: EMPTY_NOTE_BODY, shortened: false };
  let candidate = sanitized;
  let shortened = false;
  for (;;) {
    const lines = candidate.split("\n");
    if (lines.length > MAX_CAPTURE_PARAGRAPHS) {
      candidate = lines.slice(0, MAX_CAPTURE_PARAGRAPHS).join("\n");
      shortened = true;
    }
    try {
      return { document: canonicalNoteDocument(documentFromText(candidate)), shortened };
    } catch {
      const points = Array.from(candidate);
      if (points.length <= 1) return { document: EMPTY_NOTE_BODY, shortened: true };
      candidate = points.slice(0, Math.max(1, Math.floor(points.length * 0.8))).join("");
      shortened = true;
    }
  }
}

export function appendImageToDocument(
  document: TiptapDocument,
  assetId: string,
): TiptapDocument {
  const content = document.content.length === 1
    && document.content[0]?.type === "paragraph"
    && !document.content[0].content
    ? []
    : document.content;
  return canonicalNoteDocument({
    type: "doc",
    content: [
      ...content,
      { type: "paragraph", content: [{ type: "imageRef", attrs: { asset_id: assetId } }] },
    ],
  });
}

export function quickCaptureNotice(contentType: QuickCaptureContentType): string | undefined {
  if (contentType === "empty") return "剪贴板为空，已打开一条空白快速笔记。";
  if (contentType === "unsupported") return "不支持该剪贴板格式，已打开一条空白快速笔记。";
  if (contentType === "busy") return "捕获期间剪贴板正忙或内容发生变化，已打开一条空白快速笔记。";
  if (contentType === "failed") return "无法安全读取剪贴板，已打开一条空白快速笔记。";
  return undefined;
}

function sanitizeClipboardText(value: string): string {
  const normalized = value.replace(/\r\n?/gu, "\n");
  let result = "";
  for (const character of normalized) {
    const code = character.codePointAt(0) ?? 0;
    if ((code <= 0x1f && code !== 0x09 && code !== 0x0a) || code === 0x7f) continue;
    result += character;
  }
  return result;
}

function documentFromText(text: string): TiptapDocument {
  return {
    type: "doc",
    content: text.split("\n").map((line) => line.length > 0
      ? { type: "paragraph" as const, content: [{ type: "text" as const, text: line }] }
      : { type: "paragraph" as const }),
  };
}

function validateSession(session: QuickCaptureSession): void {
  const types: readonly string[] = [
    "text",
    "image",
    "text_and_image",
    "empty",
    "unsupported",
    "busy",
    "failed",
  ];
  const hasText = session.contentType === "text" || session.contentType === "text_and_image";
  const hasImage = session.contentType === "image" || session.contentType === "text_and_image";
  if (
    session.protocolVersion !== IPC_PROTOCOL_VERSION
    || !SESSION_ID.test(session.id)
    || !types.includes(session.contentType)
    || !Number.isInteger(session.acquisitionAttempts)
    || session.acquisitionAttempts < 1
    || session.acquisitionAttempts > 4
    || (hasText ? typeof session.text !== "string" : session.text !== null)
    || (hasImage ? session.mediaType !== "image/png" || typeof session.dataBase64 !== "string"
      : session.mediaType !== null || session.dataBase64 !== null)
  ) {
    throw new Error("快速笔记响应未通过验证。");
  }
}
