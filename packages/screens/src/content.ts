// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// What a table holds, read from its columns and rows rather than from its
// name: photos (a column of images), notes (a few columns, one of them long
// text), readings (numbers over time, nothing else), or anything else, shown
// as a table. Covers, Home and a collection's page all show content this way.

import { DataType } from "apache-arrow";
import type { Field, Table } from "apache-arrow";
import type { NodeClient, SpaceSummary, TableSummary } from "@oikome/client";
import { isoTime, measureLabel } from "./format";
import { numeric, show } from "./parts";

export type View = "photos" | "notes" | "table" | "series";

/** How to show a table, and which columns play which part. */
export interface Shape {
  view: View;
  /** The image column (photos). */
  image?: number;
  /** The long text (notes), a photo's title, or a row's first text. */
  text?: number;
  /** A second line under a photo's title, such as its place; a row's first number. */
  detail?: number;
  /** Where a photo comes from and under which licence. */
  credit?: number;
  /** The first date or time column: when. */
  when?: number;
  /** The number columns of readings, in table order. */
  measures?: number[];
}

/** An item of content as covers and Home show it. */
export interface Item {
  /** Its row in the table it was read from. */
  row: number;
  image?: string;
  text?: string;
  detail?: string;
  credit?: string;
  /** Milliseconds since the epoch, when the table has a date. */
  when?: number;
  /** A calendar day, a time as written (no zone), or a moment. */
  whenKind?: "day" | "wall" | "moment";
}

/** The first rows of a collection's first table, read once per screen. */
export interface Preview {
  table?: TableSummary;
  shape: Shape;
  items: Item[];
  /** Rows to show in a table cover: column labels and a few rows of text. */
  rows?: Table;
}

const MAGIC: [number[], string][] = [
  [[0xff, 0xd8, 0xff], "image/jpeg"],
  [[0x89, 0x50, 0x4e, 0x47], "image/png"],
  [[0x47, 0x49, 0x46, 0x38], "image/gif"],
];

/** The image type of `bytes`, from its first bytes; undefined for anything else. */
export function imageType(bytes: Uint8Array): string | undefined {
  for (const [magic, type] of MAGIC) {
    if (magic.every((b, i) => bytes[i] === b)) return type;
  }
  // RIFF....WEBP
  if (bytes[0] === 0x52 && bytes[1] === 0x49 && bytes[8] === 0x57 && bytes[9] === 0x45) {
    return "image/webp";
  }
  return undefined;
}

/**
 * An image as a `data:` URL, the one kind of image source besides the
 * page's own that the web app's content security policy allows.
 */
export function imageSrc(bytes: Uint8Array): string | undefined {
  const type = imageType(bytes);
  if (!type) return undefined;
  let text = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    text += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return `data:${type};base64,${btoa(text)}`;
}

const isText = (f: Field) => DataType.isUtf8(f.type) || DataType.isLargeUtf8(f.type);
const isBytes = (f: Field) => DataType.isBinary(f.type) || DataType.isLargeBinary(f.type);
const isWhen = (f: Field) => DataType.isDate(f.type) || DataType.isTimestamp(f.type);
const named = (f: Field, ...words: string[]) => words.includes(f.name.toLowerCase());

/** Readings need a few rows before they read as a line. */
const SERIES_MIN_ROWS = 12;

/** A number column's name as a label and the unit its suffix names: living_room_celsius reads "Living room", "°C". */
export const measure = measureLabel;

/** "21.4 °C", or the value alone. */
export function withUnit(value: string, unit?: string): string {
  return unit ? `${value} ${unit}` : value;
}

/** A number cell as a number, for drawing; undefined when empty. */
export function numberAt(table: Table, column: number, row: number): number | undefined {
  const text = show(table.schema.fields[column]!, table.getChildAt(column)?.get(row));
  if (text === null) return undefined;
  const n = Number(text.replace(/,/g, ""));
  return Number.isFinite(n) ? n : undefined;
}

