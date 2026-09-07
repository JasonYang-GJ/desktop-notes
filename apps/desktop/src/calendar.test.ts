import { describe, expect, it } from "vitest";

import {
  addCalendarDays,
  buildMonthGrid,
  monthFromDate,
  shiftMonth,
} from "./calendar";

describe("B03 local calendar model", () => {
  it("navigates month and year boundaries without changing the selected date", () => {
    expect(shiftMonth("2026-01", -1)).toBe("2025-12");
    expect(shiftMonth("2026-12", 1)).toBe("2027-01");
    expect(monthFromDate("2026-09-06")).toBe("2026-09");
  });

  it("uses real calendar arithmetic across leap days and years", () => {
    expect(addCalendarDays("2028-02-28", 1)).toBe("2028-02-29");
    expect(addCalendarDays("2028-02-29", 1)).toBe("2028-03-01");
    expect(addCalendarDays("2026-12-31", 1)).toBe("2027-01-01");
    expect(addCalendarDays("2027-01-01", -1)).toBe("2026-12-31");
  });

  it("builds a six-week grid and keeps Today separate from selected date", () => {
    const cells = buildMonthGrid("2026-09", "2026-09-10", "2026-09-06");

    expect(cells).toHaveLength(42);
    expect(cells[0].date).toBe("2026-08-30");
    expect(cells.at(-1)?.date).toBe("2026-10-10");
    expect(cells.find((cell) => cell.date === "2026-09-06")).toMatchObject({
      isToday: true,
      isSelected: false,
      inCurrentMonth: true,
    });
    expect(cells.find((cell) => cell.date === "2026-09-10")).toMatchObject({
      isToday: false,
      isSelected: true,
      inCurrentMonth: true,
    });
    expect(cells.find((cell) => cell.date === "2026-08-31")?.inCurrentMonth).toBe(false);
  });
});
