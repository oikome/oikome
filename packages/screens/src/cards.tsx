// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Cards for Home made from what a collection holds: a day's totals over two
// weeks (a table with a date and a number, such as spending), and the latest
// of some readings (such as a home's temperatures). Arrange (arrange.tsx)
// places them on Home.

import { useMemo, useState } from "react";
import type { PointerEvent } from "react";
import type { SpaceSummary } from "@oikome/client";
import type { Table } from "apache-arrow";
import type { Preview } from "./content";
import { itemsOf, measure, numberAt, shapeOf } from "./content";
import { useClient, useLoad } from "./data";
import { calendarDate, monthAndDay } from "./format";
import { show } from "./parts";
import { Link } from "./router";
import { strings } from "./strings";

const s = strings.home;

/** Days a totals card shows. */
const DAYS = 14;
const DAY_MS = 86_400_000;

/** Whether a collection makes a totals card: a table with a date and a number. */
export function hasTotals(preview?: Preview): boolean {
  return preview?.shape.view === "table" && preview.shape.when !== undefined && preview.shape.detail !== undefined;
}

/** Whether a collection makes a latest-readings card. */
export function hasLatest(preview?: Preview): boolean {
  return preview?.shape.view === "series";
}

/** A day's totals, for the last `DAYS` days up to the latest one with data. */
function totalsOf(table: Table) {
  const shape = shapeOf(table);
  const number = shape.detail;
  if (number === undefined || shape.when === undefined) return undefined;
  const items = itemsOf(table, shape);
  const byDay = new Map<number, number>();
  items.forEach((item, r) => {
    if (item.when === undefined) return;
    const value = numberAt(table, number, r);
    if (value === undefined) return;
    // Days as written: the UTC day of a date or a time without a zone.
    const day = Math.floor(item.when / DAY_MS) * DAY_MS;
    byDay.set(day, (byDay.get(day) ?? 0) + value);
  });
  if (byDay.size === 0) return undefined;
  const last = Math.max(...byDay.keys());
  const days = Array.from({ length: DAYS }, (_, i) => last - (DAYS - 1 - i) * DAY_MS);
  const values = days.map((d) => byDay.get(d) ?? 0);
  const field = table.schema.fields[number]!;
  const total = values.reduce((a, b) => a + b, 0);
  const busy = values.filter((v) => v > 0);
  return { days, values, total, active: busy.length, label: measure(field.name).label };
}

/** Numbers as people read money and counts: 161.85, 1,284. */
function amount(n: number): string {
  return n.toLocaleString("en", { maximumFractionDigits: 2, minimumFractionDigits: Number.isInteger(n) ? 0 : 2 });
}

/** Bars of a day's totals over two weeks, from zero, with the day under the pointer. */
export function TotalsCard({ space, className }: { space: SpaceSummary; className?: string }) {
  const client = useClient();
  const rows = useLoad(async () => {
    const tables = await client.listTables({ space: space.name });
    const first = tables[0];
    return first ? (await client.readRows({ space: space.name, table: first.name, limit: 1000 })).table : undefined;
  }, [space.name]);
  const totals = useMemo(() => (rows.data ? totalsOf(rows.data) : undefined), [rows.data]);
  const [hover, setHover] = useState<number>();
  const max = totals ? Math.max(...totals.values, 0) : 0;
  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    const box = e.currentTarget.getBoundingClientRect();
    setHover(Math.max(0, Math.min(DAYS - 1, Math.floor(((e.clientX - box.left) / box.width) * DAYS))));
  };
  return (
    <section className={`oik-card screen-panel screen-totals ${className ?? ""}`} aria-label={s.totalsTitle(space.name)}>
      <header className="screen-panel__head">
        <h2 className="oik-label">{s.totalsTitle(space.name)}</h2>
        <Link to={`/collections/${encodeURIComponent(space.name)}`} className="oik-link">
          {s.open}
        </Link>
      </header>
      {!totals ? (
        <div className="screen-panel__skeleton oik-skel" aria-hidden="true" />
      ) : (
        <div className="screen-totals__body">
          <div
            className="screen-totals__bars"
            role="img"
            aria-label={s.totalsSummary(totals.label, amount(totals.total), calendarDate(totals.days[0]!), calendarDate(totals.days[DAYS - 1]!))}
            onPointerMove={onMove}
            onPointerDown={onMove}
            onPointerLeave={() => setHover(undefined)}
          >
            {totals.values.map((v, i) => (
              <span key={i} className={i === hover ? "screen-totals__bar is-hover" : "screen-totals__bar"}>
                <i
                  style={{
                    height: max > 0 ? `${Math.max(v > 0 ? 2 : 0, (v / max) * 100)}%` : 0,
                    // The bars grow in turn, from the oldest day.
                    animationDelay: `${i * 30}ms`,
                  }}
                />
              </span>
            ))}
            {hover !== undefined && (
              <span
                className={hover >= DAYS / 2 ? "screen-series__tip is-left" : "screen-series__tip"}
                style={{ left: `${((hover + 0.5) / DAYS) * 100}%` }}
              >
                <strong>{amount(totals.values[hover]!)}</strong>
                <span>{monthAndDay(totals.days[hover]!, true)}</span>
              </span>
            )}
          </div>
          <div className="screen-totals__axis" aria-hidden="true">
            <span>{monthAndDay(totals.days[0]!, true)}</span>
            <span>{s.totalsLine(amount(totals.total), totals.active)}</span>
            <span>{monthAndDay(totals.days[DAYS - 1]!, true)}</span>
          </div>
        </div>
      )}
    </section>
  );
}

/** The latest of each measure of some readings. */
export function LatestCard({
  space,
  preview,
  className,
}: {
  space: SpaceSummary;
  preview?: Preview;
  className?: string;
}) {
  const table = preview?.rows;
  const measures = preview?.shape.measures ?? [];
  const last = table ? table.numRows - 1 : -1;
  return (
    <section className={`oik-card screen-panel screen-latest ${className ?? ""}`} aria-label={space.name}>
      <header className="screen-panel__head">
        <h2 className="oik-label">{s.latestTitle(space.name)}</h2>
        <Link to={`/collections/${encodeURIComponent(space.name)}`} className="oik-link">
          {s.open}
        </Link>
      </header>
      {!table || last < 0 ? (
        <div className="screen-panel__skeleton oik-skel" aria-hidden="true" />
      ) : (
        <dl className="screen-latest__grid">
          {measures.slice(0, 4).map((c) => {
            const field = table.schema.fields[c]!;
            const { label, unit } = measure(field.name);
            const value = show(field, table.getChildAt(c)?.get(last));
            return (
              <div key={c}>
                <dt className="oik-label">{label}</dt>
                <dd>
                  {value ?? "—"}
                  {value !== null && unit && <small>{unit}</small>}
                </dd>
              </div>
            );
          })}
        </dl>
      )}
    </section>
  );
}