/** Read how to show `table` from its columns and first rows. */
export function shapeOf(table: Table): Shape {
  const fields = table.schema.fields;
  const value = (c: number, r: number): unknown => table.getChildAt(c)?.get(r);
  // A text column of days or times, as files and SQLite keep them.
  const isoColumn = (c: number) => {
    let seen = 0;
    for (let r = 0; r < table.numRows; r++) {
      const v = value(c, r);
      if (v === null || v === undefined || v === "") continue;
      if (typeof v !== "string" || !isoTime(v)) return false;
      seen++;
    }
    return seen > 0;
  };
  const typed = fields.findIndex(isWhen);
  const when = typed >= 0 ? typed : fields.findIndex((f, c) => isText(f) && isoColumn(c));
  const firstValue = (c: number): unknown => {
    for (let r = 0; r < table.numRows; r++) {
      const v = value(c, r);
      if (v !== null && v !== undefined) return v;
    }
    return undefined;
  };

  const image = fields.findIndex(
    (f, c) => isBytes(f) && (() => {
      const v = firstValue(c);
      return v instanceof Uint8Array && imageType(v) !== undefined;
    })(),
  );
  const texts = fields.map((f, c) => (isText(f) && c !== when ? c : -1)).filter((c) => c >= 0);
  const numbers = fields.map((f, c) => (numeric(f) ? c : -1)).filter((c) => c >= 0);
  if (image >= 0) {
    const credit = texts.find((c) => named(fields[c]!, "credit", "source", "attribution", "license"));
    const rest = texts.filter((c) => c !== credit);
    const title =
      rest.find((c) => named(fields[c]!, "title", "name", "caption")) ?? rest[0];
    // A photo's second line is where it was; never its file name.
    const detail =
      rest.find((c) => c !== title && named(fields[c]!, "place", "location", "where", "city")) ??
      rest.find((c) => c !== title && !named(fields[c]!, "file", "filename", "path"));
    return { view: "photos", image, text: title, detail, credit, when: when >= 0 ? when : undefined };
  }

  // Readings: numbers over time and nothing else, such as a thermometer's.
  if (when >= 0 && numbers.length > 0 && texts.length === 0 && table.numRows >= SERIES_MIN_ROWS) {
    return { view: "series", when, measures: numbers };
  }

  // Notes: a few columns, one of them text people wrote (long on average).
  if (fields.length <= 3 && table.numRows > 0) {
    const long = texts.find((c) => {
      let total = 0;
      for (let r = 0; r < table.numRows; r++) total += String(value(c, r) ?? "").length;
      return total / table.numRows >= 40;
    });
    if (long !== undefined) return { view: "notes", text: long, when: when >= 0 ? when : undefined };
  }
  const number = fields.findIndex(numeric);
  return {
    view: "table",
    text: texts[0],
    detail: number >= 0 ? number : undefined,
    when: when >= 0 ? when : undefined,
  };
}

/** The rows of `table` as items, in the parts `shape` gave them. */
export function itemsOf(table: Table, shape: Shape): Item[] {
  const at = (c: number | undefined, r: number): unknown =>
    c === undefined ? undefined : table.getChildAt(c)?.get(r);
  const text = (c: number | undefined, r: number) =>
    c === undefined ? undefined : (show(table.schema.fields[c]!, at(c, r)) ?? undefined);
  const whenField = shape.when === undefined ? undefined : table.schema.fields[shape.when];
  // A reading reads as its first two measures: "Living room 21.4 °C, Bedroom 19.9 °C".
  const reading = (r: number) =>
    (shape.measures ?? [])
      .slice(0, 2)
      .map((c) => {
        const value = text(c, r);
        if (value === undefined) return undefined;
        const { label, unit } = measure(table.schema.fields[c]!.name);
        return `${label} ${withUnit(value, unit)}`;
      })
      .filter(Boolean)
      .join(", ") || undefined;
  return Array.from({ length: table.numRows }, (_, r) => {
    const bytes = at(shape.image, r);
    const raw = at(shape.when, r);
    const iso = typeof raw === "string" ? isoTime(raw) : undefined;
    const when = iso ? iso.ms : raw;
    return {
      row: r,
      image: bytes instanceof Uint8Array ? imageSrc(bytes) : undefined,
      text: shape.view === "series" ? reading(r) : text(shape.text, r),
      detail: text(shape.detail, r),
      credit: text(shape.credit, r),
      when: when === null || when === undefined ? undefined : Number(when),
      whenKind: !whenField
        ? undefined
        : iso
          ? iso.day
            ? "day"
            : "wall"
          : DataType.isDate(whenField.type)
            ? "day"
            : DataType.isTimestamp(whenField.type) && !whenField.type.timezone
              ? "wall"
              : "moment",
    };
  });
}

