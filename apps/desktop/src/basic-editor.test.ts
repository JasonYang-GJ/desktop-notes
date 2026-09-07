import { Editor } from "@tiptap/core";
import { describe, expect, it } from "vitest";

import { RICH_TEXT_EDITOR_EXTENSIONS } from "./basic-editor";
import { EMPTY_NOTE_BODY } from "./notes";
import { validateRichTextDocument } from "./rich-text/contract";

describe("B04 Tiptap editor", () => {
  it("exposes only the approved rich-text schema", () => {
    const editor = new Editor({
      element: document.createElement("div"),
      extensions: RICH_TEXT_EDITOR_EXTENSIONS,
      content: EMPTY_NOTE_BODY,
    });
    expect(Object.keys(editor.schema.nodes).sort()).toEqual([
      "bulletList", "codeBlock", "doc", "heading", "imageRef", "listItem",
      "noteLink", "orderedList", "paragraph", "taskItem", "taskList", "text",
    ]);
    expect(Object.keys(editor.schema.marks).sort()).toEqual(["bold", "link"]);
    editor.destroy();
  });

  it("supports current-session undo for text, formatting, lists and pasted content", () => {
    const editor = new Editor({
      element: document.createElement("div"),
      extensions: RICH_TEXT_EDITOR_EXTENSIONS,
      content: EMPTY_NOTE_BODY,
    });

    editor.commands.insertContent("B04 body");
    editor.commands.selectAll();
    expect(editor.commands.toggleMark("bold")).toBe(true);
    expect(editor.isActive("bold")).toBe(true);
    expect(editor.commands.undo()).toBe(true);
    expect(editor.isActive("bold")).toBe(false);

    editor.commands.selectAll();
    expect(editor.commands.toggleList("bulletList", "listItem")).toBe(true);
    expect(editor.getJSON().content[0]?.type).toBe("bulletList");
    expect(editor.commands.undo()).toBe(true);
    expect(editor.getJSON().content[0]?.type).toBe("paragraph");

    editor.commands.setTextSelection(editor.state.doc.content.size - 1);
    editor.commands.insertContent({ type: "paragraph", content: [{ type: "text", text: "Pasted safely" }] });
    expect(editor.getText()).toContain("Pasted safely");
    expect(editor.commands.undo()).toBe(true);
    expect(editor.getText()).not.toContain("Pasted safely");

    expect(validateRichTextDocument(editor.getJSON()).valid).toBe(true);
    expect(editor.commands.undo()).toBe(true);
    expect(editor.getText()).toBe("");
    editor.destroy();
  });

  it("toggles a checklist item through its visible checkbox and keeps the change undoable", () => {
    const element = document.createElement("div");
    document.body.append(element);
    const editor = new Editor({
      element,
      extensions: RICH_TEXT_EDITOR_EXTENSIONS,
      content: {
        type: "doc",
        content: [{
          type: "taskList",
          content: [{
            type: "taskItem",
            attrs: { checked: false },
            content: [{ type: "paragraph", content: [{ type: "text", text: "A task" }] }],
          }],
        }],
      },
    });
    const checkbox = element.querySelector<HTMLInputElement>('input[type="checkbox"]');
    expect(checkbox).not.toBeNull();
    checkbox?.click();
    const taskAttrs = () => (editor.getJSON() as {
      content: Array<{ content?: Array<{ attrs?: unknown }> }>;
    }).content[0]?.content?.[0]?.attrs;
    expect(taskAttrs()).toEqual({ checked: true });
    expect(editor.commands.undo()).toBe(true);
    expect(taskAttrs()).toEqual({ checked: false });
    editor.destroy();
    element.remove();
  });

  it("uses Enter to create the next normal-list or checklist item", () => {
    for (const [listType, itemType] of [["bulletList", "listItem"], ["taskList", "taskItem"]] as const) {
      const editor = new Editor({
        element: document.createElement("div"),
        extensions: RICH_TEXT_EDITOR_EXTENSIONS,
        content: {
          type: "doc",
          content: [{
            type: listType,
            content: [{
              type: itemType,
              ...(itemType === "taskItem" ? { attrs: { checked: false } } : {}),
              content: [{ type: "paragraph", content: [{ type: "text", text: "First" }] }],
            }],
          }],
        },
      });
      editor.commands.setTextSelection(editor.state.doc.content.size - 2);
      expect(editor.commands.keyboardShortcut("Enter"), listType).toBe(true);
      expect(editor.getJSON().content[0]?.content).toHaveLength(2);
      editor.destroy();
    }
  });
});
