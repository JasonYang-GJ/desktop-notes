import type { RichTextDoc } from "./types";

export interface ValidationIssue {
  keyword: string;
  instancePath: string;
  schemaPath: string;
  params: Record<string, unknown>;
  message: string;
}

export type ValidationResult =
  | { valid: true; document: RichTextDoc; errors: [] }
  | { valid: false; errors: ValidationIssue[] };

type JsonObject = Record<string, unknown>;

const UUID = /^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$/;
const LINK_PREFIX = /^(?:https?:\/\/|mailto:)/i;

const issue = (instancePath: string, message: string, keyword = "schema"): ValidationIssue => ({
  keyword,
  instancePath,
  schemaPath: "#/$defs",
  params: {},
  message,
});

const objectAt = (value: unknown, path: string): JsonObject | ValidationIssue => (
  value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as JsonObject
    : issue(path, "must be an object", "type")
);

const isIssue = (value: JsonObject | ValidationIssue): value is ValidationIssue => (
  typeof value.keyword === "string"
  && typeof value.instancePath === "string"
  && typeof value.schemaPath === "string"
  && value.params !== null
  && typeof value.params === "object"
  && typeof value.message === "string"
);

const exactKeys = (
  object: JsonObject,
  allowed: readonly string[],
  required: readonly string[],
  path: string,
): ValidationIssue | undefined => {
  for (const key of required) {
    if (!Object.hasOwn(object, key)) return issue(path, `must have required property '${key}'`, "required");
  }
  for (const key of Object.keys(object)) {
    if (!allowed.includes(key)) return issue(`${path}/${key}`, "must NOT have additional properties", "additionalProperties");
  }
  return undefined;
};

const stringLength = (value: string): number => Array.from(value).length;

const validateTextMark = (value: unknown, path: string): ValidationIssue | undefined => {
  const object = objectAt(value, path);
  if (isIssue(object)) return object;
  if (object.type === "bold") return exactKeys(object, ["type"], ["type"], path);
  if (object.type !== "link") return issue(`${path}/type`, "must be an approved mark", "oneOf");
  const keys = exactKeys(object, ["type", "attrs"], ["type", "attrs"], path);
  if (keys) return keys;
  const attrs = objectAt(object.attrs, `${path}/attrs`);
  if (isIssue(attrs)) return attrs;
  const attrKeys = exactKeys(attrs, ["href"], ["href"], `${path}/attrs`);
  if (attrKeys) return attrKeys;
  if (typeof attrs.href !== "string" || stringLength(attrs.href) < 1 || stringLength(attrs.href) > 2048) {
    return issue(`${path}/attrs/href`, "must be a non-empty string of at most 2048 characters", "length");
  }
  return LINK_PREFIX.test(attrs.href)
    ? undefined
    : issue(`${path}/attrs/href`, "must use the approved external-link prefix", "pattern");
};