/** Rows a cover or Home reads: enough to pick the latest and fill a collage. */
export const PREVIEW_ROWS = 12;
/** The latest readings a cover draws: three days of hourly ones. */
const SERIES_ROWS = 72;

const previews = new Map<string, Promise<Preview>>();

/**
 * The first table of `space` and its first rows, read once and shared by
 * every cover and Home tile on the screen. A collection shared by someone
 * else is not read just for a cover: its owner's computer may be off.
 */
export function loadPreview(client: NodeClient, space: SpaceSummary): Promise<Preview> {
  // One shared with you is read from its owner's data center; when that is
  // off, the read fails and its cover stays plain.
  let preview = previews.get(space.name);
  if (!preview) {
    preview = (async () => {
      const tables = await client.listTables({ space: space.name });
      const table = tables[0];
      if (!table || table.rows === 0) return { table, shape: { view: "table" as View }, items: [] };
      const { table: rows } = await client.readRows({
        space: space.name,
        table: table.name,
        limit: PREVIEW_ROWS,
      });
      const shape = shapeOf(rows);
      // Readings are drawn from the latest ones, at the end of the table.
      const total = Number(table.rows ?? 0);
      if (shape.view === "series" && total > rows.numRows) {
        const count = Math.min(SERIES_ROWS, total);
        const { table: tail } = await client.readRows({
          space: space.name,
          table: table.name,
          limit: count,
          offset: total - count,
        });
        return { table, shape, items: chronological(itemsOf(tail, shape)), rows: tail };
      }
      const items = itemsOf(rows, shape);
      return { table, shape, items: shape.view === "table" ? items : chronological(items), rows };
    })();
    // A failed read is tried again next time.
    preview.catch(() => previews.delete(space.name));
    previews.set(space.name, preview);
  }
  return preview;
}

/** Forget what was read, after collections were made or removed. */
export function forgetPreviews(name?: string) {
  if (name) previews.delete(name);
  else previews.clear();
}

/** Items in the order they happened, when they have dates; as read otherwise. */
export function chronological(items: Item[]): Item[] {
  if (!items.some((item) => item.when !== undefined)) return items;
  return [...items].sort((a, b) => (a.when ?? 0) - (b.when ?? 0));
}

/** A tag: `#` and a word, after a space or at the start of a line. */
const TAG = /(?:^|\s)#([\p{L}\p{N}][\p{L}\p{N}_\-/]*)/gu;
/** A line of tags and nothing else, as notes apps write them. */
const TAG_LINE = /^\s*(?:#[\p{L}\p{N}][\p{L}\p{N}_\-/]*\s*)+$/u;

/** The tags of a note, each once, in the order written. */
export function tagsOf(text: string | undefined): string[] {
  if (!text) return [];
  return [...new Set(Array.from(text.matchAll(TAG), (m) => m[1]!))];
}

/**
 * A note's text without its lines of tags only, which are shown as tags;
 * tags inside a sentence stay where they are.
 */
export function withoutTagLines(text: string | undefined): string | undefined {
  if (text === undefined) return undefined;
  return text
    .split("\n")
    .filter((line) => !TAG_LINE.test(line))
    .join("\n")
    .trim();
}

/** The newest item, by its date; the first one when there are no dates. */
export function latest(items: Item[]): Item | undefined {
  return items.reduce<Item | undefined>(
    (best, item) => (!best || (item.when ?? 0) > (best.when ?? 0) ? item : best),
    undefined,
  );
}
