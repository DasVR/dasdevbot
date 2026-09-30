import type { Approval } from "./api";

export const GREETING_KEY = "dasdevbot.greeting";
export const MODE_RETURN_ENTRY = "mode-return";

export type DayPart = "morning" | "afternoon" | "evening";

export function dayPart(date: Date): DayPart {
  const hour = date.getHours();
  if (hour >= 5 && hour < 12) {
    return "morning";
  }
  if (hour >= 12 && hour < 17) {
    return "afternoon";
  }
  return "evening";
}

export function localDay(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

export function greetingHeading(part: DayPart, name: string): string {
  switch (part) {
    case "morning":
      return `Morning, ${name}.`;
    case "afternoon":
      return `Afternoon, ${name}.`;
    case "evening":
      return `Evening, ${name}.`;
    default: {
      const exhaustive: never = part;
      return exhaustive;
    }
  }
}

/** Waiting count replaces the ink-2 line. Morning counts the overnight work. */
export function greetingLine(part: DayPart, waiting: number, done: number): string {
  const when = part === "morning" ? "overnight" : "today";
  const finished = `${done} done ${when}`;
  if (waiting > 0) {
    const wait = waiting === 1 ? "1 waiting on you" : `${waiting} waiting on you`;
    return `${wait} · ${finished}.`;
  }
  return `Nothing waiting · ${finished}.`;
}

export function doneToday(approvals: readonly Approval[], now: number): number {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  const from = start.getTime();
  const to = from + 86_400_000;
  return approvals.filter((approval) => {
    return (
      approval.status !== "pending" &&
      approval.decided_at != null &&
      approval.decided_at >= from &&
      approval.decided_at < to
    );
  }).length;
}

export function greetingSeen(date: Date): boolean {
  try {
    const raw = localStorage.getItem(GREETING_KEY);
    if (!raw) {
      return false;
    }
    const parsed = JSON.parse(raw) as { day?: unknown };
    return parsed.day === localDay(date);
  } catch {
    return false;
  }
}

export function markGreetingSeen(date: Date): void {
  try {
    localStorage.setItem(GREETING_KEY, JSON.stringify({ day: localDay(date) }));
  } catch {
    // Private-mode storage can refuse the write. The greeting still shows this once.
  }
}

/** Focus / Away return shows the Review sheet, not the greeting. */
export function modeReturnSkipsGreeting(): boolean {
  return sessionStorage.getItem("dasdevbot.entry") === MODE_RETURN_ENTRY;
}
