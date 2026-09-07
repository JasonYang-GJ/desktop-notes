import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CalendarWorkspace } from "./App";
import type { FoundationStatus } from "./foundation";
import type { Note } from "./notes";
import type { Tag } from "./organization";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const status: FoundationStatus = {
  protocolVersion: 1,
  ready: true,
  view: "today_normal_empty",
  databaseState: "reopened",
  schemaVersion: 3,
  encryption: "sqlcipher",
};

const note = (id: string, title: string, noteDate: string, revision = 1, isPinned = false): Note => ({
  protocolVersion: 1,
  id,
  noteDate,
  title,
  bodyJson: { type: "doc", content: [{ type: "paragraph" }] },
  bodyFormat: "tiptap-json",
  bodySchemaVersion: 1,
  bodyText: "",
  contentHash: "0".repeat(64),
  isPinned,
  createdAtMs: 100,
  updatedAtMs: 100 + revision,
  revision,
});

const tag = (id: string, name: string, isSeedDefault = false): Tag => ({
  protocolVersion: 1,
  id,
  name,
  isSeedDefault,
  createdAtMs: 100,
  updatedAtMs: 100,
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => { resolve = next; });
  return { promise, resolve };
}

describe("B05 Tags, Pin, and Recent UI", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.spyOn(globalThis.crypto, "randomUUID")
      .mockReturnValue("0ed50682-84b2-4022-9869-c2a426d90586");
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("opens Recent without changing Note identity and preserves backend ordering", async () => {
    const a = note("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "Note A", "2026-09-05", 2);
    const b = note("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "Note B", "2026-09-06", 4, true);
    const c = note("cccccccc-cccc-4ccc-8ccc-cccccccccccc", "Note C", "2026-09-07", 3);
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "list_recent_notes") return { ok: true, notes: [b, c, a] };
      if (command === "get_note") {
        const id = (args as { request: { noteId: string } }).request.noteId;
        return { ok: true, note: [a, b, c].find((item) => item.id === id) };
      }
      if (command === "list_tags" || command === "list_tags_for_note") {
        return { ok: true, tags: [] };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(screen.getByRole("button", { name: "最近" }));

    const recent = await screen.findByRole("navigation", { name: "最近笔记" });
    const rows = await within(recent).findAllByRole("button");
    expect(rows.map((row) => row.textContent)).toEqual([
      expect.stringContaining("Note B"),
      expect.stringContaining("Note C"),
      expect.stringContaining("Note A"),
    ]);
    expect(rows[0]).toHaveTextContent("已置顶");
    fireEvent.click(rows[0]);
    expect(await screen.findByRole("textbox", { name: "笔记标题" })).toHaveValue("Note B");
    expect(invokeMock).toHaveBeenCalledWith("get_note", {
      request: { protocolVersion: 1, noteId: b.id },
    });
  });

  it("flushes an in-flight body edit and serializes rapid Pin intents to the final state", async () => {
    const current = note("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "Draft", "2026-09-06");
    const contentSave = deferred<unknown>();
    const pinCalls: Array<{ request: { isPinned: boolean; baseRevision: number; clientChangeId: string } }> = [];
    const pinResponses = [deferred<unknown>(), deferred<unknown>(), deferred<unknown>()];
    invokeMock.mockImplementation((command, args) => {
      if (command === "list_notes_for_date") return Promise.resolve({ ok: true, notes: [current] });
      if (command === "list_note_counts_for_month") return Promise.resolve({ ok: true, counts: [] });
      if (command === "get_note") return Promise.resolve({ ok: true, note: current });
      if (command === "list_tags" || command === "list_tags_for_note") {
        return Promise.resolve({ ok: true, tags: [] });
      }
      if (command === "update_note_content") return contentSave.promise;
      if (command === "set_note_pinned") {
        pinCalls.push(args as typeof pinCalls[number]);
        return pinResponses[pinCalls.length - 1].promise;
      }
      return Promise.reject(new Error(`Unexpected command ${command}`));
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Draft/ }));
    fireEvent.change(await screen.findByRole("textbox", { name: "笔记标题" }), {
      target: { value: "Draft safely saved" },
    });
    fireEvent.click(screen.getByRole("button", { name: "置顶笔记" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith(
      "update_note_content",
      expect.objectContaining({ request: expect.objectContaining({ title: "Draft safely saved" }) }),
    ));
    expect(pinCalls).toHaveLength(0);
    contentSave.resolve({
      ok: true,
      note: { ...current, title: "Draft safely saved", revision: 2, updatedAtMs: 200 },
      clientChangeId: "0ed50682-84b2-4022-9869-c2a426d90586",
    });
    await waitFor(() => expect(pinCalls).toHaveLength(1));
    expect(pinCalls[0].request).toMatchObject({ isPinned: true, baseRevision: 2 });

    fireEvent.click(screen.getByRole("button", { name: "取消置顶笔记" }));
    pinResponses[0].resolve({
      ok: true,
      note: { ...current, title: "Draft safely saved", isPinned: true, revision: 3 },
      clientChangeId: "0ed50682-84b2-4022-9869-c2a426d90586",
    });
    await waitFor(() => expect(pinCalls).toHaveLength(2));
    expect(pinCalls[1].request).toMatchObject({ isPinned: false, baseRevision: 3 });

    fireEvent.click(screen.getByRole("button", { name: "置顶笔记" }));
    pinResponses[1].resolve({
      ok: true,
      note: { ...current, title: "Draft safely saved", isPinned: false, revision: 4 },
      clientChangeId: "0ed50682-84b2-4022-9869-c2a426d90586",
    });
    await waitFor(() => expect(pinCalls).toHaveLength(3));
    expect(pinCalls[2].request).toMatchObject({ isPinned: true, baseRevision: 4 });
    pinResponses[2].resolve({
      ok: true,
      note: { ...current, title: "Draft safely saved", isPinned: true, revision: 5 },
      clientChangeId: "0ed50682-84b2-4022-9869-c2a426d90586",
    });

    await waitFor(() => expect(screen.getByRole("button", { name: "取消置顶笔记" })).toHaveTextContent("已置顶"));
    expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Draft safely saved");
    await waitFor(
      () => expect(screen.getAllByText("修订版本 5")).toHaveLength(2),
      { timeout: 3_000 },
    );
  });

  it("keeps failed Tag assignment unapplied and supports create, rename, delete, and cancel", async () => {
    const current = note("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "Organized", "2026-09-06");
    const project = tag("11111111-1111-4111-8111-111111111111", "Project", true);
    const ai = tag("22222222-2222-4222-8222-222222222222", "AI", true);
    const custom = { ...tag("33333333-3333-4333-8333-333333333333", "研究"), updatedAtMs: 200 };
    const olderCustom = { ...tag("44444444-4444-4444-8444-444444444444", "Archive"), updatedAtMs: 50 };
    let catalog = [project, ai, olderCustom];
    invokeMock.mockImplementation(async (command, args) => {
      if (command === "list_notes_for_date") return { ok: true, notes: [current] };
      if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
      if (command === "get_note") return { ok: true, note: current };
      if (command === "list_tags") return { ok: true, tags: catalog };
      if (command === "list_tags_for_note") return { ok: true, tags: [project] };
      if (command === "assign_tag") {
        return { ok: false, error: { code: "DISK_FULL", message: "Synthetic", recoverable: true } };
      }
      if (command === "create_tag") {
        catalog = [...catalog, custom];
        return { ok: true, tag: custom };
      }
      if (command === "rename_tag") {
        const renamed = { ...project, name: "项目" };
        catalog = catalog.map((item) => item.id === project.id ? renamed : item);
        return { ok: true, tag: renamed };
      }
      if (command === "delete_tag") {
        const tagId = (args as { request: { tagId: string } }).request.tagId;
        catalog = catalog.filter((item) => item.id !== tagId);
        return { ok: true, deletedTagId: tagId };
      }
      throw new Error(`Unexpected command ${command}`);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    fireEvent.click(await screen.findByRole("button", { name: /Organized/ }));
    expect(await screen.findByRole("textbox", { name: "笔记标题" })).toHaveAttribute("autocomplete", "off");
    expect(screen.getByLabelText("笔记日期")).toHaveAttribute("autocomplete", "off");
    const aiChip = await screen.findByRole("button", { name: "添加标签 AI" });
    fireEvent.click(aiChip);
    expect(await screen.findByText("无法保存标签更改。")).toBeVisible();
    expect(aiChip).toHaveAttribute("aria-pressed", "false");

    fireEvent.click(screen.getByRole("button", { name: "管理标签" }));
    expect(screen.getByRole("textbox", { name: "新标签名称" })).toHaveAttribute("autocomplete", "off");
    fireEvent.change(screen.getByRole("textbox", { name: "新标签名称" }), {
      target: { value: "研究" },
    });
    fireEvent.click(screen.getByRole("button", { name: "添加标签" }));
    expect(await screen.findByRole("button", { name: "添加标签 研究" })).toBeVisible();
    expect(within(screen.getByLabelText("可用标签")).getAllByRole("button").map((item) => item.getAttribute("aria-label"))).toEqual([
      "移除标签 Project",
      "添加标签 AI",
      "添加标签 研究",
      "添加标签 Archive",
    ]);

    fireEvent.click(screen.getByRole("button", { name: "重命名标签 Project" }));
    expect(screen.getByRole("textbox", { name: "重命名标签 Project" })).toHaveAttribute("autocomplete", "off");
    fireEvent.change(screen.getByRole("textbox", { name: "重命名标签 Project" }), {
      target: { value: "项目" },
    });
    fireEvent.click(screen.getByRole("button", { name: "保存标签名称" }));
    expect(await screen.findByRole("button", { name: "移除标签 项目" })).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "重命名标签 项目" }));
    fireEvent.click(screen.getByRole("button", { name: "取消重命名标签" }));
    expect(screen.queryByRole("textbox", { name: "重命名标签 项目" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "删除标签 项目" }));
    fireEvent.click(screen.getByRole("button", { name: "确认删除标签 项目" }));
    await waitFor(() => expect(screen.queryByText("项目")).not.toBeInTheDocument());
    expect(screen.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Organized");
  });
});
