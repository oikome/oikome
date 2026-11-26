// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The explorer of an analysis table: the charts kept with it in a row, the
// one shown, and beside it the panel that describes it the way BI tools do.
//
// A chart is a description, never SQL, and whatever is chosen takes effect
// at once. The rows are grouped by the time (a column of time seen at a
// grain, over a range) and by every dimension together, as SQL's GROUP BY
// is: a table shows a column for each; a chart lays its groups along its
// axis (the time, else the dimensions), and with the time along it the
// dimensions' values are series, as what is compared is. The time keeps its
// own order; an order by a metric and a number of groups are for the
// dimensions. A click on a group zooms a period into its next grain or
// breaks the group down by another column: either is an edit of the
// description (a filter, and a finer grain or another dimension), so the
// panel always says what is drawn, and the path above the chart steps back
// through them. The node writes the query and answers; this component draws
// it (a table here, the charts with ECharts, loaded on demand) and shows the
// query that ran.
//
// A new chart starts over the table's first column of time, by month, or,
// without one, with nothing chosen: the table's rows. A chart is kept when
// saved, under a name the first time, and opens again from the row above;
// what the owner changes is a draft until saved or discarded.
// Assistants allowed to read the collection may keep charts too; those say
// who made them.

import { useEffect, useMemo, useRef, useState } from "react";
import {
  Button,
  Card,
  Checkbox,
  ConfirmDialog,
  Dialog,
  ErrorText,
  Icon,
  Menu,
  PointMenu,
  Select,
  Tag,
  TextField,
  useToast,
} from "@oikome/ui";
import type { MenuItem } from "@oikome/ui";
import type {
  AnalysisTable,
  ChartAggregate,
  ChartDimension,
  ChartFilter,
  ChartKind,
  ChartLevel,
  ChartMetric,
  ChartQuery,
  ChartResult,
  ChartSpec,
  ChartTime,
  DateGrain,
  FileAnalysis,
  TimeRange,
} from "@oikome/client";
import { useAction, useClient } from "../data";
import { strings } from "../strings";
import { locale } from "../i18n/languages";
import type { Drawing } from "../chartDraw";

const x = strings.explorer;

const CHARTS: ChartKind[] = ["table", "column", "bar", "line", "area", "pie", "treemap"];
const GRAINS: DateGrain[] = ["year", "quarter", "month", "week", "day"];
/** The grain a period zooms into; a week opens into its days. */
const FINER: Partial<Record<DateGrain, DateGrain>> = { year: "quarter", quarter: "month", month: "day", week: "day" };
const RANGES: TimeRange[] = ["all", "last7Days", "last30Days", "last90Days", "last365Days", "thisYear", "custom"];
const NUMBER_AGGREGATES: ChartAggregate[] = ["sum", "average", "median", "min", "max", "count", "distinct"];
const OTHER_AGGREGATES: ChartAggregate[] = ["count", "distinct"];
const LIMITS = [5, 10, 20, 50];
const NEW = "new";

interface View {
  id: string;
  name: string;
  spec: ChartSpec;
  by?: string;
  savedAtMs?: number;
}

type Field = AnalysisTable["fields"][number];
/** What a column holds: as the owner has it read, else as DuckDB read it. */
function kindOf(field: Field): string {
  if (field.readAs) return field.readAs;
  if (/^Date/.test(field.dataType)) return "date";
  if (/^Timestamp/.test(field.dataType)) return "timestamp";
  if (/^(U?Int|Float|Decimal)/.test(field.dataType)) return "number";
  return "text";
}
const isNumber = (field: Field | undefined) => field !== undefined && kindOf(field) === "number";
/** A column of time: a date or a timestamp. */
const isTime = (field: Field | undefined) => field !== undefined && (kindOf(field) === "date" || kindOf(field) === "timestamp");
const shownFields = (table: AnalysisTable) => table.fields.filter((f) => f.name !== "_source_file");
const fieldOf = (table: AnalysisTable, name: string | null) => table.fields.find((f) => f.name === name);

const number = (value: number) => new Intl.NumberFormat(locale(), { maximumFractionDigits: 2 }).format(value);
/** A metric as SQL writes it, the way BI tools show one: SUM(cost), COUNT(DISTINCT author), COUNT(*). */
const FUNCTIONS: Record<ChartAggregate, string> = {
  sum: "SUM",
  average: "AVG",
  median: "MEDIAN",
  min: "MIN",
  max: "MAX",
  count: "COUNT",
  distinct: "COUNT(DISTINCT",
};
const COUNT_ROWS: ChartMetric = { column: null, aggregate: "count" };
const metricName = (m: ChartMetric) =>
  !m.column ? "COUNT(*)" : m.aggregate === "distinct" ? `COUNT(DISTINCT ${m.column})` : `${FUNCTIONS[m.aggregate]}(${m.column})`;
/** An aggregate function as a choice: its name, and what it does in words. */
const functionName = (a: ChartAggregate) => `${a === "distinct" ? "COUNT DISTINCT" : FUNCTIONS[a]} · ${x.aggregates[a] ?? a}`;
const day = (key: string) => new Date(`${key}T00:00`);
/** A value a group is keyed by, in the reader's words: a period of time, or the value itself. */
function keyName(level: ChartLevel | undefined, key: string | null): string {
  if (key === null) return x.blank;
  if (!level?.grain) return key;
  try {
    if (level.grain === "year") return key;
    if (level.grain === "quarter") return x.quarter(key.slice(0, 4), key.slice(6));
    if (level.grain === "month") return new Intl.DateTimeFormat(locale(), { year: "numeric", month: "short" }).format(day(`${key}-01`));
    const date = new Intl.DateTimeFormat(locale(), { month: "short", day: "numeric" }).format(day(key));
    return level.grain === "week" ? x.weekOf(date) : date;
  } catch {
    return key;
  }
}
/** A column a chart's groups are keyed by, as a heading: the time with its grain. */
const levelName = (level: ChartLevel) => (level.grain ? x.colAt(level.column, x.grains[level.grain] ?? level.grain) : level.column);

/**
 * A new chart: over the table's first column of time, by month, when it has
 * one (what is wanted most often, and a click on a month goes on from
 * there); else nothing chosen, so the table's rows.
 */
