import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { CalendarView } from "./calendar-view";

afterEach(cleanup);

describe("B03 month calendar", () => {
  it("shows month navigation, distinct Today/selected dates, and repository counts", () => {
    const onMonthChange = vi.fn();
    render(
      <CalendarView
        month="2026-09"
        selectedDate="2026-09-10"
        today="2026-09-06"
        counts={new Map([["2026-09-06", 2], ["2026-09-10", 1]])}
        onMonthChange={onMonthChange}
        onSelectDate={vi.fn()}
        onDropNote={vi.fn()}
      />,
    );

    expect(screen.getByRole("heading", { name: /2026年9月/i })).toBeVisible();
    expect(screen.getByRole("button", { name: /2026年9月6日.*今天.*2 条笔记/i }))
      .toHaveAttribute("aria-current", "date");
    expect(screen.getByRole("button", { name: /2026年9月10日.*1 条笔记/i }))
      .toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: /2026年8月31日/i }))
      .toHaveClass("outside-month");

    fireEvent.click(screen.getByRole("button", { name: "上个月" }));
    fireEvent.click(screen.getByRole("button", { name: "下个月" }));
    expect(onMonthChange).toHaveBeenNthCalledWith(1, "2026-08");
    expect(onMonthChange).toHaveBeenNthCalledWith(2, "2026-10");
  });

  it("supports arrow-key date navigation and a Note drop target", () => {
    const onSelectDate = vi.fn();
    const onDropNote = vi.fn();
    render(
      <CalendarView
        month="2026-09"
        selectedDate="2026-09-10"
        today="2026-09-06"
        counts={new Map()}
        onMonthChange={vi.fn()}
        onSelectDate={onSelectDate}
        onDropNote={onDropNote}
      />,
    );

    const selected = screen.getByRole("button", { name: /2026年9月10日/i });
    fireEvent.keyDown(selected, { key: "ArrowRight" });
    expect(onSelectDate).toHaveBeenCalledWith("2026-09-11");

    const target = screen.getByRole("button", { name: /2026年9月14日/i });
    fireEvent.drop(target, {
      dataTransfer: {
        getData: (type: string) => type === "application/x-desktop-notes-note-id" ? "note-123" : "",
      },
    });
    expect(onDropNote).toHaveBeenCalledWith("note-123", "2026-09-14");
  });
});
