// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The explorer of an analysis table: its saved charts in a row, the one
// shown, and beside it the panel that describes it the way OLAP tools do.
//
// A chart is a description, never SQL: its dimensions (the columns its
// rows are grouped by; a column of time at a grain), what is compared
// within each group (further dimensions, drawn as series), its metrics (a
// column and how it is totalled, the same column as often as wanted, or
// the number of rows), its filters, its order and how many groups it
// shows. The node writes the query from the description and answers with
// the result; this component draws that result (a table here, the charts
// with ECharts, loaded on demand). Nothing is computed in the browser.
//
// A table starts with the charts its columns' roles give (by month, by
// category). What the owner changes is a draft of the chart shown, until
// Save keeps it where it is (a starting chart too, which can be restored),
// or it is saved as another chart under a name, or discarded. Assistants
// allowed to read the collection may save charts too; those say who made
// them.

import { useEffect, useMemo, useRef, useState } from "react";
import { Button, Card, Checkbox, ConfirmDialog, Dialog, ErrorText, Icon, IconButton, Menu, Select, Tag, TextField, useToast } from "@oikome/ui";
import type { MenuItem } from "@oikome/ui";
import type {
  AnalysisTable,
  ChartAggregate,
  ChartDimension,
  ChartFilter,
  ChartKind,
  ChartMetric,
  ChartResult,
  ChartSpec,
  DateGrain,
  FileAnalysis,
} from "@oikome/client";
import { useAction, useClient } from "../data";
import { strings } from "../strings";
import { locale } from "../i18n/languages";
import type { Drawing } from "../chartDraw";

const x = strings.explorer;

const CHARTS: ChartKind[] = ["table", "column", "bar", "line", "area", "treemap"];
const GRAINS: DateGrain[] = ["year", "quarter", "month", "week", "day"];
const NUMBER_AGGREGATES: ChartAggregate[] = ["sum", "average", "median", "min", "max", "count", "distinct"];
const OTHER_AGGREGATES: ChartAggregate[] = ["count", "distinct"];
const LIMITS = [5, 10, 20, 50];
/** The ids of the charts a table starts with, as the node knows them. */
const STARTING = ["month", "category", "rows"];

interface View {
  id: string;
  name: string;
  spec: ChartSpec;
  /** One the table starts with: not renamed or removed. */
  builtin: boolean;
  /** A starting chart the owner saved changes to: it can be restored. */
  changed?: boolean;
  by?: string;
  savedAtMs?: number;
}

type Field = AnalysisTable["fields"][number];
const isNumber = (field: Field | undefined) => field !== undefined && /^(U?Int|Float|Decimal)/.test(field.dataType);
/** A column of time: by its type, or because the owner said so (the node agrees). */
const isTime = (field: Field | undefined) => field !== undefined && (/^(Date|Timestamp)/.test(field.dataType) || field.role === "time");
const shownFields = (table: AnalysisTable) => table.fields.filter((f) => f.name !== "_source_file");
const fieldOf = (table: AnalysisTable, name: string | null) => table.fields.find((f) => f.name === name);

const number = (value: number) => new Intl.NumberFormat(locale(), { maximumFractionDigits: 2 }).format(value);
const dimensionName = (d: ChartDimension) => (d.grain ? `${d.column} · ${x.grains[d.grain] ?? d.grain}` : d.column);
const metricName = (m: ChartMetric) => (m.column ? x.metricOf(x.aggregates[m.aggregate] ?? m.aggregate, m.column) : x.rowCount);

/** A chart's description with every part present, or nothing if it is not one. */
function chartOf(raw: unknown): ChartSpec | undefined {
  if (typeof raw !== "object" || raw === null) return undefined;
  const given = raw as Partial<ChartSpec>;
  if (!CHARTS.includes(given.chart as ChartKind)) return undefined;
  const list = <T,>(value: T[] | undefined): T[] => (Array.isArray(value) ? value : []);
  return {
    chart: given.chart as ChartKind,
    dimensions: list(given.dimensions).map((d) => ({ column: d.column, grain: d.grain ?? null })),
    compare: list(given.compare).map((d) => ({ column: d.column, grain: d.grain ?? null })),
    metrics: list(given.metrics).map((m) => ({ column: m.column ?? null, aggregate: m.aggregate })),
    filters: list(given.filters).map((f) => ({
      column: f.column,
      values: list(f.values),
      min: f.min ?? null,
      max: f.max ?? null,
      lastDays: f.lastDays ?? null,
      thisYear: f.thisYear === true,
    })),
    sort: given.sort ? { metric: given.sort.metric, descending: given.sort.descending } : null,
    limit: given.limit ?? null,
  };
}

