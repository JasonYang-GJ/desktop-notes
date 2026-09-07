import { describe, expect, it } from "vitest";

import sanitizerCases from "../../../tests/fixtures/rich-text/sanitizer-cases.json";
import { extractBodyText, validateRichTextDocument } from "./rich-text/contract";
import { MAX_BODY_JSON_BYTES, MAX_BODY_NODES } from "./rich-text/limits";
import { sanitizePastedHtmlForPersistence } from "./rich-text/sanitizer";

describe("the promoted S04 paste sanitizer", () => {
  it("downgrades ordinary ChatGPT inline code to safe text instead of blocking the paste", () => {
    const result = sanitizePastedHtmlForPersistence(
      [
        "<h2><strong>Ling-3.0-flash-VL | 蚂蚁百灵的新多模态 Agent / Visual Coding 模型</strong></h2>",
        "<p><strong>发布时间：2026 年 9 月 4 日</strong><br><strong>发布方：Ant Group / inclusionAI（蚂蚁百灵）</strong></p>",
        "<p>官方文档确认，<code>Ling-3.0-flash-VL</code> 可以同时处理<strong>文本、图片和视频</strong>。</p>",
      ].join(""),
      validateRichTextDocument,
    );

    expect(result.accepted).toBe(true);
    expect(result.disposition).toBe("downgraded");
    if (result.accepted) {
      expect(extractBodyText(result.document)).toContain("Ling-3.0-flash-VL 可以同时处理文本、图片和视频");
      expect(result.events.map((event) => event.code)).toContain("INLINE_CODE_DOWNGRADED");
    }
  });

  it("flattens markup inside inline code while keeping security preflight fail-closed", () => {
    const flattened = sanitizePastedHtmlForPersistence(
      [
        "<p><code><strong>plain</strong> ",
        "<a href=\"https://example.com\">linked text</a>",
        "<span data-node=\"imageRef\" data-asset-id=\"aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee\"></span>",
        "</code></p>",
      ].join(""),
      validateRichTextDocument,
    );
    expect(flattened.accepted).toBe(true);
    if (flattened.accepted) {
      expect(flattened.document).toEqual({
        type: "doc",
        content: [{ type: "paragraph", content: [{ type: "text", text: "plain linked text" }] }],
      });
    }

    for (const html of [
      "<p><code><script>alert('x')</script>blocked</code></p>",
      "<p><code><span onclick=\"alert('x')\">blocked</span></code></p>",
      "<p><code><a href=\"javascript:alert('x')\">blocked</a></code></p>",
      "<p><code><img src=\"https://tracker.invalid/pixel.png\">blocked</code></p>",
    ]) {
      expect(sanitizePastedHtmlForPersistence(html, validateRichTextDocument).accepted, html).toBe(false);
    }

    const strictCodeBlock = sanitizePastedHtmlForPersistence(
      "<pre><code><strong>blocked nested formatting</strong></code></pre>",
      validateRichTextDocument,
    );
    expect(strictCodeBlock.accepted).toBe(false);
    expect(strictCodeBlock.events.map((event) => event.code)).toContain("UNSUPPORTED_CODE_BLOCK_CONTENT");
  });

  it("keeps the complete historical sanitizer fixture set in product regression", () => {
    for (const candidate of sanitizerCases) {
      const result = sanitizePastedHtmlForPersistence(candidate.html, validateRichTextDocument);
      expect(result.disposition, candidate.id).toBe(candidate.expected_disposition);
      for (const code of candidate.expected_event_codes) {
        expect(result.events.map((event) => event.code), candidate.id).toContain(code);
      }
      if (result.accepted) {
        expect(validateRichTextDocument(result.document).valid, candidate.id).toBe(true);
        expect(extractBodyText(result.document), candidate.id).toBe(candidate.expected_body_text);
      } else {
        expect(result.document, candidate.id).toBeNull();
      }
    }
  });

  it("rejects dangerous link schemes and preserves approved http, https and mailto links", () => {
    for (const href of [
      "javascript:alert(1)",
      "data:text/html,boom",
      "file:///blocked-local-file.txt",
      "vbscript:boom",
      "https://",
      "http://?missing-host",
      "mailto:",
    ]) {
      expect(sanitizePastedHtmlForPersistence(`<p><a href="${href}">unsafe</a></p>`, validateRichTextDocument).accepted)
        .toBe(false);
    }
    for (const href of ["http://example.com", "https://example.com/path", "mailto:hello@example.com"]) {
      const result = sanitizePastedHtmlForPersistence(`<p><a href="${href}">safe</a></p>`, validateRichTextDocument);
      expect(result.accepted, href).toBe(true);
    }
  });

  it("rejects oversized HTML before insertion and bounds parsed node count and depth", () => {
    const oversized = sanitizePastedHtmlForPersistence(
      `<p>${"x".repeat(MAX_BODY_JSON_BYTES)}</p>`,
      validateRichTextDocument,
    );
    expect(oversized.accepted).toBe(false);
    expect(oversized.events.map((event) => event.code)).toContain("PASTE_HTML_BYTES_EXCEEDED");

    const tooManyNodes = sanitizePastedHtmlForPersistence(
      `<p>${"<br>".repeat(MAX_BODY_NODES)}</p>`,
      validateRichTextDocument,
    );
    expect(tooManyNodes.accepted).toBe(false);
    expect(tooManyNodes.events.map((event) => event.code)).toContain("PASTE_HTML_NODE_LIMIT_EXCEEDED");

    const deeplyNested = `${"<div>".repeat(33)}safe text${"</div>".repeat(33)}`;
    const tooDeep = sanitizePastedHtmlForPersistence(deeplyNested, validateRichTextDocument);
    expect(tooDeep.accepted).toBe(false);
    expect(tooDeep.events.map((event) => event.code)).toContain("PASTE_HTML_DEPTH_LIMIT_EXCEEDED");
  });
});