function newChart(table: AnalysisTable): ChartSpec {
  const time = shownFields(table).find((f) => isTime(f));
  return time
    ? { chart: "column", time: { column: time.name, grain: "month", range: "all" }, dimensions: [], compare: [], metrics: [COUNT_ROWS], filters: [], sort: null, limit: null }
    : { chart: "table", time: null, dimensions: [], compare: [], metrics: [], filters: [], sort: null, limit: null };
}

/** A chart's description with every part present, or nothing if it is not one. */
function chartOf(raw: unknown): ChartSpec | undefined {
  if (typeof raw !== "object" || raw === null) return undefined;
  const given = raw as Partial<ChartSpec>;
  if (!CHARTS.includes(given.chart as ChartKind)) return undefined;
  const list = <T,>(value: T[] | undefined): T[] => (Array.isArray(value) ? value : []);
  const time =
    given.time && GRAINS.includes(given.time.grain)
      ? {
          column: given.time.column,
          grain: given.time.grain,
          range: given.time.range ?? "all",
          ...(given.time.from ? { from: given.time.from } : {}),
          ...(given.time.to ? { to: given.time.to } : {}),
        }
      : null;
  return {
    chart: given.chart as ChartKind,
    time,
    dimensions: list(given.dimensions).map((d) => ({ column: d.column })),
    compare: list(given.compare).map((d) => ({ column: d.column })),
    metrics: list(given.metrics).map((m) => ({ column: m.column ?? null, aggregate: m.aggregate })),
    filters: list(given.filters).map((f) => ({
      column: f.column,
      values: list(f.values),
      min: f.min ?? null,
      max: f.max ?? null,
      ...(f.period ? { period: { grain: f.period.grain, key: f.period.key } } : {}),
    })),
    sort: given.sort ? { metric: given.sort.metric, descending: given.sort.descending } : null,
    limit: given.limit ?? null,
  };
}

function viewsOf(table: AnalysisTable): View[] {
  return table.views.flatMap((view): View[] => {
    let spec: ChartSpec | undefined;
    try {
      spec = chartOf(JSON.parse(view.config));
    } catch {
      // Kept in a shape this app can't read: left out.
    }
    return spec ? [{ id: view.id, name: view.name, spec, by: view.by ?? undefined, savedAtMs: view.savedAtMs }] : [];
  });
}

/** What a filter keeps, in a few words. */
function filterText(filter: ChartFilter): string {
  if (filter.period) return keyName({ column: filter.column, grain: filter.period.grain }, filter.period.key);
  if (filter.values.length > 0) return filter.values.length <= 2 ? filter.values.join(", ") : x.chosenCount(filter.values.length);
  if (filter.min !== null && filter.max !== null) return `${filter.min} – ${filter.max}`;
  if (filter.min !== null) return `≥ ${filter.min}`;
  if (filter.max !== null) return `≤ ${filter.max}`;
  return x.anyValue;
}
const emptyFilter = (column: string): ChartFilter => ({ column, values: [], min: null, max: null });

/** A drill into a group, as an edit of the description. */
type Step =
  | { zoom: { column: string; grain: DateGrain; key: string } }
  | { by: string; keep: [string, string][]; period?: { column: string; grain: DateGrain; key: string } };

function drilled(spec: ChartSpec, step: Step): ChartSpec {
  // A filter replaces another on the same column.
  let filters = spec.filters;
  const only = (filter: ChartFilter) => {
    filters = [...filters.filter((f) => f.column !== filter.column), filter];
  };
  if ("zoom" in step) {
    const { column, grain, key } = step.zoom;
    only({ ...emptyFilter(column), period: { grain, key } });
    return { ...spec, filters, time: spec.time ? { ...spec.time, grain: FINER[grain] ?? grain } : null };
  }
  for (const [column, value] of step.keep) only({ ...emptyFilter(column), values: [value] });
  if (step.period) only({ ...emptyFilter(step.period.column), period: { grain: step.period.grain, key: step.period.key } });
  const metrics = spec.metrics.length > 0 ? spec.metrics : [COUNT_ROWS];
  return { ...spec, filters, metrics, time: null, dimensions: [{ column: step.by }], limit: null, sort: spec.sort ?? { metric: 0, descending: true } };
}

/** Whether a chart lays the time along its axis and the dimensions' values as series. */
const splits = (spec: ChartSpec) => !["table", "pie", "treemap"].includes(spec.chart) && spec.time !== null && spec.dimensions.length > 0;

/**
 * One chosen part of a chart (a dimension, a metric, a filter), a row of
 * its own: its name, which opens what can be chosen of it (a way to total)
 * or does `onOpen` (a filter's dialog), and a cross that removes it.
 */
function Chip({
  text,
  label,
  items,
  onOpen,
  onRemove,
  removeLabel,
}: {
  text: string;
  label: string;
  items?: MenuItem[];
  onOpen?: () => void;
  /** None when it can't be removed (the one metric of a grouped chart). */
  onRemove?: () => void;
  removeLabel: string;
}) {
  return (
    <span className="screen-explorer__chip">
      {items ? (
        <Menu label={label} trigger={{ chip: true, text }} items={items} />
      ) : (
        <button type="button" className="oik-menu__chip" title={label} aria-label={label} onClick={onOpen} disabled={!onOpen}>
          <span className="oik-menu__shown">{text}</span>
          {onOpen && <Icon name="chevron" />}
        </button>
      )}
      {onRemove && (
        <button type="button" className="screen-explorer__chip-x" aria-label={removeLabel} title={removeLabel} onClick={onRemove}>
          <Icon name="close" />
        </button>
      )}
    </span>
  );
}

/**
 * Add a metric: how it is totalled, then of which column (only those it can
 * total), with what that means. The number of rows needs no column.
 */
