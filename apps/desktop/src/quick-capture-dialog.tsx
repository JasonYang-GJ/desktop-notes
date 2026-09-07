import { EditorContent, useEditor } from "@tiptap/react";
import { useEffect, useMemo, useRef, useState } from "react";

import { richTextEditorExtensions } from "./basic-editor";
import {
  discardStagedImage,
  forgetImageContext,
  imageDisplayUrl,
  importCapturedPng,
} from "./images";
import { createNoteFromDraft, type Note, type TiptapDocument } from "./notes";
import { assignTag, listTags, type Tag } from "./organization";
import {
  appendImageToDocument,
  capturedTextDocument,
  finishQuickCapture,
  localSuggestedTitle,
  quickCaptureNotice,
  type QuickCaptureSession,
} from "./quick-capture";
import { RichTextToolbar } from "./rich-text-toolbar";

export function QuickCaptureDialog({
  session,
  defaultDate,
  onSaved,
  onCancelled,
}: {
  session: QuickCaptureSession;
  defaultDate: string;
  onSaved: (note: Note, warning?: string) => void;
  onCancelled: () => void;
}) {
  const capturedText = session.text ?? "";
  const preparedText = useMemo(() => capturedTextDocument(capturedText), [capturedText]);
  const [title, setTitle] = useState(() => localSuggestedTitle(capturedText));
  const [noteDate, setNoteDate] = useState(defaultDate);
  const [tags, setTags] = useState<Tag[]>([]);
  const [selectedTagIds, setSelectedTagIds] = useState<ReadonlySet<string>>(new Set());
  const [busy, setBusy] = useState<"saving" | "cancelling" | "importing">(
    session.dataBase64 ? "importing" : "saving",
  );
  const [working, setWorking] = useState(session.dataBase64 !== null);
  const [imageReady, setImageReady] = useState(session.dataBase64 === null);
  const [error, setError] = useState<string>();
  const [feedback, setFeedback] = useState<string>();
  const importPromiseRef = useRef<Promise<boolean>>(Promise.resolve(session.dataBase64 === null));
  const imageStagedRef = useRef(false);
  const settlingRef = useRef(false);
  const aliveRef = useRef(true);
  const editorExtensions = useMemo(
    () => richTextEditorExtensions((assetId) => imageDisplayUrl(session.id, assetId)),
    [session.id],
  );
  const editor = useEditor({
    extensions: editorExtensions,
    content: preparedText.document,
    immediatelyRender: false,
    editorProps: {
      attributes: {
        class: "basic-editor quick-capture-editor",
        "aria-label": "快速笔记正文",
      },
    },
  });

  useEffect(() => () => {
    aliveRef.current = false;
    forgetImageContext(session.id);
  }, [session.id]);

  useEffect(() => {
    let active = true;
    void listTags()
      .then((items) => {
        if (active) setTags([...items].sort((left, right) => left.name.localeCompare(right.name)));
      })
      .catch(() => {
        if (active) setError("无法加载标签，但仍可保存这条快速笔记。");
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!session.dataBase64 || !editor) return;
    const imported = importCapturedPng(session.id, session.dataBase64)
      .then((image) => {
        imageStagedRef.current = true;
        if (!aliveRef.current) return false;
        const body = appendImageToDocument(
          editor.getJSON() as TiptapDocument,
          image.assetId,
        );
        editor.commands.setContent(body);
        setImageReady(true);
        if (!settlingRef.current) setWorking(false);
        return true;
      })
      .catch(() => {
        if (aliveRef.current) {
          if (!settlingRef.current) setWorking(false);
          setBusy("importing");
          setImageReady(false);
          setError("无法安全处理捕获的图片。请取消后重新捕获。");
        }
        return false;
      });
    importPromiseRef.current = imported;
  }, [editor, session.dataBase64, session.id]);

  async function save() {
    if (!editor || working) return;
    if (session.dataBase64 && !imageReady) {
      setError("捕获的图片尚未准备好，无法保存。");
      return;
    }
    setWorking(true);
    settlingRef.current = true;
    setBusy("saving");
    setError(undefined);
    try {
      let saved = await createNoteFromDraft(
        noteDate,
        title,
        editor.getJSON() as TiptapDocument,
      );
      let warning: string | undefined;
      try {
        for (const tagId of selectedTagIds) {
          saved = await assignTag(saved.id, tagId, saved.revision);
        }
      } catch {
        warning = "快速笔记已保存，但一个或多个所选标签未能添加。";
      }
      try {
        await finishQuickCapture(session.id);
      } catch {
        warning ??= "快速笔记已保存；捕获会话将在应用重启后完成清理。";
      }
      onSaved(saved, warning);
    } catch {
      setError("无法保存快速笔记。当前内容仍保持打开，你可以重试。");
      setWorking(false);
    }
  }

  async function cancel() {
    if (working && busy !== "importing") return;
    setWorking(true);
    settlingRef.current = true;
    setBusy("cancelling");
    setError(undefined);
    try {
      await importPromiseRef.current;
      if (imageStagedRef.current) await discardStagedImage(session.id);
      await finishQuickCapture(session.id);
      onCancelled();
    } catch {
      setError("无法安全取消快速笔记。没有保存任何内容，请再次尝试取消。");
      settlingRef.current = false;
      setWorking(false);
    }
  }

  const clipboardNotice = quickCaptureNotice(session.contentType);
  const effectiveNotice = preparedText.shortened
    ? "剪贴板文字已缩短到安全长度，请确认后再保存。"
    : clipboardNotice;

  return (
    <div className="quick-capture-backdrop" role="presentation">
      <section
        className="quick-capture-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="quick-capture-heading"
      >
        <header className="quick-capture-heading">
          <div>
            <p className="eyebrow">一次性剪贴板预览</p>
            <h2 id="quick-capture-heading">快速笔记</h2>
          </div>
          <span className="quick-capture-state">未保存</span>
        </header>

        {effectiveNotice && <p className="capture-notice" role="status">{effectiveNotice}</p>}
        {working && busy === "importing" && (
          <p className="capture-notice" role="status">正在安全处理捕获的图片…</p>
        )}

        <label className="quick-field">
          <span>建议标题</span>
          <input
            aria-label="快速笔记标题"
            value={title}
            maxLength={512}
            disabled={working}
            onChange={(event) => setTitle(event.target.value)}
            autoFocus
          />
        </label>
        <label className="quick-field quick-date-field">
          <span>笔记日期</span>
          <input
            aria-label="快速笔记日期"
            type="date"
            value={noteDate}
            disabled={working}
            onChange={(event) => setNoteDate(event.target.value)}
          />
        </label>

        {editor && (
          <RichTextToolbar editor={editor} disabled={working} onFeedback={setFeedback} />
        )}
        {feedback && <p className="editor-feedback" role="status">{feedback}</p>}
        <div className="quick-capture-body">
          <EditorContent editor={editor} />
        </div>

        <fieldset className="quick-tags" disabled={working}>
          <legend>标签</legend>
          {tags.length === 0 ? <span>没有可用标签</span> : tags.map((tag) => (
            <label key={tag.id}>
              <input
                type="checkbox"
                checked={selectedTagIds.has(tag.id)}
                onChange={(event) => setSelectedTagIds((current) => {
                  const next = new Set(current);
                  if (event.target.checked) next.add(tag.id);
                  else next.delete(tag.id);
                  return next;
                })}
              />
              {tag.name}
            </label>
          ))}
        </fieldset>

        {error && <p className="note-error" role="alert">{error}</p>}
        <footer className="quick-capture-actions">
          <button type="button" className="quick-cancel" disabled={working && busy !== "importing"} onClick={() => void cancel()}>
            {working && busy === "cancelling" ? "正在取消…" : "取消"}
          </button>
          <button type="button" className="quick-save" disabled={working || !editor} onClick={() => void save()}>
            {working && busy === "saving" ? "正在保存…" : "保存快速笔记"}
          </button>
        </footer>
      </section>
    </div>
  );
}
