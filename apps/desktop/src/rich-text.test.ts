import { describe, expect, it } from "vitest";

import {
  canonicalRichTextDocument,
  extractBodyText,
  roundTripThroughSchema,
  validateRichTextDocument,
  type RichTextDocument,
} from "./rich-text/contract";

describe("the B04 rich-text persistence contract", () => {
  it("accepts every D08 format and produces stable canonical JSON and body_text", () => {
    const document: RichTextDocument = {
      type: "doc",
      content: [
        {
          type: "heading",
          attrs: { level: 2 },
          content: [{ type: "text", text: "Plan", marks: [{ type: "bold" }] }],
        },
        {
          type: "paragraph",
          content: [
            { type: "text", text: "Read " },
            {
              type: "text",
              text: "the reference",
              marks: [{ type: "link", attrs: { href: "https://example.com/reference" } }],
            },
          ],
        },
        {
          type: "bulletList",
          content: [{ type: "listItem", content: [{ type: "paragraph", content: [{ type: "text", text: "Bullet" }] }] }],
        },
        {
          type: "orderedList",
          attrs: { start: 1 },
          content: [{ type: "listItem", content: [{ type: "paragraph", content: [{ type: "text", text: "Number" }] }] }],
        },
        {
          type: "taskList",
          content: [{
            type: "taskItem",
            attrs: { checked: true },
            content: [{ type: "paragraph", content: [{ type: "text", text: "Done" }] }],
          }],
        },
        { type: "codeBlock", content: [{ type: "text", text: "const safe = true;" }] },
      ],
    };

    const validation = validateRichTextDocument(document);
    expect(validation.valid).toBe(true);
    const canonical = canonicalRichTextDocument(document);
    expect(canonical.content[3]).not.toHaveProperty("attrs");
    expect(roundTripThroughSchema(canonical)).toEqual(canonical);
    expect(canonicalRichTextDocument(canonical)).toEqual(canonical);
    expect(extractBodyText(canonical)).toBe(
      "Plan\nRead the reference\nBullet\nNumber\n[x] Done\nconst safe = true;",
    );
  });

  it("rejects malformed external links even when their scheme prefix looks allowed", () => {
    const malformed = {
      type: "doc",
      content: [{
        type: "paragraph",
        content: [{
          type: "text",
          text: "bad",
          marks: [{ type: "link", attrs: { href: "https://" } }],
        }],
      }],
    } as RichTextDocument;
    expect(validateRichTextDocument(malformed).valid).toBe(false);
    expect(() => canonicalRichTextDocument(malformed)).toThrow();
  });
});