/** The charts a table starts with, from what its columns are for. */
function starting(table: AnalysisTable): View[] {
  const of = (role: string) => table.fields.find((f) => f.role === role)?.name;
  const [time, amount, category] = [of("time"), of("amount"), of("category")];
  // Without an amount, rows are counted.
  const metrics: ChartMetric[] = [amount ? { column: amount, aggregate: "sum" } : { column: null, aggregate: "count" }];
  const base = { compare: [], metrics, filters: [], sort: null, limit: null };
  const out: View[] = [];
  if (time) {
    out.push({ id: "month", name: x.byMonth, builtin: true, spec: { ...base, chart: "column", dimensions: [{ column: time, grain: "month" }] } });
  }
  if (category) {
    out.push({
      id: "category",
      name: x.byCategory,
      builtin: true,
      spec: { ...base, chart: "bar", dimensions: [{ column: category, grain: null }], sort: { metric: 0, descending: true } },
    });
  }
  if (out.length === 0) out.push({ id: "rows", name: x.allRows, builtin: true, spec: { ...base, chart: "table", dimensions: [], metrics: [] } });
  return out;
}

function viewsOf(table: AnalysisTable): View[] {
  const kept = table.views.flatMap((view): View[] => {
    let spec: ChartSpec | undefined;
    try {
      spec = chartOf(JSON.parse(view.config));
    } catch {
      // Kept in a shape this app can't read: left out.
    }
    return spec ? [{ id: view.id, name: view.name, spec, builtin: false, by: view.by ?? undefined, savedAtMs: view.savedAtMs }] : [];
  });
  // A starting chart saved in place is kept under its own id, and shown in
  // place of what the columns' roles give.
  const first = starting(table).map((v): View => {
    const own = kept.find((k) => k.id === v.id);
    return own ? { ...v, spec: own.spec, changed: true } : v;
  });
  return [...first, ...kept.filter((k) => !STARTING.includes(k.id))];
}

/** What a filter keeps, in a few words. */
function filterText(filter: ChartFilter): string {
  if (filter.values.length > 0) return filter.values.length <= 2 ? filter.values.join(", ") : x.chosenCount(filter.values.length);
  if (filter.lastDays !== null) return x.lastDays(filter.lastDays);
  if (filter.thisYear) return x.thisYear;
  if (filter.min !== null && filter.max !== null) return `${filter.min} – ${filter.max}`;
  if (filter.min !== null) return `≥ ${filter.min}`;
  if (filter.max !== null) return `≤ ${filter.max}`;
  return x.anyValue;
}
const emptyFilter = (column: string): ChartFilter => ({ column, values: [], min: null, max: null, lastDays: null, thisYear: false });

/** Name a chart: a new one saved, or one renamed. */
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

