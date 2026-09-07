import { Editor } from "@tiptap/core";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { RICH_TEXT_EDITOR_EXTENSIONS } from "./basic-editor";
import { RichTextToolbar } from "./rich-text-toolbar";

const paragraph = (text: string) => ({
  type: "doc",
  content: [{ type: "paragraph", content: [{ type: "text", text }] }],
});

describe("B04 rich-text toolbar", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("applies every approved D08 control through Tiptap transactions", () => {
    const editor = new Editor({
      element: document.createElement("div"),
      extensions: RICH_TEXT_EDITOR_EXTENSIONS,
      content: paragraph("Format me"),
    });
    render(<RichTextToolbar editor={editor} disabled={false} onFeedback={() => undefined} />);

    editor.commands.selectAll();
    fireEvent.click(screen.getByRole("button", { name: "粗体" }));
    expect(editor.getJSON().content[0]?.content?.[0]?.marks).toEqual([{ type: "bold" }]);

    fireEvent.click(screen.getByRole("button", { name: "二级标题" }));
    expect(editor.getJSON().content[0]?.type).toBe("heading");
    fireEvent.click(screen.getByRole("button", { name: "二级标题" }));
    expect(editor.getJSON().content[0]?.type).toBe("paragraph");

    for (const [label, type] of [
      ["项目符号列表", "bulletList"],
      ["编号列表", "orderedList"],
      ["任务清单", "taskList"],
    ] as const) {
      editor.commands.setContent(paragraph(label));
      editor.commands.selectAll();
      fireEvent.click(screen.getByRole("button", { name: label }));
      expect(editor.getJSON().content[0]?.type).toBe(type);
    }

    editor.commands.setContent(paragraph("const safe = true;"));
    editor.commands.selectAll();
    fireEvent.click(screen.getByRole("button", { name: "代码块" }));
    expect(editor.getJSON().content[0]?.type).toBe("codeBlock");
    editor.destroy();
  });

  it("adds only approved links and reports an unsafe URL without changing content", () => {
    const editor = new Editor({
      element: document.createElement("div"),
      extensions: RICH_TEXT_EDITOR_EXTENSIONS,
      content: paragraph("Link me"),
    });
    const feedback = vi.fn();
    render(<RichTextToolbar editor={editor} disabled={false} onFeedback={feedback} />);
    editor.commands.selectAll();

    vi.spyOn(window, "prompt").mockReturnValueOnce("javascript:alert(1)");
    fireEvent.click(screen.getByRole("button", { name: "链接" }));
    expect(feedback).toHaveBeenCalledWith("仅允许 http、https 或 mailto 链接。");
    expect(editor.getJSON().content[0]?.content?.[0]?.marks).toBeUndefined();

    vi.mocked(window.prompt).mockReturnValueOnce("https://example.com/docs");
    fireEvent.click(screen.getByRole("button", { name: "链接" }));
    expect(editor.getJSON().content[0]?.content?.[0]?.marks).toEqual([
      { type: "link", attrs: { href: "https://example.com/docs" } },
    ]);
    editor.destroy();
  });
});
