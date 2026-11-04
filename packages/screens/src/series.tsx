// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Readings over time, such as a thermometer's: the latest values, and a line
// of one measure. A line rather than bars, because a temperature has no zero
// worth drawing from; the rows stay below the chart as a table.

import { useMemo, useState } from "react";
import type { PointerEvent } from "react";
import { Segmented } from "@oikome/ui";
import type { Table } from "apache-arrow";
import { DataType } from "apache-arrow";
import type { Shape } from "./content";
import { measure, numberAt, withUnit } from "./content";
import { dateTime, isoTime, monthAndDay, wallTime } from "./format";
import { show } from "./parts";
import { strings } from "./strings";

const s = strings.collections;

/** Width and height of the drawing; the SVG stretches to its box. */
const W = 1000;
const H = 200;

interface Line {
  values: (number | undefined)[];
  min: number;
  max: number;
}

/** The values of `column`, and a range with a little air above and below. */
function lineOf(table: Table, column: number): Line {
  const values = Array.from({ length: table.numRows }, (_, r) => numberAt(table, column, r));
  const known = values.filter((v): v is number => v !== undefined);
  const lo = known.length ? Math.min(...known) : 0;
  const hi = known.length ? Math.max(...known) : 1;
  const air = (hi - lo || 1) * 0.12;
  return { values, min: lo - air, max: hi + air };
}

/** The line as an SVG path; a gap where a reading is missing. */
function pathOf({ values, min, max }: Line): string {
  const step = values.length > 1 ? W / (values.length - 1) : 0;
  let d = "";
  let pen = false;
  values.forEach((v, i) => {
    if (v === undefined) {
      pen = false;
      return;
    }
    const x = (i * step).toFixed(1);
    const y = (H - ((v - min) / (max - min)) * H).toFixed(1);
    d += `${pen ? "L" : "M"}${x} ${y}`;
    pen = true;
  });
  return d;
}

/** The cover of readings: the latest of the first measure, and its line. */
export function SeriesCover({ table, shape }: { table: Table; shape: Shape }) {
  const column = shape.measures?.[0];
  const line = useMemo(() => (column === undefined ? undefined : lineOf(table, column)), [table, column]);
  if (column === undefined || !line) return null;
  const { label, unit } = measure(table.schema.fields[column]!.name);
  const last = table.numRows > 0 ? show(table.schema.fields[column]!, table.getChildAt(column)?.get(table.numRows - 1)) : null;
  return (
    <div aria-hidden="true" className="screen-cover__art screen-cover__art--series">
      <span className="screen-series-cover__label">{label}</span>
      {last !== null && <span className="screen-series-cover__value">{withUnit(last, unit)}</span>}
      <svg className="screen-series-cover__line" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
        <path d={pathOf(line)} vectorEffect="non-scaling-stroke" />
      </svg>
    </div>
  );
}

/** When each row was, in milliseconds, and how to read it. */
function timesOf(table: Table, column: number | undefined) {
  const field = column === undefined ? undefined : table.schema.fields[column];
  const text = !!field && (DataType.isUtf8(field.type) || DataType.isLargeUtf8(field.type));
  // A time without a zone, a date, or one written as text, is shown as written.
  const asWritten =
    !!field && (text || DataType.isDate(field.type) || (DataType.isTimestamp(field.type) && !field.type.timezone));
  const times = Array.from({ length: table.numRows }, (_, r) => {
    const v = column === undefined ? undefined : table.getChildAt(column)?.get(r);
    if (v === null || v === undefined) return undefined;
    return typeof v === "string" ? isoTime(v)?.ms : Number(v);
  });
  const format = (ms: number) =>
    field && DataType.isDate(field.type) ? monthAndDay(ms, true) : asWritten ? wallTime(ms) : dateTime(ms);
  return { times, asWritten, format };
}

