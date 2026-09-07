import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { App } from "./App";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const invokeMock = vi.mocked(invoke);
const note = {
  protocolVersion: 1,
  id: "b1000000-0000-4000-8000-000000000001",
  noteDate: "2026-09-07",
  title: "Synthetic B10 title",
  bodyJson: { type: "doc", content: [{ type: "paragraph", content: [{ type: "text", text: "Synthetic B10 body" }] }] },
  bodyFormat: "tiptap-json",
  bodySchemaVersion: 1,
  bodyText: "Synthetic B10 body",
  contentHash: "0".repeat(64),
  isPinned: false,
  createdAtMs: 1_789_000_000_000,
  updatedAtMs: 1_789_000_000_000,
  revision: 1,
};

describe("B10 three-state desktop shell", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command) => {
      if (command === "get_foundation_status") return {
        ok: true,
        status: {
          protocolVersion: 1,
          ready: true,
          view: "today_normal_empty",
          databaseState: "reopened",
          schemaVersion: 4,
          encryption: "sqlcipher",
        },
      };
      if (command === "list_notes_for_date") return { ok: true, notes: [note] };
      if (command === "list_note_counts_for_month") return {
        ok: true,
        counts: [{ noteDate: note.noteDate, count: 1 }],
      };
      if (command === "get_note") return { ok: true, note };
      if (command === "list_tags" || command === "list_tags_for_note") return { ok: true, tags: [] };
      throw new Error(`Unexpected command ${command}`);
    });
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("keeps navigation and a dirty editor mounted across Collapsed, Normal, and Expanded", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /Synthetic B10 title/ }));
    const title = await screen.findByRole("textbox", { name: "笔记标题" });
    fireEvent.change(title, { target: { value: "Unsaved state survives" } });
    expect(screen.getByText("未保存")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "收起视图" }));
    const summary = document.querySelector(".collapsed-summary");
    expect(summary).not.toBeNull();
    expect(within(summary as HTMLElement).getByText("1 条笔记")).toBeVisible();
    expect(within(summary as HTMLElement).queryByText(/Synthetic B10 title|Synthetic B10 body/)).toBeNull();
    expect(title).not.toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "打开标准视图" }));
    expect(title).toBeVisible();
    expect(title).toHaveValue("Unsaved state survives");
    expect(screen.getByText("未保存")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "展开视图" }));
    expect(document.querySelector(".calendar-workspace")).toHaveClass("is-expanded");
    expect(title).toHaveValue("Unsaved state survives");

    fireEvent.change(screen.getByRole("combobox"), { target: { value: "dark" } });
    await waitFor(() => expect(document.querySelector(".app-frame")).toHaveAttribute("data-theme", "dark"));
  });
});
