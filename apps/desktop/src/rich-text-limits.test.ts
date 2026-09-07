import { describe, expect, it } from "vitest";

import { validateRichTextDocument } from "./rich-text/contract";
import {
  MAX_BODY_DEPTH,
  MAX_BODY_JSON_BYTES,
  MAX_BODY_NODES,
  richTextBudgetViolation,
} from "./rich-text/limits";

const nestedListDocument = (levels: number): unknown => {
  let list: unknown = {
    type: "bulletList",
    content: [{ type: "listItem", content: [{ type: "paragraph" }] }],
  };
  for (let level = 1; level < levels; level += 1) {
    list = {
      type: "bulletList",
      content: [{ type: "listItem", content: [{ type: "paragraph" }, list] }],
    };
  }
  return { type: "doc", content: [list] };
};

describe("B04 frontend rich-text resource limits", () => {
  it("matches the accepted Rust/application limits", () => {
    expect(MAX_BODY_JSON_BYTES).toBe(1024 * 1024);
    expect(MAX_BODY_NODES).toBe(10_000);
    expect(MAX_BODY_DEPTH).toBe(32);
  });

  it("accepts exactly 1 MiB of canonical JSON and rejects one byte more", () => {
    const template = {
      type: "doc",
      content: [{ type: "paragraph", content: [{ type: "text", text: "x" }] }],
    };
    const overhead = new TextEncoder().encode(JSON.stringify(template)).byteLength - 1;
    const exact = {
      type: "doc",
      content: [{
        type: "paragraph",
        content: [{ type: "text", text: "x".repeat(MAX_BODY_JSON_BYTES - overhead) }],
      }],
    };
    expect(new TextEncoder().encode(JSON.stringify(exact))).toHaveLength(MAX_BODY_JSON_BYTES);
    expect(validateRichTextDocument(exact).valid).toBe(true);

    const over = structuredClone(exact);
    over.content[0].content[0].text += "x";
    expect(validateRichTextDocument(over)).toMatchObject({
      valid: false,
      errors: [{ keyword: "resourceLimit" }],
    });
  });

  it("accepts 10,000 semantic nodes and rejects node 10,001", () => {
    const exact = {
      type: "doc",
      content: Array.from({ length: MAX_BODY_NODES - 1 }, () => ({ type: "paragraph" })),
    };
    expect(richTextBudgetViolation(exact)).toBeUndefined();
    expect(validateRichTextDocument(exact).valid).toBe(true);

    exact.content.push({ type: "paragraph" });
    expect(richTextBudgetViolation(exact)?.code).toBe("BODY_NODE_LIMIT_EXCEEDED");
  });

  it("accepts the deepest schema-valid list and rejects content beyond depth 32", () => {
    expect(validateRichTextDocument(nestedListDocument(15)).valid).toBe(true);
    expect(richTextBudgetViolation(nestedListDocument(16))?.code).toBe("BODY_DEPTH_LIMIT_EXCEEDED");
    expect(validateRichTextDocument(nestedListDocument(16)).valid).toBe(false);
  });
});
