// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Dates and values as people read them.

const day = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });
// A calendar date has no time zone: show the day it names.
const calendarDay = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
  timeZone: "UTC",
});
const moment = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

const monthDay = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const monthDayAsWritten = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", timeZone: "UTC" });

/** Just the day, "Nov 7", for a caption; `asWritten` for a date or time stored without a zone. */
export function monthAndDay(ms: number, asWritten = false): string {
  return (asWritten ? monthDayAsWritten : monthDay).format(new Date(ms));
}

const ISO = /^(\d{4})-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2})(?::(\d{2}))?)?$/;

/**
 * A day or time written as text, as files and SQLite keep them
 * ("2025-11-07", "2025-11-07 09:40:00"), read as written: milliseconds on
 * the UTC clock, the way a TIMESTAMP without a zone is shown.
 */
export function isoTime(text: string): { ms: number; day: boolean } | undefined {
  const m = ISO.exec(text.trim());
  if (!m) return undefined;
  const [, y, mo, d, h, mi, s] = m;
  const ms = Date.UTC(Number(y), Number(mo) - 1, Number(d), Number(h ?? 0), Number(mi ?? 0), Number(s ?? 0));
  return Number.isNaN(ms) ? undefined : { ms, day: h === undefined };
}

/** A DATE cell: "Oct 28, 2026". */
export function calendarDate(ms: number): string {
  return calendarDay.format(new Date(ms));
}

/** A TIMESTAMP cell, in the viewer's time zone: "Oct 28, 2026, 3:04 PM". */
export function dateTime(ms: number): string {
  return moment.format(new Date(ms));
}

const wall = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
  timeZone: "UTC",
});

/**
 * A time written without a time zone (SQL `TIMESTAMP`): shown as written,
 * the clock on the wall where it was noted, not moved to the viewer's zone.
 */
export function wallTime(ms: number): string {
  return wall.format(new Date(ms));
}

/** A DECIMAL's unscaled digits with its point put back: "1500", 2 reads "15.00". */
export function decimal(unscaled: string, scale: number): string {
  let digits = unscaled;
  const negative = digits.startsWith("-");
  if (negative) digits = digits.slice(1);
  if (scale > 0) {
    digits = digits.padStart(scale + 1, "0");
    digits = `${digits.slice(0, -scale)}.${digits.slice(-scale)}`;
  }
  return negative ? `-${digits}` : digits;
}

const SIZE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

/** A size as the system's file browser writes it, in powers of 1000: "412 GB". */
export function bytes(n: number): string {
  let value = n;
  let unit = 0;
  while (value >= 1000 && unit < SIZE_UNITS.length - 1) {
    value /= 1000;
    unit++;
  }
  const digits = value >= 100 || unit === 0 ? 0 : 1;
  return `${value.toFixed(digits).replace(/\.0$/, "")} ${SIZE_UNITS[unit]}`;
}

/** A path under a home folder as ~/…, the way people know it. */
export function homeRelative(path: string): string {
  return path.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, "~");
}

const ACRONYMS = new Set(["id", "url", "uuid", "ip", "api"]);

/** A column name as a header: "created_at" reads "Created at", "user_id" "User ID". */
// Suffixes that name a unit and little else.
const UNITS: Record<string, string> = {
  celsius: "°C",
  fahrenheit: "°F",
  pct: "%",
  percent: "%",
  kwh: "kWh",
  km: "km",
  kg: "kg",
};

/** A column name and the unit its suffix names: living_room_celsius is "living_room", "°C". */
export function splitUnit(name: string): { base: string; unit?: string } {
  const m = /^(.+?)[_-](celsius|fahrenheit|pct|percent|kwh|km|kg)$/i.exec(name);
  return m ? { base: m[1]!, unit: UNITS[m[2]!.toLowerCase()] } : { base: name };
}

/** A column name as words: "living_room" reads "Living room". */
function words(name: string): string {
  const text = name
    .replace(/[_-]+/g, " ")
    .trim()
    .split(/\s+/)
    .map((word) => (ACRONYMS.has(word.toLowerCase()) ? word.toUpperCase() : word))
    .join(" ");
  return text ? text[0]!.toUpperCase() + text.slice(1) : name;
}

/** A column's header: "Living room (°C)", "Day". */
export function columnLabel(name: string): string {
  const { base, unit } = splitUnit(name);
  return unit ? `${words(base)} (${unit})` : words(name);
}

/** A measure's name and unit apart: "Living room", "°C". */
export function measureLabel(name: string): { label: string; unit?: string } {
  const { base, unit } = splitUnit(name);
  return { label: words(base), unit };
}
const time = new Intl.DateTimeFormat("en", { hour: "numeric", minute: "2-digit" });

/** "Dec 1, 2026" for seconds since the epoch. */
export function dateFromSeconds(seconds: number): string {
  return day.format(new Date(seconds * 1000));
}

/** "Dec 1, 2026" for milliseconds since the epoch. */
export function dateFromMs(ms: number): string {
  return day.format(new Date(ms));
}

/** "7:36 PM" today, else "Nov 2, 7:36 PM". */
export function sameDayTime(ms: number, now = Date.now()): string {
  const today = new Date(ms).toDateString() === new Date(now).toDateString();
  return today ? time.format(new Date(ms)) : `${monthAndDay(ms)}, ${time.format(new Date(ms))}`;
}

/** "Today 7:36 PM", or "Nov 2, 7:36 PM". */
export function dayAndTime(ms: number, now = Date.now()): string {
  const today = new Date(ms).toDateString() === new Date(now).toDateString();
  return today ? `Today ${time.format(new Date(ms))}` : `${monthAndDay(ms)}, ${time.format(new Date(ms))}`;
}

/** "Just now", "4 min ago", "3 h ago", then the date and time. */
export function ago(ms: number, now = Date.now()): string {
  const minutes = Math.round((now - ms) / 60_000);
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return `${day.format(new Date(ms))}, ${time.format(new Date(ms))}`;
}

/** A cell of a table of rows, as text. */
export function cell(value: unknown): string | null {
  if (value === null || value === undefined) return null;
  if (typeof value === "bigint") return value.toString();
  if (value instanceof Date) return value.toISOString();
  if (value instanceof Uint8Array) return `${value.length} bytes`;
  if (typeof value === "object") {
    try {
      return JSON.stringify(value, (_, v) => (typeof v === "bigint" ? v.toString() : v));
    } catch {
      return String(value);
    }
  }
  return String(value);
}

const KEY_PREFIX = "did:key:";

/** An ID as people see and pass it on: the key, without the `did:key:` scheme. */
export function shortId(id: string): string {
  return id.startsWith(KEY_PREFIX) ? id.slice(KEY_PREFIX.length) : id;
}

/** A long ID cut in the middle, "z6MkrYLgFi4cuz…TTsM5U6Ui", for a row; the whole one is copied. */
export function middle(id: string, head = 14, tail = 9): string {
  return id.length <= head + tail + 1 ? id : `${id.slice(0, head)}…${id.slice(-tail)}`;
}

/** An ID someone pasted, in either form, as the node expects it. */
export function fullId(input: string): string {
  const id = input.trim();
  return id.startsWith("did:") ? id : `${KEY_PREFIX}${id}`;
}