/**
 * A filter on one column: the values to keep, ticked (text); a range with
 * either end open (a number); a period (a column of time).
 */
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
  const field = fieldOf(table, was.column);
  const kind = isNumber(field) ? "number" : isTime(field) ? "time" : "text";
  const [values, setValues] = useState(was.values);
  const [offered, setOffered] = useState<string[]>();
  const [sought, setSought] = useState("");
  const [min, setMin] = useState(was.min ?? "");
  const [max, setMax] = useState(was.max ?? "");
  const [period, setPeriod] = useState(was.thisYear ? "year" : was.lastDays !== null ? String(was.lastDays) : was.min !== null || was.max !== null ? "range" : "90");

  useEffect(() => {
    if (kind !== "text") return;
    let stale = false;
    client.analysisValues({ space, table: table.name, column: was.column }).then(
      (list) => !stale && setOffered(list),
      () => !stale && setOffered([]),
    );
    return () => {
      stale = true;
    };
  }, [client, space, table.name, was.column, kind]);

  const apply = () => {
    const base = emptyFilter(was.column);
    if (kind === "text") return onApply(values.length > 0 ? { ...base, values } : undefined);
    const ends = { min: min.trim() === "" ? null : min.trim(), max: max.trim() === "" ? null : max.trim() };
    if (kind === "number" || period === "range") return onApply(ends.min === null && ends.max === null ? undefined : { ...base, ...ends });
    return onApply(period === "year" ? { ...base, thisYear: true } : { ...base, lastDays: Number(period) });
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
          {kind === "text" && values.length > 0 && <Button onClick={() => setValues([])}>{x.clearValues}</Button>}
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button variant="primary" onClick={apply}>
            {strings.common.done}
          </Button>
        </>
      }
    >
      {kind === "text" && (
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
      {kind === "time" && (
        <div className="screen-explorer__filter">
          <Select
            label={x.period}
            value={period}
            onChange={setPeriod}
            options={[
              { value: "30", label: x.lastDays(30) },
              { value: "90", label: x.lastDays(90) },
              { value: "365", label: x.lastDays(365) },
              { value: "year", label: x.thisYear },
              { value: "range", label: x.dateRange },
            ]}
          />
        </div>
      )}
      {(kind === "number" || (kind === "time" && period === "range")) && (
        <div className="screen-explorer__range">
          {(
            [
              [kind === "number" ? x.atLeast : x.from, min, setMin],
              [kind === "number" ? x.atMost : x.to, max, setMax],
            ] as const
          ).map(([label, value, set]) => (
            <label key={label} className="oik-field">
              <span className="oik-field__label">{label}</span>
              <input
                className="oik-input"
                type={kind === "number" ? "text" : "date"}
                inputMode={kind === "number" ? "decimal" : undefined}
                value={value}
                onChange={(event) => set(event.target.value)}
              />
            </label>
          ))}
        </div>
      )}
    </Dialog>
  );
}

