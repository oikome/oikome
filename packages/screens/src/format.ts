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

/** A path under a home folder as ~/…, the way people know it. */
export function homeRelative(path: string): string {
  return path.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, "~");
}

const ACRONYMS = new Set(["id", "url", "uuid", "ip", "api"]);

/** A column name as a header: "created_at" reads "Created at", "user_id" "User ID". */
export function columnLabel(name: string): string {
  const words = name
    .replace(/[_-]+/g, " ")
    .trim()
    .split(/\s+/)
    .map((word) => (ACRONYMS.has(word.toLowerCase()) ? word.toUpperCase() : word));
  const text = words.join(" ");
  return text ? text[0]!.toUpperCase() + text.slice(1) : name;
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

/** An ID someone pasted, in either form, as the node expects it. */
export function fullId(input: string): string {
  const id = input.trim();
  return id.startsWith("did:") ? id : `${KEY_PREFIX}${id}`;
}
