import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CalendarWorkspace } from "./App";
import type { FoundationStatus } from "./foundation";
import type { Note } from "./notes";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const noteId = "cdd026eb-e8ab-41e0-a6a9-763087d78c4b";
const status: FoundationStatus = {
  protocolVersion: 1,
  ready: true,
  view: "today_normal_empty",
  databaseState: "reopened",
  schemaVersion: 4,
  encryption: "sqlcipher",
};
const openedNote: Note = {
  protocolVersion: 1,
  id: noteId,
  noteDate: "2026-09-05",
  title: "Project Search <script>alert(1)</script>",
  bodyJson: { type: "doc", content: [{ type: "paragraph" }] },
  bodyFormat: "tiptap-json",
  bodySchemaVersion: 1,
  bodyText: "Search body",
  contentHash: "0".repeat(64),
  isPinned: false,
  createdAtMs: 100,
  updatedAtMs: 200,
  revision: 2,
};

function defaultCommand(command: string) {
  if (command === "list_notes_for_date") return { ok: true, notes: [] };
  if (command === "list_note_counts_for_month") return { ok: true, counts: [] };
  if (command === "list_tags" || command === "list_tags_for_note") return { ok: true, tags: [] };
  if (command === "get_note") return { ok: true, note: openedNote };
  throw new Error(`Unexpected command ${command}`);
}

describe("B06 Local Search UI", () => {
  beforeEach(() => invokeMock.mockReset());

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders title, body, and tag matches as safe React text and opens the stable Note ID", async () => {
    invokeMock.mockImplementation(async (command) => {
      if (command === "search_notes") {
        return {
          ok: true,
          hits: [{
            id: noteId,
            noteDate: "2026-09-05",
            title: "Project Search <script>alert(1)</script>",
            snippet: "正文 Search <img src=x onerror=alert(1)>",
            matchingTags: [{
              id: "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
              name: "Search<tag>",
            }],
            updatedAtMs: 200,
            revision: 2,
          }],
        };
      }
      return defaultCommand(command);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    const input = screen.getByRole("textbox", { name: "搜索笔记" });
    expect(input).toHaveAttribute("autocomplete", "off");
    fireEvent.change(input, { target: { value: "Search" } });

    const results = await screen.findByRole("navigation", { name: "搜索结果" });
    const hit = await within(results).findByRole("button");
    expect(hit).toHaveTextContent("Project Search <script>alert(1)</script>");
    expect(hit).toHaveTextContent("正文 Search <img src=x onerror=alert(1)>");
    expect(hit).toHaveTextContent("Search<tag>");
    expect(hit.querySelector("script")).toBeNull();
    expect(hit.querySelector("img")).toBeNull();
    expect(hit.querySelectorAll("mark")).toHaveLength(3);

    fireEvent.click(hit);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_note", {
      request: { protocolVersion: 1, noteId },
    }));
    expect(await screen.findByRole("textbox", { name: "笔记标题" })).toHaveValue(openedNote.title);
  });

  it("shows loading, no-result, and controlled error states", async () => {
    let fail = false;
    invokeMock.mockImplementation(async (command) => {
      if (command === "search_notes") {
        if (fail) throw new Error("raw backend detail");
        return { ok: true, hits: [] };
      }
      return defaultCommand(command);
    });

    render(<CalendarWorkspace status={status} today="2026-09-06" />);
    const input = screen.getByRole("textbox", { name: "搜索笔记" });
    fireEvent.change(input, { target: { value: "none" } });
    expect(await screen.findByText("正在搜索…")).toBeVisible();
    expect(await screen.findByText("没有匹配的笔记。")).toBeVisible();

    fail = true;
    fireEvent.change(input, { target: { value: "failure" } });
    expect(await screen.findByText("无法完成搜索。")).toBeVisible();
    expect(screen.queryByText("raw backend detail")).not.toBeInTheDocument();
  });
});