/** A chart's result as a table: a column for each dimension, then one for each series. */
function ResultTable({ spec, result, seriesNames }: { spec: ChartSpec; result: ChartResult; seriesNames: string[] }) {
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
  return (
    <div className="screen-explorer__table">
      <table>
        <thead>
          <tr>
            {spec.dimensions.length === 0 ? <th /> : spec.dimensions.map((d, i) => <th key={i}>{dimensionName(d)}</th>)}
            {seriesNames.map((name, i) => (
              <th key={i} className="screen-explorer__num">
                {name}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {result.groups.map((group, i) => (
            <tr key={i}>
              {spec.dimensions.length === 0 ? (
                <td>{x.total}</td>
              ) : (
                group.keys.map((key, j) => (
                  <td key={j} translate="no">
                    {key ?? x.blank}
                  </td>
                ))
              )}
              {group.values.map((value, j) => (
                <td key={j} className="screen-explorer__num">
                  {value === null ? "–" : number(value)}
                </td>
              ))}
            </tr>
          ))}
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
  // What the owner made of the chart shown, not saved yet.
  const [draft, setDraft] = useState<{ of: string; spec: ChartSpec }>();
  const [result, setResult] = useState<ChartResult>();
  const [failed, setFailed] = useState<string>();
  const [dialog, setDialog] = useState<"save" | "rename" | "remove">();
  const [filtering, setFiltering] = useState<ChartFilter>();

  const views = useMemo(() => viewsOf(table), [table]);
  const view = views.find((v) => v.id === picked) ?? views[0]!;
  const spec = draft?.of === view.id ? draft.spec : view.spec;
  const edited = JSON.stringify(spec) !== JSON.stringify(view.spec);
  const change = (next: Partial<ChartSpec>) => {
    const made = { ...spec, ...next };
    // The order and the number shown are of the groups: none without a dimension.
    if (made.dimensions.length === 0) Object.assign(made, { sort: null, limit: null });
    if (made.sort && made.sort.metric >= Math.max(made.metrics.length, 1)) made.sort = made.metrics.length > 0 ? { ...made.sort, metric: 0 } : null;
    setDraft({ of: view.id, spec: made });
  };

  // What the chart shows, asked of the node whenever its description (or
  // the table) changes; an answer that arrives late is dropped.
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
  const forget = (id: string) => setDraft((was) => (was?.of === id ? undefined : was));
  /** Save the chart shown, where it is: no name to give, nothing to choose. */
  const saveHere = async () => {
    const of = view.id;
    const next = await saving.run(() =>
      client.saveAnalysisView({ space, table: table.name, id: view.id, name: view.builtin ? view.id : view.name, config: JSON.stringify(spec) }),
    );
    if (!next) return;
    onChange(next);
    forget(of);
    toast(x.savedNow);
  };
  /** Keep what is shown as another chart, under a name. */
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
    const next = await saving.run(() =>
      client.saveAnalysisView({ space, table: table.name, id: view.id, name, config: JSON.stringify(view.spec) }),
    );
    if (!next) return;
    setDialog(undefined);
    onChange(next);
  };
  const restore = async () => {
    const of = view.id;
    const next = await saving.run(() => client.removeAnalysisView({ space, table: table.name, id: view.id }));
    if (!next) return;
    onChange(next);
    forget(of);
  };

  const more: MenuItem[] = [
    {
      label: x.saveAs,
      onSelect: () => {
        saving.clear();
        setDialog("save");
      },
    },
    ...(edited ? [{ label: x.discard, onSelect: () => setDraft(undefined) }] : []),
    ...(view.builtin && view.changed ? [{ label: x.reset, onSelect: () => void restore() }] : []),
    ...(view.builtin
      ? []
      : [
          { label: x.rename, onSelect: () => setDialog("rename") },
          { label: x.remove, destructive: true, onSelect: () => setDialog("remove") },
        ]),
  ];
  const when = (ms: number) => new Intl.DateTimeFormat(locale(), { dateStyle: "medium" }).format(new Date(ms));

  // ---- What the panel offers ----
  const fields = shownFields(table);
  const metrics: ChartMetric[] = spec.metrics.length > 0 || (spec.dimensions.length === 0 && spec.compare.length === 0) ? spec.metrics : [{ column: null, aggregate: "count" }];
  // A column is a dimension once, a column of time once at each grain.
  const offered = (list: ChartDimension[]) => {
    const used = [...spec.dimensions, ...spec.compare];
    return fields
      .filter((f) => !isNumber(f) && (isTime(f) ? GRAINS.some((g) => !used.some((d) => d.column === f.name && d.grain === g)) : !used.some((d) => d.column === f.name)))
      .map((f): MenuItem => ({
        label: f.name,
        onSelect: () => {
          const taken = used.filter((d) => d.column === f.name).map((d) => d.grain);
          const grain = isTime(f) ? ((taken.includes("month") ? GRAINS.find((g) => !taken.includes(g)) : "month") ?? "year") : null;
          change(list === spec.dimensions ? { dimensions: [...list, { column: f.name, grain }] } : { compare: [...list, { column: f.name, grain }] });
        },
      }));
  };
  const addMetric = (metric: ChartMetric) => change({ metrics: [...spec.metrics, metric] });
  const metricItems: MenuItem[] = [
    ...(spec.metrics.some((m) => m.column === null) ? [] : [{ label: x.rowCount, onSelect: () => addMetric({ column: null, aggregate: "count" }) }]),
    ...fields.flatMap((f): MenuItem[] => {
      // A number is summed first; totalled again, another way each time.
      const ways = isNumber(f) ? NUMBER_AGGREGATES : OTHER_AGGREGATES;
      const free = ways.find((a) => !spec.metrics.some((m) => m.column === f.name && m.aggregate === a));
      return free ? [{ label: f.name, onSelect: () => addMetric({ column: f.name, aggregate: free }) }] : [];
    }),
  ];
  const filterItems: MenuItem[] = fields
    .filter((f) => !spec.filters.some((filter) => filter.column === f.name))
    .map((f) => ({ label: f.name, onSelect: () => setFiltering(emptyFilter(f.name)) }));

  const dimensionRows = (list: ChartDimension[], put: (next: ChartDimension[]) => void) =>
    list.map((d, at) => (
      <div key={`${d.column}:${d.grain ?? ""}`} className="screen-explorer__item">
        <span className="screen-explorer__item-name" translate="no">
          {d.column}
        </span>
        {isTime(fieldOf(table, d.column)) && (
          <Select
            label={x.grainOf(d.column)}
            hideLabel
            value={d.grain ?? "day"}
            onChange={(grain) => put(list.map((o, i) => (i === at ? { ...o, grain: grain as DateGrain } : o)))}
            options={GRAINS.map((g) => ({ value: g, label: x.grains[g] ?? g }))}
          />
        )}
        <IconButton icon="close" label={x.removeItem(dimensionName(d))} onClick={() => put(list.filter((_, i) => i !== at))} />
      </div>
    ));

  // ---- What is drawn ----
  const seriesNames = (result?.series ?? []).map((s) => {
    const compared = s.compare.map((v) => v ?? x.blank).join(" · ");
    const metric = metricName(metrics[s.metric] ?? { column: null, aggregate: "count" });
    return compared === "" ? metric : metrics.length > 1 ? `${compared} · ${metric}` : compared;
  });
  const named = seriesNames.join("\u0001");
  const drawing = useMemo((): Drawing | undefined => {
    if (!result || result.rows || spec.chart === "table") return undefined;
    const names = named === "" ? [] : named.split("\u0001");
    return {
      kind: spec.chart,
      groups: result.groups.map((g) => (g.keys.length === 0 ? x.total : g.keys.map((k) => k ?? x.blank).join(" · "))),
      series: names.map((name, i) => ({ name, values: result.groups.map((g) => g.values[i] ?? null) })),
      format: number,
    };
  }, [result, spec.chart, named]);
  const empty = result !== undefined && (result.rows ? result.rows.rows.length === 0 : result.groups.length === 0);
  const facts = result
    ? [
        result.matched === table.rows ? undefined : x.matched(result.matched, table.rows),
        result.rows && result.truncated ? x.firstRows(result.rows.rows.length) : undefined,
        !result.rows && result.allGroups > result.groups.length ? x.topOf(result.groups.length, result.allGroups) : undefined,
      ].filter((fact): fact is string => fact !== undefined)
    : [];
  const sortValue = spec.sort ? `${spec.sort.metric}:${spec.sort.descending ? "desc" : "asc"}` : "";

  return (
    <>
      <Card className="screen-explorer">
        <div className="screen-explorer__head">
          <div className="screen-explorer__views" role="group" aria-label={x.views}>
            {views.map((v) => (
              <button
                key={v.id}
                type="button"
                className="screen-explorer__view"
                aria-pressed={v.id === view.id}
                title={v.by ? x.madeByShort(v.by) : undefined}
                onClick={() => setPicked(v.id)}
              >
                {v.by && <Icon name="ai" />}
                <span translate={v.builtin ? undefined : "no"}>{v.name}</span>
              </button>
            ))}
          </div>
          <div className="screen-explorer__actions">
            {edited && <Tag>{x.edited}</Tag>}
            {edited && (
              <Button variant="primary" disabled={saving.busy} onClick={() => void saveHere()}>
                {saving.busy ? x.saving : strings.common.save}
              </Button>
            )}
            <Menu label={x.more} items={more} />
          </div>
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
            {facts.length > 0 && <p className="screen-explorer__facts">{facts.join(" · ")}</p>}
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
              <ResultTable spec={spec} result={result} seriesNames={seriesNames} />
            )}
          </div>
          <div className="screen-explorer__panel" role="group" aria-label={x.settings}>
            <Select
              label={x.chartType}
              value={spec.chart}
              onChange={(chart) => change({ chart: chart as ChartKind })}
              options={CHARTS.map((chart) => ({ value: chart, label: x.charts[chart] ?? chart }))}
            />
            <div className="screen-explorer__part" role="group" aria-label={x.dimensions}>
              <span className="oik-field__label">{x.dimensions}</span>
              {dimensionRows(spec.dimensions, (dimensions) => change({ dimensions }))}
              {offered(spec.dimensions).length > 0 && <Menu label={x.addDimension} trigger={{ icon: "plus", text: x.addDimension }} items={offered(spec.dimensions)} />}
            </div>
            <div className="screen-explorer__part" role="group" aria-label={x.compare}>
              <span className="oik-field__label">{x.compare}</span>
              {dimensionRows(spec.compare, (compare) => change({ compare }))}
              {offered(spec.compare).length > 0 && <Menu label={x.addComparison} trigger={{ icon: "plus", text: x.addComparison }} items={offered(spec.compare)} />}
            </div>
            <div className="screen-explorer__part" role="group" aria-label={x.metrics}>
              <span className="oik-field__label">{x.metrics}</span>
              {spec.metrics.map((m, at) => (
                <div key={`${m.column ?? ""}:${m.aggregate}`} className="screen-explorer__item">
                  <span className="screen-explorer__item-name" translate={m.column ? "no" : undefined}>
                    {m.column ?? x.rowCount}
                  </span>
                  {m.column && (
                    <Select
                      label={x.aggregateOf(m.column)}
                      hideLabel
                      value={m.aggregate}
                      onChange={(aggregate) => change({ metrics: spec.metrics.map((o, i) => (i === at ? { ...o, aggregate: aggregate as ChartAggregate } : o)) })}
                      options={(isNumber(fieldOf(table, m.column)) ? NUMBER_AGGREGATES : OTHER_AGGREGATES).map((a) => ({
                        value: a,
                        label: x.aggregates[a] ?? a,
                        disabled: a !== m.aggregate && spec.metrics.some((o) => o.column === m.column && o.aggregate === a),
                      }))}
                    />
                  )}
                  <IconButton icon="close" label={x.removeItem(metricName(m))} onClick={() => change({ metrics: spec.metrics.filter((_, i) => i !== at) })} />
                </div>
              ))}
              {metricItems.length > 0 && <Menu label={x.addMetric} trigger={{ icon: "plus", text: x.addMetric }} items={metricItems} />}
            </div>
            <div className="screen-explorer__part" role="group" aria-label={x.filter}>
              <span className="oik-field__label">{x.filter}</span>
              {spec.filters.map((filter, at) => (
                <div key={filter.column} className="screen-explorer__item">
                  <button type="button" className="screen-explorer__item-filter" aria-label={x.filterOn(filter.column)} onClick={() => setFiltering(filter)}>
                    <span translate="no">{filter.column}</span>
                    <span className="screen-explorer__item-what">{filterText(filter)}</span>
                  </button>
                  <IconButton icon="close" label={x.removeItem(filter.column)} onClick={() => change({ filters: spec.filters.filter((_, i) => i !== at) })} />
                </div>
              ))}
              {filterItems.length > 0 && <Menu label={x.addFilter} trigger={{ icon: "plus", text: x.addFilter }} items={filterItems} />}
            </div>
            {spec.dimensions.length > 0 && (
              <>
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
                <Select
                  label={x.show}
                  value={spec.limit === null ? "" : String(spec.limit)}
                  onChange={(value) => change({ limit: value === "" ? null : Number(value) })}
                  options={[
                    { value: "", label: x.showAll },
                    ...[...new Set([...LIMITS, ...(spec.limit === null ? [] : [spec.limit])])].sort((a, b) => a - b).map((n) => ({ value: String(n), label: x.showTop(n) })),
                  ]}
                />
              </>
            )}
          </div>
        </div>
      </Card>
      {/* Outside the card: a dialog is laid out by the page, not by the card's own rules. */}
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
      {dialog === "save" && (
        <NameView title={x.saveTitle} start="" hint={x.saveHint} busy={saving.busy} error={saving.error} onSave={(name) => void saveAs(name)} onClose={() => setDialog(undefined)} />
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
            onChange(next);
            toast(x.removed(view.name));
          }}
        />
      )}
    </>
  );
}