function AddMetric({
  table,
  taken,
  onAdd,
  onClose,
}: {
  table: AnalysisTable;
  taken: ChartMetric[];
  onAdd: (metric: ChartMetric) => void;
  onClose: () => void;
}) {
  const fields = shownFields(table);
  const numbers = fields.filter((f) => isNumber(f));
  const ways: ("rows" | ChartAggregate)[] = ["rows", ...(numbers.length > 0 ? NUMBER_AGGREGATES : OTHER_AGGREGATES)];
  const usableFor = (w: "rows" | ChartAggregate) => (w === "rows" ? [] : OTHER_AGGREGATES.includes(w) ? fields : numbers);
  const free = (w: "rows" | ChartAggregate, column: string | null) =>
    !taken.some((m) => (w === "rows" ? m.column === null : m.column === column && m.aggregate === w));
  // The first metric not in the chart yet: a sum of a number, else a count.
  type Choice = { how: "rows" | ChartAggregate; column: string | null };
  const choices: Choice[] = ways.flatMap((w): Choice[] =>
    w === "rows" ? [{ how: w, column: null }] : usableFor(w).map((f) => ({ how: w, column: f.name })),
  );
  const first = [...choices.filter((c) => c.how === "sum"), ...choices.filter((c) => c.how !== "sum")].find((c) => free(c.how, c.column));
  const [how, setHow] = useState<"rows" | ChartAggregate>(first?.how ?? "rows");
  const usable = usableFor(how);
  const [column, setColumn] = useState<string>(first?.column ?? usable[0]?.name ?? "");
  const chosen = usable.some((f) => f.name === column) ? column : (usable[0]?.name ?? "");
  const metric: ChartMetric = how === "rows" ? COUNT_ROWS : { column: chosen, aggregate: how };
  const already = taken.some((m) => m.column === metric.column && m.aggregate === metric.aggregate);
  return (
    <Dialog
      title={x.addMetricTitle}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button variant="primary" disabled={already || (how !== "rows" && !chosen)} onClick={() => onAdd(metric)}>
            {x.add}
          </Button>
        </>
      }
    >
      <div className="screen-explorer__metric-form">
        <Select
          label={x.totalledBy}
          value={how}
          onChange={(value) => setHow(value as "rows" | ChartAggregate)}
          options={ways.map((w) => ({ value: w, label: w === "rows" ? `COUNT(*) · ${x.rowCount}` : functionName(w) }))}
        />
        {how !== "rows" && (
          <Select label={x.column} value={chosen} onChange={setColumn} options={usable.map((f) => ({ value: f.name, label: f.name }))} />
        )}
        <p className="screen-help">{x.aggregateNotes[how]}</p>
        <p className="screen-explorer__will" translate="no">
          {already ? x.already : x.willAdd(metricName(metric))}
        </p>
      </div>
    </Dialog>
  );
}

/** Name a chart: saved the first time, saved as another, or renamed. */
function NameView({
  title,
  start,
  hint,
  busy,
  error,
  onSave,
  onClose,
}: {
  title: string;
  start: string;
  hint?: string;
  busy: boolean;
  error?: string;
  onSave: (name: string) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(start);
  const ready = name.trim().length > 0 && !busy;
  return (
    <Dialog
      title={title}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button variant="primary" disabled={!ready} onClick={() => onSave(name.trim())}>
            {busy ? x.saving : strings.common.save}
          </Button>
        </>
      }
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (ready) onSave(name.trim());
        }}
      >
        <TextField label={x.viewName} name="view-name" value={name} onChange={setName} hint={hint} autoFocus selectOnFocus />
      </form>
      {error && <ErrorText>{error}</ErrorText>}
    </Dialog>
  );
}