const validateInline = (value: unknown, path: string): ValidationIssue | undefined => {
  const object = objectAt(value, path);
  if (isIssue(object)) return object;
  if (object.type === "text") {
    const keys = exactKeys(object, ["type", "text", "marks"], ["type", "text"], path);
    if (keys) return keys;
    if (typeof object.text !== "string" || stringLength(object.text) < 1) {
      return issue(`${path}/text`, "must be a non-empty string", "minLength");
    }
    if (object.marks === undefined) return undefined;
    if (!Array.isArray(object.marks) || object.marks.length > 2) {
      return issue(`${path}/marks`, "must be an array with at most two marks", "maxItems");
    }
    const seen = new Set<string>();
    for (let index = 0; index < object.marks.length; index += 1) {
      const error = validateTextMark(object.marks[index], `${path}/marks/${index}`);
      if (error) return error;
      const markType = (object.marks[index] as JsonObject).type as string;
      if (seen.has(markType)) return issue(`${path}/marks/${index}`, "must not repeat a mark type", "uniqueItems");
      seen.add(markType);
    }
    return undefined;
  }
  if (object.type === "imageRef") {
    const keys = exactKeys(object, ["type", "attrs"], ["type", "attrs"], path);
    if (keys) return keys;
    const attrs = objectAt(object.attrs, `${path}/attrs`);
    if (isIssue(attrs)) return attrs;
    const attrKeys = exactKeys(attrs, ["asset_id", "display_width"], ["asset_id"], `${path}/attrs`);
    if (attrKeys) return attrKeys;
    if (typeof attrs.asset_id !== "string" || !UUID.test(attrs.asset_id)) {
      return issue(`${path}/attrs/asset_id`, "must be a UUID", "pattern");
    }
    if (attrs.display_width !== undefined
      && (!Number.isInteger(attrs.display_width) || (attrs.display_width as number) < 48 || (attrs.display_width as number) > 4096)) {
      return issue(`${path}/attrs/display_width`, "must be an integer from 48 through 4096", "range");
    }
    return undefined;
  }
  if (object.type === "noteLink") {
    const keys = exactKeys(object, ["type", "attrs"], ["type", "attrs"], path);
    if (keys) return keys;
    const attrs = objectAt(object.attrs, `${path}/attrs`);
    if (isIssue(attrs)) return attrs;
    const attrKeys = exactKeys(attrs, ["target_note_id", "display_text"], ["target_note_id"], `${path}/attrs`);
    if (attrKeys) return attrKeys;
    if (typeof attrs.target_note_id !== "string" || !UUID.test(attrs.target_note_id)) {
      return issue(`${path}/attrs/target_note_id`, "must be a UUID", "pattern");
    }
    if (attrs.display_text !== undefined
      && (typeof attrs.display_text !== "string"
        || stringLength(attrs.display_text) < 1
        || stringLength(attrs.display_text) > 500)) {
      return issue(`${path}/attrs/display_text`, "must be a string from 1 through 500 characters", "length");
    }
    return undefined;
  }
  return issue(`${path}/type`, "must be an approved inline node", "oneOf");
};

const validateInlineContent = (value: unknown, path: string): ValidationIssue | undefined => {
  if (value === undefined) return undefined;
  if (!Array.isArray(value)) return issue(path, "must be an array", "type");
  for (let index = 0; index < value.length; index += 1) {
    const error = validateInline(value[index], `${path}/${index}`);
    if (error) return error;
  }
  return undefined;
};

const validateParagraph = (object: JsonObject, path: string): ValidationIssue | undefined => {
  const keys = exactKeys(object, ["type", "content"], ["type"], path);
  return keys ?? validateInlineContent(object.content, `${path}/content`);
};

const validateListBlock = (value: unknown, path: string): ValidationIssue | undefined => {
  const object = objectAt(value, path);
  if (isIssue(object)) return object;
  return ["bulletList", "orderedList", "taskList"].includes(String(object.type))
    ? validateBlock(value, path)
    : issue(`${path}/type`, "must be an approved nested list", "oneOf");
};

const validateListItem = (value: unknown, path: string, task: boolean): ValidationIssue | undefined => {
  const object = objectAt(value, path);
  if (isIssue(object)) return object;
  const expectedType = task ? "taskItem" : "listItem";
  const keys = exactKeys(
    object,
    task ? ["type", "attrs", "content"] : ["type", "content"],
    task ? ["type", "attrs", "content"] : ["type", "content"],
    path,
  );
  if (keys) return keys;
  if (object.type !== expectedType) return issue(`${path}/type`, `must equal '${expectedType}'`, "const");
  if (task) {
    const attrs = objectAt(object.attrs, `${path}/attrs`);
    if (isIssue(attrs)) return attrs;
    const attrKeys = exactKeys(attrs, ["checked"], ["checked"], `${path}/attrs`);
    if (attrKeys) return attrKeys;
    if (typeof attrs.checked !== "boolean") return issue(`${path}/attrs/checked`, "must be boolean", "type");
  }
  if (!Array.isArray(object.content) || object.content.length < 1) {
    return issue(`${path}/content`, "must be a non-empty array", "minItems");
  }
  const first = objectAt(object.content[0], `${path}/content/0`);
  if (isIssue(first)) return first;
  if (first.type !== "paragraph") return issue(`${path}/content/0/type`, "first list child must be a paragraph", "prefixItems");
  const paragraphError = validateParagraph(first, `${path}/content/0`);
  if (paragraphError) return paragraphError;
  for (let index = 1; index < object.content.length; index += 1) {
    const error = validateListBlock(object.content[index], `${path}/content/${index}`);
    if (error) return error;
  }
  return undefined;
};

