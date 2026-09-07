import { invoke } from "@tauri-apps/api/core";

import { IPC_PROTOCOL_VERSION, type FoundationError } from "./foundation";

export const MAX_IMAGE_INPUT_BYTES = 20 * 1024 * 1024;
export const APPROVED_IMAGE_MEDIA_TYPES = ["image/png", "image/jpeg", "image/webp"] as const;
export type ApprovedImageMediaType = typeof APPROVED_IMAGE_MEDIA_TYPES[number];

export interface ImageAsset {
  protocolVersion: typeof IPC_PROTOCOL_VERSION;
  assetId: string;
  mediaType: "image/png";
  pixelWidth: number;
  pixelHeight: number;
  dataBase64: string;
}

type ImageAssetEnvelope =
  | { ok: true; image: ImageAsset }
  | { ok: false; error: FoundationError };

type ActionEnvelope = { ok: true } | { ok: false; error: FoundationError };

const inMemoryDisplayUrls = new Map<string, string>();
let cacheGeneration = 0;

export async function importPastedImage(
  noteId: string,
  file: File,
  clientImportId = crypto.randomUUID(),
): Promise<ImageAsset> {
  if (!isApprovedImageMediaType(file.type)) throw imageRejected();
  if (file.size <= 0 || file.size > MAX_IMAGE_INPUT_BYTES) throw imageTooLarge();
  const bytes = new Uint8Array(await file.arrayBuffer());
  if (bytes.byteLength !== file.size || bytes.byteLength > MAX_IMAGE_INPUT_BYTES) {
    throw imageTooLarge();
  }
  return importImageBytes(noteId, clientImportId, file.type, bytes, file.type === "image/png"
    ? "clipboard_screenshot"
    : "editor_paste");
}

export async function importCapturedPng(
  captureSessionId: string,
  dataBase64: string,
): Promise<ImageAsset> {
  if (dataBase64.length === 0 || dataBase64.length > 45 * 1024 * 1024) throw imageTooLarge();
  let binary: string;
  try {
    binary = atob(dataBase64);
  } catch {
    throw imageRejected();
  }
  if (binary.length === 0 || binary.length > MAX_IMAGE_INPUT_BYTES) throw imageTooLarge();
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  return importImageBytes(
    captureSessionId,
    captureSessionId,
    "image/png",
    bytes,
    "clipboard_screenshot",
  );
}

export async function discardStagedImage(clientImportId: string): Promise<void> {
  const response = await invoke<ActionEnvelope>("discard_image_asset", {
    request: { protocolVersion: IPC_PROTOCOL_VERSION, clientImportId },
  });
  if (!response.ok) throw response.error;
  forgetImageContext(clientImportId);
}

export function forgetImageContext(noteId: string): void {
  const prefix = `${noteId}\0`;
  for (const key of inMemoryDisplayUrls.keys()) {
    if (key.startsWith(prefix)) inMemoryDisplayUrls.delete(key);
  }
}

async function importImageBytes(
  noteId: string,
  clientImportId: string,
  mediaType: ApprovedImageMediaType,
  bytes: Uint8Array,
  source: "editor_paste" | "clipboard_screenshot",
): Promise<ImageAsset> {
  const generation = cacheGeneration;
  const response = await invoke<ImageAssetEnvelope>("import_image_asset", {
    request: {
      protocolVersion: IPC_PROTOCOL_VERSION,
      clientImportId,
      mediaType,
      source,
      dataBase64: bytesToBase64(bytes),
    },
  });
  if (!response.ok) throw response.error;
  validateImageAsset(response.image);
  if (generation === cacheGeneration) {
    inMemoryDisplayUrls.set(cacheKey(noteId, response.image.assetId), imageDataUrl(response.image));
  }
  return response.image;
}

export async function imageDisplayUrl(noteId: string, assetId: string): Promise<string> {
  const pending = inMemoryDisplayUrls.get(cacheKey(noteId, assetId));
  if (pending) return pending;
  const generation = cacheGeneration;
  const response = await invoke<ImageAssetEnvelope>("read_image_asset", {
    request: {
      protocolVersion: IPC_PROTOCOL_VERSION,
      noteId,
      assetId,
    },
  });
  if (!response.ok) throw response.error;
  validateImageAsset(response.image);
  if (response.image.assetId !== assetId) throw imageRejected();
  const url = imageDataUrl(response.image);
  if (generation === cacheGeneration) inMemoryDisplayUrls.set(cacheKey(noteId, assetId), url);
  return url;
}

export function clearImageMemoryCache(): void {
  cacheGeneration += 1;
  inMemoryDisplayUrls.clear();
}

function isApprovedImageMediaType(value: string): value is ApprovedImageMediaType {
  return (APPROVED_IMAGE_MEDIA_TYPES as readonly string[]).includes(value);
}

function validateImageAsset(image: ImageAsset): void {
  if (
    image.protocolVersion !== IPC_PROTOCOL_VERSION
    || image.mediaType !== "image/png"
    || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(image.assetId)
    || !Number.isSafeInteger(image.pixelWidth)
    || !Number.isSafeInteger(image.pixelHeight)
    || image.pixelWidth <= 0
    || image.pixelHeight <= 0
    || image.pixelWidth > 8192
    || image.pixelHeight > 8192
    || image.dataBase64.length === 0
    || image.dataBase64.length > 45 * 1024 * 1024
  ) {
    throw imageRejected();
  }
}

function imageDataUrl(image: ImageAsset): string {
  return `data:image/png;base64,${image.dataBase64}`;
}

function cacheKey(noteId: string, assetId: string): string {
  return `${noteId}\0${assetId}`;
}

function bytesToBase64(bytes: Uint8Array): string {
  const chunks: string[] = [];
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    chunks.push(String.fromCharCode(...bytes.subarray(offset, offset + chunkSize)));
  }
  return btoa(chunks.join(""));
}

function imageRejected(): FoundationError {
  return {
    code: "IMAGE_REJECTED",
    message: "The pasted image was rejected.",
    recoverable: true,
  };
}

function imageTooLarge(): FoundationError {
  return {
    code: "IMAGE_TOO_LARGE",
    message: "The pasted image exceeds the approved limit.",
    recoverable: true,
  };
}
