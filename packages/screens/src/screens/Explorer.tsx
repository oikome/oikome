// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The explorer of an analysis table: its views, the one shown, and beside it
// the panel that says what it shows (chart type, group, split, value, order
// and filter, each a choice from a list).
//
// The view is drawn by Perspective in a page of its own that this embeds.
// That page alone is served with a policy that lets Perspective run
// (WebAssembly, blob: scripts and workers); the app's policy is unchanged,
// and the page makes no connections. It asks this component for each query,
// which asks the node; the node runs it as one read-only statement over
// this table. Perspective's own settings panel is not shown: this panel
// writes the view's description and hands it to the page.
//
// A table starts with the views its columns' roles give (by month, by
// category). What the owner changes is a draft of the view shown, until
// Save keeps it where it is (a starting view too, which can be restored),
// or it is saved as another view under a name, or discarded. Assistants allowed to read
// the collection may save views too; those say who made them.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { tableFromIPC } from "apache-arrow";
import {
  Button,
  Card,
  ConfirmDialog,
  Dialog,
  ErrorText,
  Icon,
  IconButton,
  Menu,
  Select,
  Tag,
  TextField,
  useToast,
} from "@oikome/ui";
import type { MenuItem } from "@oikome/ui";
import type { AnalysisTable, FileAnalysis } from "@oikome/client";
import { useAction, useClient } from "../data";
import { strings } from "../strings";
import { language, locale } from "../i18n/languages";

const x = strings.explorer;

/** Where the node serves the explorer's page (apps/web/analysis-frame). */
const FRAME = "/analysis-frame/index.html";
/** The charts offered, by Perspective's names for them. */
const CHARTS = ["Datagrid", "Y Bar", "X Bar", "Y Line", "Y Area", "Treemap"];
const AGGREGATES = ["sum", "avg", "count", "min", "max"];
/** The ids of the views a table starts with, as the node knows them. */
const STARTING = ["month", "category", "rows"];
/** The most values a filter offers to choose from. */
const MAX_FILTER_VALUES = 200;

type Config = Record<string, unknown>;
interface View {
  id: string;
  name: string;
  config: Config;
  /** One the table starts with: not renamed or removed. */
  builtin: boolean;
  /** A starting view the owner saved changes to: it can be restored. */
  changed?: boolean;
  by?: string;
  savedAtMs?: number;
}
type FromFrame =
  | { oikomeExplorer: 1; type: "ready" }
  | { oikomeExplorer: 1; type: "query"; id: number; sql: string }
  | { oikomeExplorer: 1; type: "shown" | "changed"; config: Config }
  | { oikomeExplorer: 1; type: "failed"; message: string };

/** What the panel says of a view: one choice for each of its questions. */
interface Shape {
  chart: string;
  group: string;
  split: string;
  /** What is totalled once grouped or split: each column once, with how. */
  values: { column: string; aggregate: string }[];
  order: "none" | "desc" | "asc";
  filterColumn: string;
  filterValue: string;
}

const isNumber = (dataType: string) => /^(U?Int|Float|Decimal)/.test(dataType);
/** A column of time has a month and a year to group by (the node adds them). */
const isTime = (field: AnalysisTable["fields"][number]) =>
  /^(Date|Timestamp)/.test(field.dataType) || field.role === "time";
const shownFields = (table: AnalysisTable) =>
  table.fields.filter((f) => f.name !== "_source_file");

/** The columns a view can be grouped or split by, each with its name in words. */
function groupable(table: AnalysisTable): { value: string; label: string }[] {
  return shownFields(table).flatMap((field) => [
    { value: field.name, label: field.name },
    ...(isTime(field)
      ? [
          { value: `${field.name} (month)`, label: x.monthOf(field.name) },
          { value: `${field.name} (year)`, label: x.yearOf(field.name) },
        ]
      : []),
  ]);
}
/** The columns a view totals: the numbers, or any column to count when there are none. */
function measurable(table: AnalysisTable): string[] {
  const numbers = shownFields(table).filter((f) => isNumber(f.dataType));
  return (numbers.length > 0 ? numbers : shownFields(table)).map((f) => f.name);
}
/** The columns a view is filtered by: text, and the months and years. */
function filterable(table: AnalysisTable): { value: string; label: string }[] {
  const text = new Set(
    shownFields(table)
      .filter((f) => /Utf8/.test(f.dataType))
      .map((f) => f.name),
  );
  return groupable(table).filter(
    (option) =>
      text.has(option.value) || / \((month|year)\)$/.test(option.value),
  );
}

