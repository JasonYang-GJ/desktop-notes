import { getSchema } from "@tiptap/core";

import richTextSchema from "../../../../contracts/rich-text/tiptap-v1.schema.json";
import { canonicalize } from "./canonical";
import { s04Extensions } from "./extensions";
import { isApprovedExternalLink } from "./link-policy";
import { richTextBudgetViolation } from "./limits";
import type { BlockNode, InlineNode, RichTextDoc } from "./types";
import { createRichTextValidator } from "./validator";

export type { RichTextDocument } from "./types";
export { canonicalStringify } from "./canonical";

const validate = createRichTextValidator(richTextSchema);

const containsOnlyApprovedLinks = (value: unknown): boolean => {
  if (Array.isArray(value)) return value.every(containsOnlyApprovedLinks);
  if (!value || typeof value !== "object") return true;
  const object = value as Record<string, unknown>;
  if (object.type === "link") {
    const attrs = object.attrs as Record<string, unknown> | undefined;
    if (!attrs || typeof attrs.href !== "string" || !isApprovedExternalLink(attrs.href)) return false;
  }
  return Object.values(object).every(containsOnlyApprovedLinks);
};

export const validateRichTextDocument = (value: unknown) => {
  const budget = richTextBudgetViolation(value);
  if (budget) {
    return {
      valid: false as const,
      errors: [{
        keyword: "resourceLimit",
        instancePath: "",
        schemaPath: `#/${budget.code}`,
        params: { code: budget.code },
        message: budget.message,
      }],
    };
  }
  const result = validate(value);
  if (!result.valid || containsOnlyApprovedLinks(result.document)) return result;
  return {
    valid: false as const,
    errors: [{
      keyword: "format",
      instancePath: "",
      schemaPath: "#/approvedExternalLink",
      params: {},
      message: "external link is malformed or unsafe",
    }],
  };
};

export const canonicalRichTextDocument = (document: RichTextDoc): RichTextDoc => {
  const validation = validateRichTextDocument(document);
  if (!validation.valid) {
    const resourceLimit = validation.errors.some((error) => error.keyword === "resourceLimit");
    throw new Error(resourceLimit
      ? "Rich-text content exceeds the approved resource limit."
      : "Rich-text content does not match the approved v1 schema.");
  }
  return canonicalize(validation.document);
};

export const roundTripThroughSchema = (document: RichTextDoc): RichTextDoc => {
  const canonical = canonicalRichTextDocument(document);
  const schema = getSchema(s04Extensions);
  return canonicalRichTextDocument(schema.nodeFromJSON(canonical).toJSON() as RichTextDoc);
};

const inlineText = (node: InlineNode): string => {
  if (node.type === "text") return node.text;
  if (node.type === "noteLink") return node.attrs.display_text ?? node.attrs.target_note_id;
  return "";
};

const blockText = (node: BlockNode): string => {
  switch (node.type) {
    case "paragraph":
    case "heading":
      return (node.content ?? []).map(inlineText).join("");
    case "codeBlock":
      return (node.content ?? []).map((child) => child.text).join("");
    case "bulletList":
    case "orderedList":
      return node.content
        .map((item) => item.content.map(blockText).filter(Boolean).join("\n"))
        .filter(Boolean)
        .join("\n");
    case "taskList":
      return node.content
        .map((item) => {
          const content = item.content.map(blockText).filter(Boolean).join("\n");
          return `${item.attrs.checked ? "[x]" : "[ ]"} ${content}`.trimEnd();
        })
        .join("\n");
  }
};

export const extractBodyText = (document: RichTextDoc): string =>
  document.content.map(blockText).filter(Boolean).join("\n");

export { s04Extensions as RICH_TEXT_EXTENSIONS } from "./extensions";