const validateList = (object: JsonObject, path: string, kind: "bulletList" | "orderedList" | "taskList"): ValidationIssue | undefined => {
  const ordered = kind === "orderedList";
  const keys = exactKeys(object, ordered ? ["type", "attrs", "content"] : ["type", "content"], ["type", "content"], path);
  if (keys) return keys;
  if (ordered && object.attrs !== undefined) {
    const attrs = objectAt(object.attrs, `${path}/attrs`);
    if (isIssue(attrs)) return attrs;
    const attrKeys = exactKeys(attrs, ["start"], ["start"], `${path}/attrs`);
    if (attrKeys) return attrKeys;
    if (!Number.isInteger(attrs.start) || (attrs.start as number) < 1 || (attrs.start as number) > 1_000_000) {
      return issue(`${path}/attrs/start`, "must be an integer from 1 through 1000000", "range");
    }
  }
  if (!Array.isArray(object.content) || object.content.length < 1) {
    return issue(`${path}/content`, "must be a non-empty array", "minItems");
  }
  for (let index = 0; index < object.content.length; index += 1) {
    const error = validateListItem(object.content[index], `${path}/content/${index}`, kind === "taskList");
    if (error) return error;
  }
  return undefined;
};

const validateBlock = (value: unknown, path: string): ValidationIssue | undefined => {
  const object = objectAt(value, path);
  if (isIssue(object)) return object;
  if (object.type === "paragraph") return validateParagraph(object, path);
  if (object.type === "heading") {
    const keys = exactKeys(object, ["type", "attrs", "content"], ["type", "attrs"], path);
    if (keys) return keys;
    const attrs = objectAt(object.attrs, `${path}/attrs`);
    if (isIssue(attrs)) return attrs;
    const attrKeys = exactKeys(attrs, ["level"], ["level"], `${path}/attrs`);
    if (attrKeys) return attrKeys;
    if (!Number.isInteger(attrs.level) || ![1, 2, 3, 4, 5, 6].includes(attrs.level as number)) {
      return issue(`${path}/attrs/level`, "must be a heading level from 1 through 6", "enum");
    }
    return validateInlineContent(object.content, `${path}/content`);
  }
  if (object.type === "codeBlock") {
    const keys = exactKeys(object, ["type", "content"], ["type"], path);
    if (keys) return keys;
    if (object.content === undefined) return undefined;
    if (!Array.isArray(object.content)) return issue(`${path}/content`, "must be an array", "type");
    for (let index = 0; index < object.content.length; index += 1) {
      const childPath = `${path}/content/${index}`;
      const child = objectAt(object.content[index], childPath);
      if (isIssue(child)) return child;
      const childKeys = exactKeys(child, ["type", "text"], ["type", "text"], childPath);
      if (childKeys) return childKeys;
      if (child.type !== "text" || typeof child.text !== "string" || stringLength(child.text) < 1) {
        return issue(childPath, "code content must be plain non-empty text", "schema");
      }
    }
    return undefined;
  }
  if (object.type === "bulletList" || object.type === "orderedList" || object.type === "taskList") {
    return validateList(object, path, object.type);
  }
  return issue(`${path}/type`, "must be an approved block node", "oneOf");
};

const validateDocument = (value: unknown): ValidationIssue | undefined => {
  const object = objectAt(value, "");
  if (isIssue(object)) return object;
  const keys = exactKeys(object, ["type", "content"], ["type", "content"], "");
  if (keys) return keys;
  if (object.type !== "doc") return issue("/type", "must equal 'doc'", "const");
  if (!Array.isArray(object.content) || object.content.length < 1) {
    return issue("/content", "must be a non-empty array", "minItems");
  }
  for (let index = 0; index < object.content.length; index += 1) {
    const error = validateBlock(object.content[index], `/content/${index}`);
    if (error) return error;
  }
  return undefined;
};

export const createRichTextValidator = (schema: object) => {
  const schemaId = (schema as JsonObject).$id;
  if (schemaId !== "https://desktop-notes.local/contracts/rich-text/tiptap-v1.schema.json") {
    throw new TypeError("Unsupported rich-text schema");
  }

  return (value: unknown): ValidationResult => {
    const error = validateDocument(value);
    return error
      ? { valid: false, errors: [error] }
      : { valid: true, document: value as RichTextDoc, errors: [] };
  };
};
