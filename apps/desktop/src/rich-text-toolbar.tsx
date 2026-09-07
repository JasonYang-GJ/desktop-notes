import type { Editor } from "@tiptap/core";
import { useEffect, useState } from "react";

import { isApprovedExternalLink } from "./rich-text/link-policy";

export { isApprovedExternalLink } from "./rich-text/link-policy";

export function RichTextToolbar({
  editor,
  disabled,
  onFeedback,
}: {
  editor: Editor;
  disabled: boolean;
  onFeedback: (message?: string) => void;
}) {
  const [, renderTransaction] = useState(0);

  useEffect(() => {
    const refresh = () => renderTransaction((value) => value + 1);
    editor.on("transaction", refresh);
    editor.on("selectionUpdate", refresh);
    return () => {
      editor.off("transaction", refresh);
      editor.off("selectionUpdate", refresh);
    };
  }, [editor]);

  const run = (command: () => boolean) => {
    onFeedback(undefined);
    command();
    editor.commands.focus();
  };

  const editLink = () => {
    const current = editor.getAttributes("link").href;
    const value = window.prompt("请输入 http、https 或 mailto 链接", typeof current === "string" ? current : "https://");
    if (value === null) return;
    const href = value.trim();
    if (href === "") {
      run(() => editor.commands.unsetMark("link"));
      return;
    }
    if (!isApprovedExternalLink(href)) {
      onFeedback("仅允许 http、https 或 mailto 链接。");
      return;
    }
    run(() => editor.commands.setMark("link", { href }));
  };

  const button = (
    label: string,
    shortLabel: string,
    active: boolean,
    command: () => boolean,
    className?: string,
  ) => (
    <button
      className={`format-button${active ? " active" : ""}${className ? ` ${className}` : ""}`}
      type="button"
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => run(command)}
    >
      {shortLabel}
    </button>
  );

  return (
    <div className="format-toolbar" role="toolbar" aria-label="文字格式">
      {button("粗体", "B", editor.isActive("bold"), () => editor.commands.toggleMark("bold"), "format-bold")}
      {button(
        "二级标题",
        "标题",
        editor.isActive("heading", { level: 2 }),
        () => editor.commands.toggleNode("heading", "paragraph", { level: 2 }),
      )}
      <span className="format-divider" aria-hidden="true" />
      {button(
        "项目符号列表",
        "• 列表",
        editor.isActive("bulletList"),
        () => editor.commands.toggleList("bulletList", "listItem"),
      )}
      {button(
        "编号列表",
        "1. 列表",
        editor.isActive("orderedList"),
        () => editor.commands.toggleList("orderedList", "listItem"),
      )}
      {button(
        "任务清单",
        "☐ 清单",
        editor.isActive("taskList"),
        () => editor.commands.toggleList("taskList", "taskItem"),
      )}
      <span className="format-divider" aria-hidden="true" />
      {button(
        "代码块",
        "</>",
        editor.isActive("codeBlock"),
        () => editor.commands.toggleNode("codeBlock", "paragraph"),
        "format-code",
      )}
      <button
        className={`format-button${editor.isActive("link") ? " active" : ""}`}
        type="button"
        aria-label="链接"
        aria-pressed={editor.isActive("link")}
        disabled={disabled}
        onMouseDown={(event) => event.preventDefault()}
        onClick={editLink}
      >
        链接
      </button>
    </div>
  );
}