/** A collection of readings: the latest of each measure, a line of the one picked, and the rows. */
export function SeriesView({ table, shape }: { table: Table; shape: Shape }) {
  const measures = shape.measures ?? [];
  const [picked, setPicked] = useState(measures[0]);
  const column = measures.includes(picked!) ? picked! : measures[0]!;
  const line = useMemo(() => lineOf(table, column), [table, column]);
  const { times, asWritten, format } = useMemo(() => timesOf(table, shape.when), [table, shape.when]);
  const [hover, setHover] = useState<number>();
  const field = table.schema.fields[column]!;
  const { label, unit } = measure(field.name);
  const n = line.values.length;

  // A mark at each day's start, at most eight of them.
  const days = useMemo(() => {
    const starts: number[] = [];
    times.forEach((t, i) => {
      if (t === undefined) return;
      const d = new Date(t);
      if ((asWritten ? d.getUTCHours() : d.getHours()) === 0 && (asWritten ? d.getUTCMinutes() : d.getMinutes()) === 0) {
        starts.push(i);
      }
    });
    const every = Math.ceil(starts.length / 8) || 1;
    return starts.filter((_, k) => k % every === 0);
  }, [times, asWritten]);

  const y = (v: number) => (1 - (v - line.min) / (line.max - line.min)) * 100;
  const x = (i: number) => (n > 1 ? (i / (n - 1)) * 100 : 0);
  const known = line.values.filter((v): v is number => v !== undefined);
  const lo = known.length ? Math.min(...known) : undefined;
  const hi = known.length ? Math.max(...known) : undefined;
  // Numbers as the column writes them: 21.0 for a DECIMAL(4, 1), 48 for an integer.
  const digits = DataType.isDecimal(field.type) ? field.type.scale : DataType.isInt(field.type) ? 0 : undefined;
  const text = (v: number | undefined) =>
    v === undefined ? "—" : withUnit(digits === undefined ? String(v) : v.toFixed(digits), unit);
  const first = times.find((t) => t !== undefined);
  const lastTime = [...times].reverse().find((t) => t !== undefined);
  const summary =
    lo !== undefined && hi !== undefined && first !== undefined && lastTime !== undefined
      ? s.seriesSummary(label, text(lo), text(hi), format(first), format(lastTime))
      : label;

  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    const box = e.currentTarget.getBoundingClientRect();
    const i = Math.round(((e.clientX - box.left) / box.width) * (n - 1));
    setHover(Math.max(0, Math.min(n - 1, i)));
  };
  const at = hover !== undefined && line.values[hover] !== undefined ? hover : undefined;

  return (
    <section className="oik-card screen-series">
      <dl className="screen-series__figures">
        {measures.map((c) => {
          const m = measure(table.schema.fields[c]!.name);
          const value = show(table.schema.fields[c]!, table.getChildAt(c)?.get(table.numRows - 1));
          return (
            <div key={c} className={c === column ? "screen-series__figure is-picked" : "screen-series__figure"}>
              <dt className="oik-label">{m.label}</dt>
              <dd>{value === null ? "—" : withUnit(value, m.unit)}</dd>
            </div>
          );
        })}
      </dl>
      <div className="screen-series__head">
        <span className="screen-series__title">
          {label}
          {lo !== undefined && hi !== undefined && (
            <span className="screen-series__range">{s.seriesRange(text(lo), text(hi))}</span>
          )}
        </span>
        {measures.length > 1 && (
          <Segmented
            label={s.seriesPick}
            value={String(column)}
            onChange={(value) => setPicked(Number(value))}
            options={measures.map((c) => ({ value: String(c), label: measure(table.schema.fields[c]!.name).label }))}
          />
        )}
      </div>
      <div className="screen-series__plot">
        <span className="screen-series__tick" style={{ top: `${y(hi ?? 0)}%` }}>
          {text(hi)}
        </span>
        <span className="screen-series__tick" style={{ top: `${y(lo ?? 0)}%` }}>
          {text(lo)}
        </span>
        <div
          className="screen-series__area"
          role="img"
          aria-label={summary}
          onPointerMove={onMove}
          onPointerDown={onMove}
          onPointerLeave={() => setHover(undefined)}
        >
          <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" aria-hidden="true">
            {days.map((i) => (
              <line key={i} className="screen-series__day" x1={(x(i) * W) / 100} x2={(x(i) * W) / 100} y1={0} y2={H} vectorEffect="non-scaling-stroke" />
            ))}
            {hi !== undefined && (
              <line className="screen-series__grid" x1={0} x2={W} y1={(y(hi) * H) / 100} y2={(y(hi) * H) / 100} vectorEffect="non-scaling-stroke" />
            )}
            {lo !== undefined && (
              <line className="screen-series__grid" x1={0} x2={W} y1={(y(lo) * H) / 100} y2={(y(lo) * H) / 100} vectorEffect="non-scaling-stroke" />
            )}
            <path className="screen-series__line" d={pathOf(line)} vectorEffect="non-scaling-stroke" />
          </svg>
          {at !== undefined && (
            <>
              <span className="screen-series__cross" style={{ left: `${x(at)}%` }} />
              <span className="screen-series__dot" style={{ left: `${x(at)}%`, top: `${y(line.values[at]!)}%` }} />
              <span
                className={x(at) > 50 ? "screen-series__tip is-left" : "screen-series__tip"}
                style={{ left: `${x(at)}%` }}
              >
                <strong>{text(line.values[at])}</strong>
                {times[at] !== undefined && <span>{format(times[at]!)}</span>}
              </span>
            </>
          )}
        </div>
        <div className="screen-series__days" aria-hidden="true">
          {days.map((i) => (
            // A day near the right edge ends at its line, so it never runs off the card.
            <span key={i} className={x(i) > 80 ? "is-end" : undefined} style={{ left: `${x(i)}%` }}>
              {times[i] !== undefined ? monthAndDay(times[i]!, asWritten) : ""}
            </span>
          ))}
        </div>
      </div>
    </section>
  );
}
