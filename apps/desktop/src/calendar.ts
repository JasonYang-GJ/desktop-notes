export interface CalendarCell {
  date: string;
  dayNumber: number;
  inCurrentMonth: boolean;
  isToday: boolean;
  isSelected: boolean;
}

export function monthFromDate(date: string): string {
  return date.slice(0, 7);
}

export function shiftMonth(month: string, offset: number): string {
  const { year, monthIndex } = parseMonth(month);
  const value = localDate(year, monthIndex, 1);
  value.setMonth(value.getMonth() + offset);
  return formatMonth(value);
}

export function addCalendarDays(date: string, offset: number): string {
  const value = parseDate(date);
  value.setDate(value.getDate() + offset);
  return formatDate(value);
}

export function buildMonthGrid(
  month: string,
  selectedDate: string,
  today: string,
): CalendarCell[] {
  const { year, monthIndex } = parseMonth(month);
  const first = localDate(year, monthIndex, 1);
  first.setDate(first.getDate() - first.getDay());

  return Array.from({ length: 42 }, (_, index) => {
    const value = new Date(first);
    value.setDate(first.getDate() + index);
    const date = formatDate(value);
    return {
      date,
      dayNumber: value.getDate(),
      inCurrentMonth: value.getMonth() === monthIndex && value.getFullYear() === year,
      isToday: date === today,
      isSelected: date === selectedDate,
    };
  });
}

export function formatCalendarHeading(month: string): string {
  const { year, monthIndex } = parseMonth(month);
  return new Intl.DateTimeFormat("zh-CN", {
    month: "long",
    year: "numeric",
  }).format(localDate(year, monthIndex, 1));
}

export function formatSelectedDate(date: string): string {
  return new Intl.DateTimeFormat("zh-CN", {
    weekday: "long",
    month: "long",
    day: "numeric",
    year: "numeric",
  }).format(parseDate(date));
}

function parseMonth(month: string): { year: number; monthIndex: number } {
  const match = /^(\d{4})-(\d{2})$/.exec(month);
  if (!match) throw new Error("Invalid calendar month");
  const year = Number(match[1]);
  const monthNumber = Number(match[2]);
  if (year < 1 || year > 9999 || monthNumber < 1 || monthNumber > 12) {
    throw new Error("Invalid calendar month");
  }
  return { year, monthIndex: monthNumber - 1 };
}

function parseDate(date: string): Date {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!match) throw new Error("Invalid calendar date");
  const year = Number(match[1]);
  const monthIndex = Number(match[2]) - 1;
  const day = Number(match[3]);
  const value = localDate(year, monthIndex, day);
  if (
    value.getFullYear() !== year
    || value.getMonth() !== monthIndex
    || value.getDate() !== day
  ) {
    throw new Error("Invalid calendar date");
  }
  return value;
}

function localDate(year: number, monthIndex: number, day: number): Date {
  const value = new Date(0);
  value.setHours(12, 0, 0, 0);
  value.setFullYear(year, monthIndex, day);
  return value;
}

function formatDate(date: Date): string {
  const year = String(date.getFullYear()).padStart(4, "0");
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function formatMonth(date: Date): string {
  return formatDate(date).slice(0, 7);
}
