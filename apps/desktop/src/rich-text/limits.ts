export const MAX_BODY_JSON_BYTES = 1024 * 1024;
export const MAX_BODY_NODES = 10_000;
export const MAX_BODY_DEPTH = 32;

export type RichTextBudgetViolation = {
  code:
    | "BODY_JSON_BYTES_EXCEEDED"
    | "BODY_NODE_LIMIT_EXCEEDED"
    | "BODY_DEPTH_LIMIT_EXCEEDED"
    | "BODY_JSON_CYCLIC";
  message: string;
};

const utf8CodeUnitBytes = (value: string, index: number): { bytes: number; consumed: number } => {
  const code = value.charCodeAt(index);
  if (code <= 0x7f) return { bytes: 1, consumed: 1 };
  if (code <= 0x7ff) return { bytes: 2, consumed: 1 };
  if (code >= 0xd800 && code <= 0xdbff) {
    const next = value.charCodeAt(index + 1);
    if (next >= 0xdc00 && next <= 0xdfff) return { bytes: 4, consumed: 2 };
    return { bytes: 3, consumed: 1 };
  }
  return { bytes: 3, consumed: 1 };
};

export const utf8BytesWithinLimit = (value: string, maximum: number): boolean => {
  let bytes = 0;
  for (let index = 0; index < value.length;) {
    const current = utf8CodeUnitBytes(value, index);
    bytes += current.bytes;
    if (bytes > maximum) return false;
    index += current.consumed;
  }
  return true;
};

const jsonStringBytes = (value: string, maximum: number): number | undefined => {
  let bytes = 2;
  for (let index = 0; index < value.length;) {
    const code = value.charCodeAt(index);
    if (code === 0x22 || code === 0x5c || code === 0x08 || code === 0x0c
      || code === 0x0a || code === 0x0d || code === 0x09) {
      bytes += 2;
      index += 1;
    } else if (code < 0x20 || (code >= 0xd800 && code <= 0xdfff
      && !(code <= 0xdbff && value.charCodeAt(index + 1) >= 0xdc00
        && value.charCodeAt(index + 1) <= 0xdfff))) {
      bytes += 6;
      index += 1;
    } else {
      const current = utf8CodeUnitBytes(value, index);
      bytes += current.bytes;
      index += current.consumed;
    }
    if (bytes > maximum) return undefined;
  }
  return bytes;
};

const jsonBytesWithinLimit = (value: unknown, maximum: number): boolean => {
  const stack: unknown[] = [value];
  const seen = new WeakSet<object>();
  let bytes = 0;
  const add = (amount: number): boolean => {
    bytes += amount;
    return bytes <= maximum;
  };

  while (stack.length > 0) {
    const current = stack.pop();
    if (current === null) {
      if (!add(4)) return false;
      continue;
    }
    if (typeof current === "string") {
      const measured = jsonStringBytes(current, maximum - bytes);
      if (measured === undefined || !add(measured)) return false;
      continue;
    }
    if (typeof current === "number") {
      if (!add(Number.isFinite(current) ? String(current).length : 4)) return false;
      continue;
    }
    if (typeof current === "boolean") {
      if (!add(current ? 4 : 5)) return false;
      continue;
    }
    if (typeof current !== "object") {
      if (!add(4)) return false;
      continue;
    }
    if (seen.has(current)) return false;
    seen.add(current);
    if (Array.isArray(current)) {
      if (!add(2 + Math.max(0, current.length - 1))) return false;
      for (let index = current.length - 1; index >= 0; index -= 1) stack.push(current[index]);
      continue;
    }
    const entries = Object.entries(current);
    if (!add(2 + Math.max(0, entries.length - 1))) return false;
    for (let index = entries.length - 1; index >= 0; index -= 1) {
      const [key, child] = entries[index];
      const keyBytes = jsonStringBytes(key, maximum - bytes);
      if (keyBytes === undefined || !add(keyBytes + 1)) return false;
      stack.push(child);
    }
  }
  return true;
};

export const richTextBudgetViolation = (value: unknown): RichTextBudgetViolation | undefined => {
  if (!jsonBytesWithinLimit(value, MAX_BODY_JSON_BYTES)) {
    return {
      code: "BODY_JSON_BYTES_EXCEEDED",
      message: `canonical JSON must not exceed ${MAX_BODY_JSON_BYTES} UTF-8 bytes`,
    };
  }

  const stack: Array<{ value: unknown; depth: number }> = [{ value, depth: 0 }];
  const seen = new WeakSet<object>();
  let nodes = 0;
  while (stack.length > 0) {
    const current = stack.pop()!;
    if (!current.value || typeof current.value !== "object" || Array.isArray(current.value)) continue;
    if (seen.has(current.value)) {
      return { code: "BODY_JSON_CYCLIC", message: "canonical JSON must not contain cycles" };
    }
    seen.add(current.value);
    nodes += 1;
    if (nodes > MAX_BODY_NODES) {
      return { code: "BODY_NODE_LIMIT_EXCEEDED", message: `body must not exceed ${MAX_BODY_NODES} nodes` };
    }
    if (current.depth > MAX_BODY_DEPTH) {
      return { code: "BODY_DEPTH_LIMIT_EXCEEDED", message: `body depth must not exceed ${MAX_BODY_DEPTH}` };
    }
    const content = (current.value as Record<string, unknown>).content;
    if (Array.isArray(content)) {
      for (let index = content.length - 1; index >= 0; index -= 1) {
        stack.push({ value: content[index], depth: current.depth + 1 });
      }
    }
  }
  return undefined;
};
