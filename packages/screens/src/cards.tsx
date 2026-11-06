// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Cards for Home made from what a collection holds: a day's totals over two
// weeks (a table with a date and a number, such as spending), and the latest
// of some readings (such as a home's temperatures). Arrange (arrange.tsx)
// places them on Home.

import { useState } from "react";
import type { PointerEvent } from "react";
import type { NodeClient, SpaceSummary } from "@oikome/client";
import type { Preview } from "./content";
import { measure, shapeOf } from "./content";
import { useClient, useLoad } from "./data";
import { calendarDate, isoTime, monthAndDay } from "./format";
import { show } from "./parts";
import { Link } from "./router";
import { strings } from "./strings";

const s = strings.home;

/** Days a totals card shows. */
const DAYS = 14;
/** Rows read to tell which columns hold a day and an amount. */
const SAMPLE_ROWS = 50;

/** Whether a collection makes a totals card: a table with a date and a number. */
export function hasTotals(preview?: Preview): boolean {
  return preview?.shape.view === "table" && preview.shape.when !== undefined && preview.shape.detail !== undefined;
}

/** Whether a collection makes a latest-readings card. */
export function hasLatest(preview?: Preview): boolean {
  return preview?.shape.view === "series";
}

/**
 * A day's totals, for the last `DAYS` days up to the latest one with data.
 * A sample of the table decides which columns hold the day and the amount;
 * the node adds up every row, here or on a computer lent to you.
 */
async function totalsOf(client: NodeClient, space: string) {
  const tables = await client.listTables({ space });
  const first = tables[0];
  if (!first) return undefined;
  const sample = (await client.readRows({ space, table: first.name, limit: SAMPLE_ROWS })).table;
  const shape = shapeOf(sample);
  if (shape.when === undefined || shape.detail === undefined) return undefined;
  const fields = sample.schema.fields;
  const amountField = fields[shape.detail]!;
  const answer = await client.dailyTotals({
    space,
    table: first.name,
    when: fields[shape.when]!.name,
    amount: amountField.name,
  });
  const days = answer.days.map((d) => isoTime(d.day)?.ms ?? 0);
  if (days.length !== DAYS) return undefined;
  const values = answer.days.map((d) => d.total);
  const total = values.reduce((a, b) => a + b, 0);
  const busy = values.filter((v) => v > 0);
  // Where it was worked out, when a computer lent to you is in use.
  const lender = answer.lender ?? "";
  const where = answer.borrowed
    ? s.workedOutOn(lender)
    : answer.stayedHere && answer.stayedHere !== "small"
      ? s.stayedHere[answer.stayedHere]?.(lender)
      : undefined;
  return { days, values, total, active: busy.length, label: measure(amountField.name).label, where };
}

/** Numbers as people read money and counts: 161.85, 1,284. */
function amount(n: number): string {
  return n.toLocaleString("en", { maximumFractionDigits: 2, minimumFractionDigits: Number.isInteger(n) ? 0 : 2 });
}

/** Bars of a day's totals over two weeks, from zero, with the day under the pointer. */
export function TotalsCard({ space, className }: { space: SpaceSummary; className?: string }) {
  const client = useClient();
  const totals = useLoad(() => totalsOf(client, space.name), [space.name]).data;
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
          {totals.where && <p className="screen-totals__where">{totals.where}</p>}
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
