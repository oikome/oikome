// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Dates and values as people read them.

const day = new Intl.DateTimeFormat("en", { month: "short", day: "numeric", year: "numeric" });
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