/** A filter on one column: the values to keep, ticked (text), or a range (a number). */
function FilterDialog({
  space,
  table,
  was,
  onApply,
  onClose,
}: {
  space: string;
  table: AnalysisTable;
  was: ChartFilter;
  /** The filter as set; nothing when it keeps everything. */
  onApply: (next: ChartFilter | undefined) => void;
  onClose: () => void;
}) {
  const client = useClient();
  const ranged = isNumber(fieldOf(table, was.column)) && was.values.length === 0;
  const [values, setValues] = useState(was.values);
  const [offered, setOffered] = useState<string[]>();
  const [sought, setSought] = useState("");
  const [min, setMin] = useState(was.min ?? "");
  const [max, setMax] = useState(was.max ?? "");

  useEffect(() => {
    if (ranged) return;
    let stale = false;
    client.analysisValues({ space, table: table.name, column: was.column }).then(
      (list) => !stale && setOffered(list),
      () => !stale && setOffered([]),
    );
    return () => {
      stale = true;
    };
  }, [client, space, table.name, was.column, ranged]);

  const apply = () => {
    const base = emptyFilter(was.column);
    if (!ranged) return onApply(values.length > 0 ? { ...base, values } : undefined);
    const ends = { min: min.trim() === "" ? null : min.trim(), max: max.trim() === "" ? null : max.trim() };
    return onApply(ends.min === null && ends.max === null ? undefined : { ...base, ...ends });
  };
  const all = [...new Set([...was.values, ...(offered ?? [])])];
  const found = all.filter((v) => v.toLocaleLowerCase().includes(sought.trim().toLocaleLowerCase()));
  const toggle = (value: string, on: boolean) => setValues(on ? [...values, value] : values.filter((v) => v !== value));

  return (
    <Dialog
      title={x.filterOn(was.column)}
      closeLabel={strings.common.close}
      onClose={onClose}
      placement="top"
      footer={
        <>
          {!ranged && values.length > 0 && <Button onClick={() => setValues([])}>{x.clearValues}</Button>}
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button variant="primary" onClick={apply}>
            {strings.common.done}
          </Button>
        </>
      }
    >
      {ranged ? (
        <div className="screen-explorer__range">
          {(
            [
              [x.atLeast, min, setMin],
              [x.atMost, max, setMax],
            ] as const
          ).map(([label, value, set]) => (
            <label key={label} className="oik-field">
              <span className="oik-field__label">{label}</span>
              <input className="oik-input" type="text" inputMode="decimal" value={value} placeholder={x.anyValue} onChange={(event) => set(event.target.value)} />
            </label>
          ))}
        </div>
      ) : (
        <div className="screen-explorer__filter">
          {all.length > 8 && <TextField label={x.searchValues} name="filter-search" value={sought} onChange={setSought} />}
          <p className="screen-help">{values.length > 0 ? x.chosenCount(values.length) : x.anyValue}</p>
          {offered === undefined ? (
            <div className="screen-explorer__filter-loading oik-skel" aria-hidden="true" />
          ) : found.length === 0 ? (
            <p className="screen-help">{x.noValues}</p>
          ) : (
            <ul className="screen-explorer__filter-values" aria-label={x.filterOn(was.column)}>
              {found.map((value) => (
                <li key={value}>
                  <Checkbox label={<span translate="no">{value}</span>} checked={values.includes(value)} onChange={(on) => toggle(value, on)} />
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </Dialog>
  );
}

/** The query a chart ran, for those who want to read it, with the value each placeholder is bound to. */
function QueryDialog({ query, onClose }: { query: ChartQuery; onClose: () => void }) {
  const toast = useToast();
  const copy = async () => {
    const params = query.params.map((p, i) => `-- ?${i + 1} = ${p}`).join("\n");
    try {
      await navigator.clipboard.writeText(params ? `${query.sql}\n${params}` : query.sql);
      toast(x.sqlCopied);
    } catch {
      // No clipboard here: the text can still be selected.
    }
  };
  return (
    <Dialog
      title={x.sqlTitle}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={() => void copy()}>{x.copySql}</Button>
          <Button variant="primary" onClick={onClose}>
            {strings.common.done}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{x.sqlBody}</p>
      <pre className="screen-explorer__sql" translate="no">
        {query.sql}
      </pre>
      {query.params.length > 0 && (
        <>
          <h3 className="oik-label">{x.params}</h3>
          <ol className="screen-explorer__params" translate="no">
            {query.params.map((param, i) => (
              <li key={i}>
                <span>?{i + 1}</span>
                <code>{param}</code>
              </li>
            ))}
          </ol>
        </>
      )}
    </Dialog>
  );
}

/**
 * A chart's result as a table: a column for the time and each dimension,
 * then the metrics, under each compared value when there are any. A group
 * opens what can be done with it.
 */
function ResultTable({
  result,
  metrics,
  onPick,
}: {
  result: ChartResult;
  metrics: ChartMetric[];
  onPick?: (group: number, at: { x: number; y: number }) => void;
}) {
  if (result.rows) {
    return (
      <div className="screen-explorer__table">
        <table>
          <thead>
            <tr>
              {result.rows.columns.map((name) => (
                <th key={name} translate="no">
                  {name}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {result.rows.rows.map((row, i) => (
              <tr key={i}>
                {row.map((cell, j) => (
                  <td key={j} translate="no">
                    {cell ?? ""}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }
  const columns = result.columns;
  const compared = result.series.some((s) => s.compare.length > 0);
  const many = metrics.length > 1;
  const comparedName = (values: (string | null)[]) => values.map((v) => v ?? x.blank).join(" / ");
  // Compared values: one metric over them all, or each value over its metrics.
  const values = [...new Map(result.series.map((s) => [JSON.stringify(s.compare), s.compare])).values()];
  const pads = Math.max(columns.length, 1);
  return (
    <div className="screen-explorer__table">
      <table>
        <thead>
          {compared && (
            <tr className="screen-explorer__over">
              {Array.from({ length: pads }, (_, i) => (
                <th key={i} aria-hidden="true" />
              ))}
              {many ? (
                values.map((v) => (
                  <th key={JSON.stringify(v)} colSpan={metrics.length} className="screen-explorer__num" translate="no">
                    {comparedName(v)}
                  </th>
                ))
              ) : (
                <th colSpan={result.series.length} className="screen-explorer__num" translate="no">
                  {metricName(metrics[0] ?? COUNT_ROWS)}
                </th>
              )}
            </tr>
          )}
          <tr>
            {columns.length === 0 && <th aria-hidden="true" />}
            {columns.map((c) => (
              <th key={c.column} translate={c.grain ? undefined : "no"}>
                {levelName(c)}
              </th>
            ))}
            {result.series.map((s, i) => (
              <th key={i} className="screen-explorer__num" translate="no">
                {compared && !many ? comparedName(s.compare) : metricName(metrics[s.metric] ?? COUNT_ROWS)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {result.groups.map((group, i) => {
            const pick = onPick && columns.length > 0 ? (at: { x: number; y: number }) => onPick(i, at) : undefined;
            return (
              <tr
                key={i}
                className={pick ? "screen-explorer__drillable" : undefined}
                tabIndex={pick ? 0 : undefined}
                onClick={pick ? (event) => pick({ x: event.clientX, y: event.clientY }) : undefined}
                onKeyDown={(event) => {
                  if (pick && (event.key === "Enter" || event.key === " ")) {
                    event.preventDefault();
                    const box = event.currentTarget.getBoundingClientRect();
                    pick({ x: box.left + 16, y: box.bottom });
                  }
                }}
              >
                {columns.length === 0 && <td>{x.total}</td>}
                {columns.map((c, j) => (
                  <td key={c.column} translate={c.grain ? undefined : "no"}>
                    {keyName(c, group.keys[j] ?? null)}
                    {pick && j === 0 && <Icon name="chevron" />}
                  </td>
                ))}
                {group.values.map((value, j) => (
                  <td key={j} className="screen-explorer__num">
                    {value === null ? "–" : number(value)}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

/** The chart itself, drawn by ECharts once it has loaded. */
function Drawn({ drawing, label }: { drawing: Drawing; label: string }) {
  const host = useRef<HTMLDivElement>(null);
  const chart = useRef<{ draw(d: Drawing): void; dispose(): void }>(undefined);
  const latest = useRef(drawing);
  latest.current = drawing;
  useEffect(() => {
    let gone = false;
    void import("../chartDraw").then(({ mount }) => {
      if (gone || !host.current) return;
      chart.current = mount(host.current);
      chart.current.draw(latest.current);
    });
    return () => {
      gone = true;
      chart.current?.dispose();
      chart.current = undefined;
    };
  }, []);
  useEffect(() => chart.current?.draw(drawing), [drawing]);
  return <div ref={host} className="screen-explorer__chart" role="img" aria-label={label} />;
}

/** What a chart lays along its axis: each item, the columns it stands for and its values of them. */
interface Axis {
  labels: string[];
  columns: ChartLevel[];
  keys: (string | null)[][];
  series: { name: string; values: (number | null)[] }[];
}

/** A chart's result as what is drawn: along the axis, the time (its dimensions' values then series) or the groups. */
function axisOf(result: ChartResult, spec: ChartSpec, metrics: ChartMetric[]): Axis {
  const many = metrics.length > 1;
  const named = (parts: string[], metric: ChartMetric) =>
    parts.length === 0 ? metricName(metric) : many ? `${metricName(metric)}, ${parts.join(" / ")}` : parts.join(" / ");
  const shown = (values: (string | null)[]) => values.map((v) => v ?? x.blank);
  if (splits(spec) && result.columns.length > 1) {
    const time = result.columns[0]!;
    const periods = [...new Set(result.groups.map((g) => g.keys[0] ?? null))];
    const combos = [...new Map(result.groups.map((g) => [JSON.stringify(g.keys.slice(1)), g.keys.slice(1)])).values()].sort((a, b) =>
      shown(a).join(" / ").localeCompare(shown(b).join(" / "), locale(), { numeric: true }),
    );
    const at = new Map(result.groups.map((g) => [JSON.stringify(g.keys), g]));
    return {
      labels: periods.map((p) => keyName(time, p)),
      columns: [time],
      keys: periods.map((p) => [p]),
      series: combos.flatMap((combo) =>
        result.series.map((s, i) => ({
          name: named([...shown(combo), ...shown(s.compare)], metrics[s.metric] ?? COUNT_ROWS),
          values: periods.map((p) => at.get(JSON.stringify([p, ...combo]))?.values[i] ?? null),
        })),
      ),
    };
  }
  return {
    labels: result.groups.map((g) => (result.columns.length === 0 ? x.total : result.columns.map((c, j) => keyName(c, g.keys[j] ?? null)).join(" / "))),
    columns: result.columns,
    keys: result.groups.map((g) => g.keys),
    series: result.series.map((s, i) => ({
      name: named(shown(s.compare), metrics[s.metric] ?? COUNT_ROWS),
      values: result.groups.map((g) => g.values[i] ?? null),
    })),
  };
}

export function Explorer({
  space,
  table,
  onChange,
}: {
  space: string;
  table: AnalysisTable;
  onChange: (next: FileAnalysis) => void;
}) {
  const client = useClient();
  const toast = useToast();
  const saving = useAction();
  const removing = useAction();
  const [picked, setPicked] = useState<string>();
  // What the owner made of the chart shown, not saved yet; and the drills that led there.
  const [draft, setDraft] = useState<{ of: string; spec: ChartSpec }>();
  const [trail, setTrail] = useState<{ of: string; steps: { label: string; before: ChartSpec }[] }>();
  const [result, setResult] = useState<ChartResult>();
  const [failed, setFailed] = useState<string>();
  const [dialog, setDialog] = useState<"saveNew" | "saveAs" | "rename" | "remove" | "sql" | "metric">();
  const [filtering, setFiltering] = useState<ChartFilter>();
  const [pointed, setPointed] = useState<{ at: { x: number; y: number }; items: MenuItem[]; label: string }>();

  const views = useMemo(() => viewsOf(table), [table]);
  // The chart picked; else the first kept; else a new one.
  const kept = picked === NEW ? undefined : (views.find((v) => v.id === picked) ?? views[0]);
  const started = useMemo(() => newChart(table), [table]);
  const view: View = kept ?? { id: NEW, name: x.untitled, spec: started };
  const fresh = view.id === NEW;
  const spec = draft?.of === view.id ? draft.spec : view.spec;
  const edited = JSON.stringify(spec) !== JSON.stringify(view.spec);
  const steps = trail?.of === view.id ? trail.steps : [];
  /** The description as changed: made whole (an order and a number for dimensions; rows counted when grouped). */
  const settle = (made: ChartSpec): ChartSpec => {
    const next = { ...made };
    if (next.dimensions.length === 0) next.sort = null;
    if (next.time !== null || next.dimensions.length === 0) next.limit = null;
    const grouped = next.time !== null || next.dimensions.length > 0 || next.compare.length > 0;
    if (grouped && next.metrics.length === 0) next.metrics = [COUNT_ROWS];
    if (next.sort && next.sort.metric >= next.metrics.length) next.sort = next.metrics.length > 0 ? { ...next.sort, metric: 0 } : null;
    return next;
  };
  const show = (next: ChartSpec) => setDraft(JSON.stringify(next) === JSON.stringify(view.spec) ? undefined : { of: view.id, spec: next });
  const change = (next: Partial<ChartSpec>) => {
    show(settle({ ...spec, ...next }));
    setTrail(undefined);
  };
  const open = (id: string) => {
    setPicked(id);
    setTrail(undefined);
  };

  // What the chart shows, asked of the node whenever its description or the
  // table changes; an answer that arrives late is dropped.
  const asked = JSON.stringify(spec);
  useEffect(() => {
    let stale = false;
    client.analysisChart({ space, table: table.name, spec: JSON.parse(asked) as ChartSpec }).then(
      (answer) => {
        if (stale) return;
        setResult(answer);
        setFailed(undefined);
      },
      (error: unknown) => {
        if (!stale) setFailed(error instanceof Error ? error.message : String(error));
      },
    );
    return () => {
      stale = true;
    };
  }, [client, space, table.name, table.builtAtMs, asked]);

  // Once a chart is saved, its draft is done with: that chart's only. By
  // then the owner may be changing another, whose draft stays.
  const forget = (id: string) => {
    setDraft((was) => (was?.of === id ? undefined : was));
    setTrail(undefined);
  };
  /** Save a kept chart where it is: nothing to name, nothing to choose. */
  const saveHere = async () => {
    const of = view.id;
    const next = await saving.run(() => client.saveAnalysisView({ space, table: table.name, id: view.id, name: view.name, config: JSON.stringify(spec) }));
    if (!next) return;
    onChange(next);
    forget(of);
    toast(x.savedNow);
  };
  /** Keep what is shown as a chart of its own, under a name: a new chart's first save, or another. */
  const saveAs = async (name: string) => {
    const of = view.id;
    const next = await saving.run(() => client.saveAnalysisView({ space, table: table.name, name, config: JSON.stringify(spec) }));
    if (!next) return;
    setDialog(undefined);
    onChange(next);
    forget(of);
    setPicked(next.tables.find((t) => t.name === table.name)?.views.find((v) => v.name === name)?.id);
    toast(x.saved(name));
  };
  const rename = async (name: string) => {
    const next = await saving.run(() => client.saveAnalysisView({ space, table: table.name, id: view.id, name, config: JSON.stringify(view.spec) }));
    if (!next) return;
    setDialog(undefined);
    onChange(next);
  };
  const when = (ms: number) => new Intl.DateTimeFormat(locale(), { dateStyle: "medium" }).format(new Date(ms));

  // ---- What the panel offers ----
  const fields = shownFields(table);
  const times = fields.filter((f) => isTime(f));
  const grouped = spec.time !== null || spec.dimensions.length > 0;
  const metrics: ChartMetric[] = spec.metrics.length > 0 || (!grouped && spec.compare.length === 0) ? spec.metrics : [COUNT_ROWS];
  // Any column is a dimension or compared, once; the chart's time is chosen above.
  const used = [...(spec.time ? [spec.time.column] : []), ...spec.dimensions.map((d) => d.column), ...spec.compare.map((d) => d.column)];
  const offered = (list: "dimensions" | "compare") =>
    fields
      .filter((f) => !used.includes(f.name))
      .map((f): MenuItem => ({ label: f.name, onSelect: () => change({ [list]: [...spec[list], { column: f.name }] }) }));
  const filterItems: MenuItem[] = fields
    .filter((f) => !isTime(f) && !spec.filters.some((filter) => filter.column === f.name))
    .map((f) => ({ label: f.name, onSelect: () => setFiltering(emptyFilter(f.name)) }));
  const dimensionChips = (list: "dimensions" | "compare") =>
    spec[list].map((d: ChartDimension, at) => (
      <Chip
        key={d.column}
        text={d.column}
        label={d.column}
        removeLabel={x.removeItem(d.column)}
        onRemove={() => change({ [list]: spec[list].filter((_, i) => i !== at) })}
      />
    ));
  const adding = (label: string, items: MenuItem[]) =>
    items.length > 0 ? <Menu label={label} trigger={{ icon: "plus", text: label }} items={items} /> : <p className="screen-explorer__none">{x.allUsed}</p>;
  // A column chosen as the time leaves the dimensions: it is grouped once, at its grain.
  const setTime = (next: ChartTime | null) =>
    change({
      time: next,
      dimensions: spec.dimensions.filter((d) => d.column !== next?.column),
      compare: spec.compare.filter((d) => d.column !== next?.column),
    });

  // ---- What is drawn, and what a click on a group does ----
  const axis = useMemo(() => (result && !result.rows ? axisOf(result, spec, metrics) : undefined), [result, spec, metrics]);
  /** Step down into a group: an edit, the chart it came from kept to step back to. */
  const drill = (label: string, step: Step) => {
    show(settle(drilled(spec, step)));
    setTrail({ of: view.id, steps: [...steps, { label, before: spec }] });
  };
  const back = (to: number) => {
    show(steps[to]!.before);
    setTrail(to === 0 ? undefined : { of: view.id, steps: steps.slice(0, to) });
  };
  /** What a group of the chart can be: a period zoomed into, or broken down by a column not in use. */
  const pointAt = (columns: ChartLevel[], keys: (string | null)[], at: { x: number; y: number }) => {
    const items: MenuItem[] = [];
    const t = columns.findIndex((c) => c.grain);
    const period = t >= 0 && keys[t] !== null ? { column: columns[t]!.column, grain: columns[t]!.grain!, key: keys[t]! } : undefined;
    if (period && FINER[period.grain]) {
      const name = keyName(columns[t], period.key);
      items.push({ label: x.zoomInto(name), onSelect: () => drill(name, { zoom: period }) });
    }
    const keep = columns.flatMap((c, j): [string, string][] => (c.grain || keys[j] === null ? [] : [[c.column, keys[j]!]]));
    const label = columns.map((c, j) => keyName(c, keys[j] ?? null)).join(" / ");
    for (const f of fields.filter((f) => !used.includes(f.name))) {
      items.push({ label: x.breakDownBy(f.name), onSelect: () => drill(x.drillStep(label, f.name), { by: f.name, keep, period }) });
    }
    if (items.length > 0) setPointed({ at, items, label });
  };
  const clickable = Boolean(axis && axis.columns.length > 0);
  // The grain a click on a period zooms into, when the time is drawn and can go finer.
  const finer = axis?.columns.some((c) => c.grain) && spec.time ? FINER[spec.time.grain] : undefined;
  const named = axis ? JSON.stringify(axis) : "";
  const drawing = useMemo((): Drawing | undefined => {
    if (!axis || spec.chart === "table") return undefined;
    return {
      kind: spec.chart,
      groups: axis.labels,
      series: axis.series,
      valueName: metrics.length === 1 ? metricName(metrics[0]!) : undefined,
      format: number,
      onPick: clickable ? (i, at) => pointAt(axis.columns, axis.keys[i] ?? [], at) : undefined,
    };
    // `pointAt` stands for the chart shown, which `axis` follows.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [named, spec.chart, clickable]);
  const empty = result !== undefined && (result.rows ? result.rows.rows.length === 0 : result.groups.length === 0);
  const facts = result
    ? [
        result.matched === table.rows ? undefined : x.matched(result.matched, table.rows),
        result.rows && result.truncated ? x.firstRows(result.rows.rows.length) : undefined,
        !result.rows && result.truncated ? x.firstGroups(result.groups.length) : undefined,
      ].filter((fact): fact is string => fact !== undefined)
    : [];
  const sortValue = spec.sort ? `${spec.sort.metric}:${spec.sort.descending ? "desc" : "asc"}` : "";

  return (
    <>
      <Card className="screen-explorer">
        <div className="screen-explorer__head">
          <div className="screen-explorer__views" role="group" aria-label={x.views}>
            {views.map((v) =>
              v.id === view.id ? (
                // The chart shown: its tab holds what can be done with it.
                <span key={v.id} className="screen-explorer__view screen-explorer__view--here" translate="no">
                  <Menu
                    label={x.chartActions(v.name)}
                    trigger={{ chip: true, text: v.name }}
                    items={[
                      { label: x.rename, onSelect: () => setDialog("rename") },
                      {
                        label: x.saveAs,
                        onSelect: () => {
                          saving.clear();
                          setDialog("saveAs");
                        },
                      },
                      { label: x.remove, onSelect: () => setDialog("remove") },
                    ]}
                  />
                </span>
              ) : (
                <button key={v.id} type="button" className="screen-explorer__view" title={v.by ? x.madeByShort(v.by) : undefined} onClick={() => open(v.id)}>
                  {v.by && <Icon name="ai" />}
                  <span translate="no">{v.name}</span>
                </button>
              ),
            )}
            {fresh ? (
              // The chart being made, not kept yet: named when saved, which a click on it starts.
              <button
                type="button"
                className="screen-explorer__view screen-explorer__view--untitled"
                aria-current="true"
                title={x.nameToSave}
                onClick={() => {
                  saving.clear();
                  setDialog("saveNew");
                }}
              >
                {x.untitled}
              </button>
            ) : (
              <button type="button" className="screen-explorer__view screen-explorer__view--add" onClick={() => open(NEW)}>
                <Icon name="plus" />
                <span>{x.newChart}</span>
              </button>
            )}
          </div>
          {(edited || fresh) && (
            <div className="screen-explorer__actions">
              {edited && <Tag>{fresh ? x.unsaved : x.edited}</Tag>}
              {edited && <Button onClick={() => forget(view.id)}>{x.discard}</Button>}
              {fresh ? (
                <Button
                  variant="primary"
                  onClick={() => {
                    saving.clear();
                    setDialog("saveNew");
                  }}
                >
                  {x.saveDots}
                </Button>
              ) : (
                <Button variant="primary" disabled={saving.busy} onClick={() => void saveHere()}>
                  {saving.busy ? x.saving : strings.common.save}
                </Button>
              )}
            </div>
          )}
        </div>
        {view.by && view.savedAtMs !== undefined && (
          <p className="screen-explorer__by">
            <Icon name="ai" />
            <span>{x.madeBy(view.by, when(view.savedAtMs))}</span>
          </p>
        )}
        {saving.error && dialog === undefined && (
          <div className="screen-explorer__failed">
            <ErrorText>{saving.error}</ErrorText>
          </div>
        )}
        <div className="screen-explorer__body">
          <div className="screen-explorer__stage" aria-busy={result === undefined && failed === undefined}>
            <div className="screen-explorer__where">
              <div className="screen-explorer__whereabouts">
                {steps.length > 0 && (
                  <nav className="screen-explorer__path" aria-label={x.drillPath}>
                    {[x.drillAll, ...steps.map((s) => s.label)].map((name, j) =>
                      j < steps.length ? (
                        <span key={j} className="screen-explorer__step">
                          <button type="button" onClick={() => back(j)}>
                            {name}
                          </button>
                          <Icon name="chevron" />
                        </span>
                      ) : (
                        <span key={j} className="screen-explorer__step screen-explorer__step--here" aria-current="location" translate="no">
                          {name}
                        </span>
                      ),
                    )}
                  </nav>
                )}
                {facts.length > 0 && (
                  <span className="screen-explorer__facts">
                    {facts.map((fact) => (
                      <span key={fact}>{fact}</span>
                    ))}
                  </span>
                )}
              </div>
              <div className="screen-explorer__tools">
                {clickable && (
                  <span className="screen-explorer__hint">
                    <Icon name="info" />
                    <span>{finer ? x.drillHintZoom(x.grains[finer] ?? finer) : x.drillHint}</span>
                  </span>
                )}
                <Button className="screen-explorer__sql-button" disabled={!result} onClick={() => setDialog("sql")}>
                  {x.showSql}
                </Button>
              </div>
            </div>
            {failed ? (
              <ErrorText>
                {x.failed} {failed}
              </ErrorText>
            ) : result === undefined ? (
              <div className="screen-explorer__loading oik-skel" role="status" aria-label={x.loading} />
            ) : empty ? (
              <p className="screen-explorer__empty">{x.noRows}</p>
            ) : drawing ? (
              <Drawn drawing={drawing} label={view.name} />
            ) : (
              <ResultTable
                result={result}
                metrics={metrics}
                onPick={clickable ? (i, at) => pointAt(result.columns, result.groups[i]?.keys ?? [], at) : undefined}
              />
            )}
          </div>
          <div className="screen-explorer__panel" role="group" aria-label={x.settings}>
            <Select
              label={x.chartType}
              value={spec.chart}
              onChange={(chart) => change({ chart: chart as ChartKind })}
              options={CHARTS.map((chart) => ({ value: chart, label: x.charts[chart] ?? chart }))}
            />
            {times.length > 0 && (
              <div className="screen-explorer__part" role="group" aria-label={x.time}>
                <Select
                  label={x.time}
                  value={spec.time?.column ?? ""}
                  onChange={(column) => setTime(column === "" ? null : { column, grain: spec.time?.grain ?? "month", range: spec.time?.range ?? "all" })}
                  options={[{ value: "", label: x.noTime }, ...times.map((f) => ({ value: f.name, label: f.name }))]}
                />
                {spec.time && (
                  <div className="screen-explorer__sub">
                    {/* A grain is picked from a list, as BI tools do: five buttons don't fit the panel. */}
                    <Select
                      label={x.grain}
                      value={spec.time.grain}
                      onChange={(grain) => setTime({ ...spec.time!, grain: grain as DateGrain })}
                      options={GRAINS.map((g) => ({ value: g, label: x.grains[g] ?? g }))}
                    />
                    <Select
                      label={x.range}
                      value={spec.time.range}
                      onChange={(range) => {
                        // A range of its own keeps its days; another has none.
                        const { from: _from, to: _to, ...rest } = spec.time!;
                        setTime(range === "custom" ? { ...spec.time!, range: "custom" } : { ...rest, range: range as TimeRange });
                      }}
                      options={RANGES.map((r) => ({ value: r, label: x.ranges[r] ?? r }))}
                    />
                    {spec.time.range === "custom" && (
                      <div className="screen-explorer__days">
                        {(
                          [
                            ["from", x.rangeFrom],
                            ["to", x.rangeTo],
                          ] as const
                        ).map(([end, label]) => (
                          <label key={end} className="oik-field">
                            <span className="oik-field__label">{label}</span>
                            <input
                              className="oik-input"
                              type="date"
                              value={spec.time![end] ?? ""}
                              min={end === "to" ? spec.time!.from : undefined}
                              max={end === "from" ? spec.time!.to : undefined}
                              onChange={(event) => {
                                const { [end]: _was, ...rest } = spec.time!;
                                setTime(event.target.value ? { ...spec.time!, [end]: event.target.value } : rest);
                              }}
                            />
                          </label>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            )}
            <div className="screen-explorer__part" role="group" aria-label={x.dimensions}>
              <span className="oik-field__label">
                {x.dimensions}
                {splits(spec) && <span className="screen-explorer__note">{x.dimsSeries}</span>}
              </span>
              {dimensionChips("dimensions")}
              {adding(x.addDimension, offered("dimensions"))}
            </div>
            <div className="screen-explorer__part" role="group" aria-label={x.compare}>
              <span className="oik-field__label">{x.compare}</span>
              {dimensionChips("compare")}
              {adding(x.addComparison, offered("compare"))}
            </div>
            <div className="screen-explorer__part" role="group" aria-label={x.metrics}>
              <span className="oik-field__label">{x.metrics}</span>
              {spec.metrics.map((m, at) => (
                <Chip
                  key={`${m.column ?? ""}:${m.aggregate}`}
                  text={metricName(m)}
                  label={m.column ? x.aggregateOf(m.column) : x.rowCount}
                  items={
                    m.column
                      ? (isNumber(fieldOf(table, m.column)) ? NUMBER_AGGREGATES : OTHER_AGGREGATES)
                          .filter((a) => a === m.aggregate || !spec.metrics.some((o) => o.column === m.column && o.aggregate === a))
                          .map((a) => ({
                            label: functionName(a),
                            checked: a === m.aggregate,
                            onSelect: () => change({ metrics: spec.metrics.map((o, i) => (i === at ? { ...o, aggregate: a } : o)) }),
                          }))
                      : undefined
                  }
                  removeLabel={x.removeItem(metricName(m))}
                  onRemove={
                    spec.metrics.length === 1 && m.column === null && (grouped || spec.compare.length > 0)
                      ? undefined
                      : () => change({ metrics: spec.metrics.filter((_, i) => i !== at) })
                  }
                />
              ))}
              <Button icon="plus" className="screen-explorer__add" onClick={() => setDialog("metric")}>
                {x.addMetric}
              </Button>
            </div>
            <div className="screen-explorer__part" role="group" aria-label={x.filter}>
              <span className="oik-field__label">{x.filter}</span>
              {spec.filters.map((filter, at) => (
                <Chip
                  key={filter.column}
                  text={x.filterChip(filter.column, filterText(filter))}
                  label={x.filterOn(filter.column)}
                  // A period kept by a zoom or a breakdown is removed, not edited.
                  onOpen={filter.period ? undefined : () => setFiltering(filter)}
                  removeLabel={x.removeItem(filter.column)}
                  onRemove={() => change({ filters: spec.filters.filter((_, i) => i !== at) })}
                />
              ))}
              {adding(x.addFilter, filterItems)}
            </div>
            {spec.dimensions.length > 0 && (
              <Select
                label={x.order}
                value={sortValue}
                onChange={(value) => {
                  const [metric, direction] = value.split(":");
                  change({ sort: value === "" ? null : { metric: Number(metric), descending: direction === "desc" } });
                }}
                options={[
                  { value: "", label: x.orderDefault },
                  ...metrics.flatMap((m, i) => [
                    { value: `${i}:desc`, label: x.orderBy(metricName(m), x.descending) },
                    { value: `${i}:asc`, label: x.orderBy(metricName(m), x.ascending) },
                  ]),
                ]}
              />
            )}
            {spec.dimensions.length > 0 && spec.time === null && (
              <Select
                label={x.show}
                value={spec.limit === null ? "" : String(spec.limit)}
                onChange={(value) => change({ limit: value === "" ? null : Number(value) })}
                options={[
                  { value: "", label: x.showAll },
                  ...[...new Set([...LIMITS, ...(spec.limit === null ? [] : [spec.limit])])].sort((a, b) => a - b).map((n) => ({ value: String(n), label: x.showTop(n) })),
                ]}
              />
            )}
          </div>
        </div>
      </Card>
      {/* Outside the card: a dialog is laid out by the page, not by the card's own rules. */}
      {pointed && <PointMenu label={pointed.label} at={pointed.at} items={pointed.items} onClose={() => setPointed(undefined)} />}
      {dialog === "sql" && result && <QueryDialog query={result.query} onClose={() => setDialog(undefined)} />}
      {dialog === "metric" && (
        <AddMetric
          table={table}
          taken={spec.metrics}
          onClose={() => setDialog(undefined)}
          onAdd={(metric) => {
            setDialog(undefined);
            change({ metrics: [...spec.metrics, metric] });
          }}
        />
      )}
      {filtering && (
        <FilterDialog
          space={space}
          table={table}
          was={filtering}
          onClose={() => setFiltering(undefined)}
          onApply={(next) => {
            const others = spec.filters.filter((f) => f.column !== filtering.column);
            const at = spec.filters.findIndex((f) => f.column === filtering.column);
            const filters = next === undefined ? others : at < 0 ? [...spec.filters, next] : spec.filters.map((f, i) => (i === at ? next : f));
            setFiltering(undefined);
            change({ filters });
          }}
        />
      )}
      {(dialog === "saveNew" || dialog === "saveAs") && (
        <NameView
          title={dialog === "saveNew" ? x.saveFirstTitle : x.saveTitle}
          start=""
          hint={x.saveHint}
          busy={saving.busy}
          error={saving.error}
          onSave={(name) => void saveAs(name)}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog === "rename" && (
        <NameView title={x.renameTitle} start={view.name} busy={saving.busy} error={saving.error} onSave={(name) => void rename(name)} onClose={() => setDialog(undefined)} />
      )}
      {dialog === "remove" && (
        <ConfirmDialog
          title={x.removeTitle(view.name)}
          body={x.removeBody}
          confirmLabel={removing.busy ? x.removing : x.removeYes}
          cancelLabel={strings.common.cancel}
          busy={removing.busy}
          error={removing.error}
          onCancel={() => setDialog(undefined)}
          onConfirm={async () => {
            const next = await removing.run(() => client.removeAnalysisView({ space, table: table.name, id: view.id }));
            if (!next) return;
            setDialog(undefined);
            setPicked(undefined);
            forget(view.id);
            onChange(next);
            toast(x.removed(view.name));
          }}
        />
      )}
    </>
  );
}
