import { useRef } from "react";

import {
  addCalendarDays,
  buildMonthGrid,
  formatCalendarHeading,
  monthFromDate,
  shiftMonth,
} from "./calendar";

export const NOTE_DRAG_MIME = "application/x-desktop-notes-note-id";

interface CalendarViewProps {
  month: string;
  selectedDate: string;
  today: string;
  counts: ReadonlyMap<string, number>;
  onMonthChange: (month: string) => void;
  onSelectDate: (date: string) => void;
  onDropNote: (noteId: string, date: string) => void;
}

const WEEKDAYS = ["日", "一", "二", "三", "四", "五", "六"];

export function CalendarView({
  month,
  selectedDate,
  today,
  counts,
  onMonthChange,
  onSelectDate,
  onDropNote,
}: CalendarViewProps) {
  const gridRef = useRef<HTMLDivElement>(null);
  const cells = buildMonthGrid(month, selectedDate, today);

  function selectDate(date: string) {
    const nextMonth = monthFromDate(date);
    if (nextMonth !== month) onMonthChange(nextMonth);
    onSelectDate(date);
  }

  function selectFromKeyboard(date: string) {
    selectDate(date);
    requestAnimationFrame(() => {
      gridRef.current
        ?.querySelector<HTMLButtonElement>(`#calendar-day-${date}`)
        ?.focus();
    });
  }

  return (
    <aside className="calendar-panel" aria-label="日历">
      <div className="calendar-heading">
        <button type="button" aria-label="上个月" onClick={() => onMonthChange(shiftMonth(month, -1))}>
          <span aria-hidden="true">←</span>
        </button>
        <h2>{formatCalendarHeading(month)}</h2>
        <button type="button" aria-label="下个月" onClick={() => onMonthChange(shiftMonth(month, 1))}>
          <span aria-hidden="true">→</span>
        </button>
      </div>

      <button
        className="calendar-today"
        type="button"
        onClick={() => selectDate(today)}
      >
        今天
      </button>

      <div className="calendar-weekdays" aria-hidden="true">
        {WEEKDAYS.map((weekday) => <span key={weekday}>{weekday}</span>)}
      </div>

      <div className="calendar-grid" aria-label={formatCalendarHeading(month)} ref={gridRef}>
        {cells.map((cell) => {
          const count = counts.get(cell.date) ?? 0;
          return (
            <button
              className={[
                "calendar-day",
                !cell.inCurrentMonth && "outside-month",
                cell.isToday && "today",
                cell.isSelected && "selected",
                count > 0 && "has-notes",
              ].filter(Boolean).join(" ")}
              id={`calendar-day-${cell.date}`}
              type="button"
              key={cell.date}
              aria-current={cell.isToday ? "date" : undefined}
              aria-pressed={cell.isSelected}
              aria-label={calendarDayLabel(cell.date, cell.isToday, count)}
              onClick={() => selectDate(cell.date)}
              onKeyDown={(event) => {
                const offsets: Record<string, number> = {
                  ArrowLeft: -1,
                  ArrowRight: 1,
                  ArrowUp: -7,
                  ArrowDown: 7,
                };
                const offset = offsets[event.key];
                if (offset === undefined) return;
                event.preventDefault();
                selectFromKeyboard(addCalendarDays(cell.date, offset));
              }}
              onDragOver={(event) => {
                event.preventDefault();
                event.dataTransfer.dropEffect = "move";
              }}
              onDrop={(event) => {
                event.preventDefault();
                const noteId = event.dataTransfer.getData(NOTE_DRAG_MIME);
                if (noteId) onDropNote(noteId, cell.date);
              }}
            >
              <span className="calendar-day-number">{cell.dayNumber}</span>
              {count > 0 && <span className="calendar-note-count" aria-hidden="true">{count}</span>}
            </button>
          );
        })}
      </div>
    </aside>
  );
}

function calendarDayLabel(date: string, isToday: boolean, count: number): string {
  const [year, month, day] = date.split("-").map(Number);
  const value = new Date(0);
  value.setHours(12, 0, 0, 0);
  value.setFullYear(year, month - 1, day);
  const parts = [
    new Intl.DateTimeFormat("zh-CN", {
      month: "long",
      day: "numeric",
      year: "numeric",
    }).format(value),
  ];
  if (isToday) parts.push("今天");
  if (count > 0) parts.push(`${count} 条笔记`);
  return parts.join(", ");
}
