import { useState } from "react";

import type { Tag } from "./organization";

interface TagControlsProps {
  tags: Tag[];
  assignedIds: ReadonlySet<string>;
  disabled: boolean;
  onToggle: (tag: Tag, assigned: boolean) => Promise<boolean>;
  onCreate: (name: string) => Promise<boolean>;
  onRename: (tag: Tag, name: string) => Promise<boolean>;
  onDelete: (tag: Tag) => Promise<boolean>;
}

export function TagControls({
  tags,
  assignedIds,
  disabled,
  onToggle,
  onCreate,
  onRename,
  onDelete,
}: TagControlsProps) {
  const [managing, setManaging] = useState(false);
  const [newName, setNewName] = useState("");
  const [renaming, setRenaming] = useState<Tag>();
  const [renameValue, setRenameValue] = useState("");
  const [deleting, setDeleting] = useState<Tag>();
  const actionableTags = [...tags].sort((left, right) => {
    const leftAssigned = assignedIds.has(left.id);
    const rightAssigned = assignedIds.has(right.id);

    if (leftAssigned !== rightAssigned) return leftAssigned ? -1 : 1;
    if (left.isSeedDefault !== right.isSeedDefault) return left.isSeedDefault ? -1 : 1;
    if (!left.isSeedDefault && left.updatedAtMs !== right.updatedAtMs) {
      return right.updatedAtMs - left.updatedAtMs;
    }
    return left.name.localeCompare(right.name);
  });

  return (
    <section className="tag-controls" aria-label="笔记标签">
      <div className="tag-heading">
        <span>标签</span>
        <button
          type="button"
          disabled={disabled}
          onClick={() => {
            setManaging((current) => !current);
            setRenaming(undefined);
            setDeleting(undefined);
          }}
        >
          {managing ? "关闭标签管理" : "管理标签"}
        </button>
      </div>

      <div className="tag-chips" aria-label="可用标签">
        {actionableTags.map((tag) => {
          const assigned = assignedIds.has(tag.id);
          return (
            <button
              className={assigned ? "tag-chip assigned" : "tag-chip"}
              type="button"
              key={tag.id}
              aria-label={`${assigned ? "移除" : "添加"}标签 ${tag.name}`}
              aria-pressed={assigned}
              disabled={disabled}
              onClick={() => void onToggle(tag, assigned)}
            >
              <span aria-hidden="true">{assigned ? "#" : "+"}</span>
              {tag.name}
            </button>
          );
        })}
        {tags.length === 0 && <span className="tag-empty">还没有标签</span>}
      </div>

      {managing && (
        <div className="tag-manager">
          <form
            className="tag-create"
            autoComplete="off"
            onSubmit={(event) => {
              event.preventDefault();
              void onCreate(newName).then((saved) => {
                if (saved) setNewName("");
              });
            }}
          >
            <input
              aria-label="新标签名称"
              autoComplete="off"
              maxLength={256}
              placeholder="新标签"
              value={newName}
              disabled={disabled}
              onChange={(event) => setNewName(event.target.value)}
            />
            <button type="submit" disabled={disabled || newName.trim().length === 0}>添加标签</button>
          </form>

          <div className="tag-manager-list">
            {tags.map((tag) => (
              <div className="tag-manager-row" key={tag.id}>
                {renaming?.id === tag.id ? (
                  <>
                    <input
                      aria-label={`重命名标签 ${tag.name}`}
                      autoComplete="off"
                      maxLength={256}
                      value={renameValue}
                      disabled={disabled}
                      onChange={(event) => setRenameValue(event.target.value)}
                    />
                    <button
                      type="button"
                      aria-label="保存标签名称"
                      disabled={disabled || renameValue.trim().length === 0}
                      onClick={() => void onRename(tag, renameValue).then((saved) => {
                        if (saved) setRenaming(undefined);
                      })}
                    >
                      保存
                    </button>
                    <button
                      type="button"
                      aria-label="取消重命名标签"
                      disabled={disabled}
                      onClick={() => setRenaming(undefined)}
                    >
                      取消
                    </button>
                  </>
                ) : deleting?.id === tag.id ? (
                  <>
                    <span>{tag.name}</span>
                    <button
                      className="danger"
                      type="button"
                      aria-label={`确认删除标签 ${tag.name}`}
                      disabled={disabled}
                      onClick={() => void onDelete(tag).then((saved) => {
                        if (saved) setDeleting(undefined);
                      })}
                    >
                      确认
                    </button>
                    <button
                      type="button"
                      aria-label={`取消删除标签 ${tag.name}`}
                      disabled={disabled}
                      onClick={() => setDeleting(undefined)}
                    >
                      取消
                    </button>
                  </>
                ) : (
                  <>
                    <span>{tag.name}</span>
                    <button
                      type="button"
                      aria-label={`重命名标签 ${tag.name}`}
                      disabled={disabled}
                      onClick={() => {
                        setDeleting(undefined);
                        setRenaming(tag);
                        setRenameValue(tag.name);
                      }}
                    >
                      重命名
                    </button>
                    <button
                      type="button"
                      aria-label={`删除标签 ${tag.name}`}
                      disabled={disabled}
                      onClick={() => {
                        setRenaming(undefined);
                        setDeleting(tag);
                      }}
                    >
                      删除
                    </button>
                  </>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  );
}