function shapeOf(config: Config, table: AnalysisTable): Shape {
  const first = (key: string) => {
    const list = config[key];
    return Array.isArray(list) && typeof list[0] === "string" ? list[0] : "";
  };
  const group = first("group_by");
  const split = first("split_by");
  const columns = Array.isArray(config.columns)
    ? config.columns.filter((c): c is string => typeof c === "string")
    : [];
  const numbers = (name: string) =>
    table.fields.some((f) => f.name === name && isNumber(f.dataType));
  const chosen = config.aggregates as Record<string, unknown> | undefined;
  const first1 = measurable(table)[0];
  const named = group || split ? columns : [];
  const values = (named.length > 0 ? named : first1 ? [first1] : []).map(
    (column) => ({
      column,
      aggregate:
        typeof chosen?.[column] === "string"
          ? (chosen[column] as string)
          : numbers(column)
            ? "sum"
            : "count",
    }),
  );
  const sort = Array.isArray(config.sort)
    ? (config.sort[0] as unknown[] | undefined)
    : undefined;
  const filter = Array.isArray(config.filter)
    ? (config.filter[0] as unknown[] | undefined)
    : undefined;
  const filtered =
    filter && filter[1] === "==" && typeof filter[0] === "string";
  return {
    chart: typeof config.plugin === "string" ? config.plugin : "Datagrid",
    group,
    split,
    values,
    order: sort?.[1] === "desc" || sort?.[1] === "asc" ? sort[1] : "none",
    filterColumn: filtered ? String(filter[0]) : "",
    filterValue: filtered ? String(filter[2]) : "",
  };
}

/** A view's description from the panel's choices; how a chart is styled is kept from `was`. */
function configOf(shape: Shape, table: AnalysisTable, was: Config): Config {
  const pivoted = shape.group !== "" || shape.split !== "";
  return {
    ...(was.plugin === shape.chart && was.plugin_config
      ? { plugin_config: was.plugin_config }
      : {}),
    ...(was.columns_config ? { columns_config: was.columns_config } : {}),
    plugin: shape.chart,
    group_by: shape.group ? [shape.group] : [],
    split_by: shape.split ? [shape.split] : [],
    // Grouped or split, the chosen columns are totalled; else every column
    // is shown.
    columns:
      pivoted && shape.values.length > 0
        ? shape.values.map((v) => v.column)
        : shownFields(table).map((f) => f.name),
    aggregates: pivoted
      ? Object.fromEntries(shape.values.map((v) => [v.column, v.aggregate]))
      : {},
    // Ordered by the first of them.
    sort:
      shape.group && shape.values[0] && shape.order !== "none"
        ? [[shape.values[0].column, shape.order]]
        : [],
    filter:
      shape.filterColumn && shape.filterValue
        ? [[shape.filterColumn, "==", shape.filterValue]]
        : [],
  };
}

/** The views a table starts with, from what its columns are for. */
function builtin(table: AnalysisTable): View[] {
  const of = (role: string) => table.fields.find((f) => f.role === role)?.name;
  const [time, amount, category] = [of("time"), of("amount"), of("category")];
  const start: Shape = {
    chart: "Datagrid",
    group: "",
    split: "",
    // Without an amount, rows are counted.
    values: [amount ?? measurable(table)[0]]
      .filter((column): column is string => column !== undefined)
      .map((column) => ({ column, aggregate: amount ? "sum" : "count" })),
    order: "none",
    filterColumn: "",
    filterValue: "",
  };
  const view = (id: string, name: string, shape: Partial<Shape>): View => ({
    id,
    name,
    builtin: true,
    config: configOf({ ...start, ...shape }, table, {}),
  });
  const out: View[] = [];
  if (time)
    out.push(
      view("month", x.byMonth, { chart: "Y Bar", group: `${time} (month)` }),
    );
  if (category)
    out.push(
      view("category", x.byCategory, {
        chart: "X Bar",
        group: category,
        order: "desc",
      }),
    );
  if (out.length === 0) out.push(view("rows", x.allRows, {}));
  return out;
}

