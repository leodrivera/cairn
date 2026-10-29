// Sizes and dates for people, in the current language.
import { lang } from "../i18n/index";

const units = ["B", "KB", "MB", "GB", "TB"];

/** 1024 steps; whole bytes, one decimal from KB up when it is not zero. */
export function formatSize(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  const text = new Intl.NumberFormat(lang(), { maximumFractionDigits: digits }).format(value);
  return `${text} ${units[unit]}`;
}

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

function capital(s: string): string {
  return s.charAt(0).toLocaleUpperCase(lang()) + s.slice(1);
}

/** "Today, 09:14", "Yesterday", "Oct 24", or "Oct 24, 2024" for another year. */
export function formatWhen(sec: number, now: Date = new Date()): string {
  const d = new Date(sec * 1000);
  const rel = new Intl.RelativeTimeFormat(lang(), { numeric: "auto" });
  if (sameDay(d, now)) {
    const time = new Intl.DateTimeFormat(lang(), { hour: "2-digit", minute: "2-digit", hour12: false }).format(d);
    return `${capital(rel.format(0, "day"))}, ${time}`;
  }
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (sameDay(d, yesterday)) return capital(rel.format(-1, "day"));
  const opts: Intl.DateTimeFormatOptions = { month: "short", day: "numeric" };
  if (d.getFullYear() !== now.getFullYear()) opts.year = "numeric";
  return new Intl.DateTimeFormat(lang(), opts).format(d);
}