function saved(table: AnalysisTable): View[] {
  return table.views.map((view) => {
    let config: Config = {};
    try {
      config = JSON.parse(view.config) as Config;
    } catch {
      // A view kept in a shape this app can't read opens as the table itself.
    }
    return {
      id: view.id,
      name: view.name,
      config,
      builtin: false,
      by: view.by ?? undefined,
      savedAtMs: view.savedAtMs,
    };
  });
}

const quoted = (name: string) => `"${name.replace(/"/g, '""')}"`;

/** Name a view: a new one saved, or one renamed. */
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
          <Button
            variant="primary"
            disabled={!ready}
            onClick={() => onSave(name.trim())}
          >
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
        <TextField
          label={x.viewName}
          name="view-name"
          value={name}
          onChange={setName}
          hint={hint}
          autoFocus
          selectOnFocus
        />
      </form>
      {error && <ErrorText>{error}</ErrorText>}
    </Dialog>
  );
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
  const frame = useRef<HTMLIFrameElement>(null);
  const [ready, setReady] = useState(false);
  const [drawn, setDrawn] = useState(false);
  const [failed, setFailed] = useState<string>();
  const [picked, setPicked] = useState<string>();
  // What the owner made of the view shown, not saved yet.
  const [draft, setDraft] = useState<Config>();
  // A column chosen to filter by, before its value is.
  const [filterBy, setFilterBy] = useState<string>();
  const [values, setValues] = useState<{ column: string; list: string[] }>();
  const [dialog, setDialog] = useState<"save" | "rename" | "remove">();

  const views = useMemo(() => {
    const kept = saved(table);
    // A starting view saved in place is kept under its own id, and shown
    // in place of what the columns' roles give.
    const starting = builtin(table).map((v): View => {
      const own = kept.find((k) => k.id === v.id);
      return own ? { ...v, config: own.config, changed: true } : v;
    });
    return [...starting, ...kept.filter((k) => !STARTING.includes(k.id))];
  }, [table]);
  const view = views.find((v) => v.id === picked) ?? views[0]!;
  const opened = `${view.id}:${JSON.stringify(view.config)}`;
  const current = draft ?? view.config;
  const shape = shapeOf(current, table);
  const edited =
    draft !== undefined &&
    JSON.stringify(shape) !== JSON.stringify(shapeOf(view.config, table));

  const tell = useCallback(
    (message: Record<string, unknown>, transfer: Transferable[] = []) => {
      frame.current?.contentWindow?.postMessage(
        { oikomeExplorer: 1, ...message },
        location.origin,
        transfer,
      );
    },
    [],
  );
  const show = useCallback(
    (config: Config, fresh: boolean) => {
      tell({
        type: "open",
        config,
        fresh,
        lang: language(),
        theme: document.documentElement.dataset.theme ?? "ink",
      });
    },
    [tell],
  );

  // The page's messages: only from the frame embedded here, on this origin.
  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (
        event.origin !== location.origin ||
        event.source !== frame.current?.contentWindow
      )
        return;
      const message = event.data as FromFrame;
      if (message?.oikomeExplorer !== 1) return;
      if (message.type === "ready") setReady(true);
      else if (message.type === "failed") setFailed(message.message);
      else if (message.type === "shown") {
        setDrawn(true);
        setFailed(undefined);
      } else if (message.type === "changed") setDraft(message.config);
      else if (message.type === "query") {
        const { id, sql } = message;
        client.analysisQuery({ space, table: table.name, sql }).then(
          (bytes) => tell({ type: "answer", id, bytes }, [bytes]),
          (error: unknown) =>
            tell({
              type: "answer",
              id,
              error: error instanceof Error ? error.message : String(error),
            }),
        );
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [client, space, table.name, tell]);

  // Show the picked view when the page is ready, and again when it changes
  // (another view, or a role that changes what a starting view groups by).
  const open = useCallback(() => {
    setDrawn(false);
    setDraft(undefined);
    setFilterBy(undefined);
    show(view.config, true);
    // `opened` stands for the view and its description.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [opened, show]);
  useEffect(() => {
    if (ready) open();
  }, [ready, open]);

  /** One of the panel's choices changed: the view shown becomes a draft. */
  const change = (patch: Partial<Shape>) => {
    const next = { ...shape, ...patch };
    if (next.split === next.group) next.split = "";
    const config = configOf(next, table, current);
    setDraft(config);
    show(config, false);
  };

  // The values a filter offers: what the chosen column holds.
  const filterColumn = filterBy ?? shape.filterColumn;
  useEffect(() => {
    if (!filterColumn) return;
    let stale = false;
    const sql = `SELECT DISTINCT CAST(${quoted(filterColumn)} AS VARCHAR) AS value FROM oikome_table WHERE ${quoted(filterColumn)} IS NOT NULL ORDER BY 1 LIMIT ${MAX_FILTER_VALUES}`;
    client.analysisQuery({ space, table: table.name, sql }).then(
      (bytes) => {
        if (stale) return;
        const list = tableFromIPC(new Uint8Array(bytes))
          .toArray()
          .map((row) => String((row as { value: unknown }).value));
        setValues({ column: filterColumn, list });
      },
      () => {
        if (!stale) setValues({ column: filterColumn, list: [] });
      },
    );
    return () => {
      stale = true;
    };
  }, [client, space, table.name, table.builtAtMs, filterColumn]);

  /** Save the view shown, where it is: no name to give, nothing to choose. */
  const saveHere = async () => {
    const next = await saving.run(() =>
      client.saveAnalysisView({
        space,
        table: table.name,
        id: view.id,
        name: view.builtin ? view.id : view.name,
        config: JSON.stringify(current),
      }),
    );
    if (!next) return;
    onChange(next);
    toast(x.savedNow);
  };
  /** Keep what is shown as another view, under a name. */
  const saveAs = async (name: string) => {
    const next = await saving.run(() =>
      client.saveAnalysisView({ space, table: table.name, name, config: JSON.stringify(current) }),
    );
    if (!next) return;
    setDialog(undefined);
    onChange(next);
    setPicked(
      next.tables
        .find((t) => t.name === table.name)
        ?.views.find((v) => v.name === name)?.id,
    );
    toast(x.saved(name));
  };
  const restore = async () => {
    const next = await saving.run(() => client.removeAnalysisView({ space, table: table.name, id: view.id }));
    if (next) onChange(next);
  };
  const rename = async (name: string) => {
    const next = await saving.run(() =>
      client.saveAnalysisView({
        space,
        table: table.name,
        id: view.id,
        name,
        config: JSON.stringify(view.config),
      }),
    );
    if (!next) return;
    setDialog(undefined);
    onChange(next);
  };

  const more: MenuItem[] = [
    {
      label: x.save,
      onSelect: () => {
        saving.clear();
        setDialog("save");
      },
    },
    ...(edited ? [{ label: x.discard, onSelect: open }] : []),
    ...(view.builtin && view.changed ? [{ label: x.reset, onSelect: () => void restore() }] : []),
    ...(view.builtin
      ? []
      : [
          { label: x.rename, onSelect: () => setDialog("rename") },
          {
            label: x.remove,
            destructive: true,
            onSelect: () => setDialog("remove"),
          },
        ]),
  ];
  const when = (ms: number) =>
    new Intl.DateTimeFormat(locale(), { dateStyle: "medium" }).format(
      new Date(ms),
    );

  const none = { value: "", label: x.none };
  const columns = groupable(table);
  const pivoted = shape.group !== "" || shape.split !== "";
  const measures = measurable(table);
  const isNumeric = (name: string) => table.fields.some((f) => f.name === name && isNumber(f.dataType));
  const unused = measures.filter((name) => !shape.values.some((v) => v.column === name));
  const charts = CHARTS.includes(shape.chart)
    ? CHARTS
    : [...CHARTS, shape.chart];
  const offered = values?.column === filterColumn ? values.list : [];
  const filterValues =
    shape.filterValue && !offered.includes(shape.filterValue)
      ? [shape.filterValue, ...offered]
      : offered;

  return (
    <>
      <Card className="screen-explorer">
        <div className="screen-explorer__head">
          <div
            className="screen-explorer__views"
            role="group"
            aria-label={x.views}
          >
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
        {failed && (
          <div className="screen-explorer__failed">
            <ErrorText>
              {x.failed} {failed}
            </ErrorText>
          </div>
        )}
        <div className="screen-explorer__body">
          <div className="screen-explorer__stage" aria-busy={!ready || !drawn}>
            <iframe
              ref={frame}
              className="screen-explorer__frame"
              src={FRAME}
              title={x.frameTitle(table.name)}
            />
            {!ready && (
              <div
                className="screen-explorer__loading oik-skel"
                role="status"
                aria-label={x.loading}
              />
            )}
          </div>
          <div
            className="screen-explorer__panel"
            role="group"
            aria-label={x.settings}
          >
            <Select
              label={x.chartType}
              value={shape.chart}
              onChange={(chart) => change({ chart })}
              options={charts.map((chart) => ({
                value: chart,
                label: x.charts[chart] ?? chart,
              }))}
            />
            <Select
              label={x.groupBy}
              value={shape.group}
              onChange={(group) => change({ group })}
              options={[none, ...columns]}
            />
            <Select
              label={x.splitBy}
              value={shape.split}
              onChange={(split) => change({ split })}
              options={[
                none,
                ...columns.filter((c) => c.value !== shape.group),
              ]}
            />
            {pivoted && measures.length > 0 && (
              <div className="screen-explorer__values" role="group" aria-label={x.value}>
                <span className="oik-field__label">{x.value}</span>
                {shape.values.map((value, at) => {
                  const numeric = isNumeric(value.column);
                  const others = shape.values.filter((_, i) => i !== at).map((v) => v.column);
                  const put = (next: { column: string; aggregate: string }) =>
                    change({ values: shape.values.map((v, i) => (i === at ? next : v)) });
                  return (
                    <div key={value.column} className="screen-explorer__value">
                      <Select
                        label={x.aggregateOf(value.column)}
                        hideLabel
                        value={value.aggregate}
                        onChange={(aggregate) => put({ ...value, aggregate })}
                        options={(numeric ? AGGREGATES : ["count"]).map((a) => ({ value: a, label: x.aggregates[a] ?? a }))}
                      />
                      <Select
                        label={x.valueColumn(at + 1)}
                        hideLabel
                        value={value.column}
                        onChange={(column) => put({ column, aggregate: isNumeric(column) ? value.aggregate : "count" })}
                        options={measures.filter((name) => !others.includes(name)).map((name) => ({ value: name, label: name }))}
                      />
                      {shape.values.length > 1 && (
                        <IconButton
                          icon="close"
                          label={x.removeValue(value.column)}
                          onClick={() => change({ values: shape.values.filter((_, i) => i !== at) })}
                        />
                      )}
                    </div>
                  );
                })}
                {unused.length > 0 && (
                  <Button
                    onClick={() =>
                      change({ values: [...shape.values, { column: unused[0]!, aggregate: isNumeric(unused[0]!) ? "sum" : "count" }] })
                    }
                  >
                    {x.addValue}
                  </Button>
                )}
              </div>
            )}
            {shape.group !== "" && (
              <Select
                label={x.order}
                value={shape.order}
                onChange={(order) => change({ order: order as Shape["order"] })}
                options={(["none", "desc", "asc"] as const).map((o) => ({
                  value: o,
                  label: x.orders[o] ?? o,
                }))}
              />
            )}
            <div className="screen-explorer__pair screen-explorer__pair--stacked">
              <Select
                label={x.filter}
                value={filterColumn}
                onChange={(column) => {
                  setFilterBy(column);
                  if (shape.filterColumn)
                    change({ filterColumn: "", filterValue: "" });
                }}
                options={[none, ...filterable(table)]}
              />
              {filterColumn !== "" && (
                <Select
                  label={x.chooseValue}
                  hideLabel
                  value={
                    shape.filterColumn === filterColumn ? shape.filterValue : ""
                  }
                  onChange={(filterValue) =>
                    change({ filterColumn, filterValue })
                  }
                  options={[
                    { value: "", label: x.chooseValue, disabled: true },
                    ...filterValues.map((value) => ({ value, label: value })),
                  ]}
                />
              )}
            </div>
          </div>
        </div>
      </Card>
      {/* Outside the card: a dialog is laid out by the page, not by the card's own rules. */}
      {dialog === "save" && (
        <NameView
          title={x.saveTitle}
          start=""
          hint={x.saveHint}
          busy={saving.busy}
          error={saving.error}
          onSave={(name) => void saveAs(name)}
          onClose={() => setDialog(undefined)}
        />
      )}
      {dialog === "rename" && (
        <NameView
          title={x.renameTitle}
          start={view.name}
          busy={saving.busy}
          error={saving.error}
          onSave={(name) => void rename(name)}
          onClose={() => setDialog(undefined)}
        />
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
            const next = await removing.run(() =>
              client.removeAnalysisView({
                space,
                table: table.name,
                id: view.id,
              }),
            );
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
