// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The explorer: Perspective, in a page of its own that the Analysis view
// embeds. Perspective needs to compile WebAssembly and to start scripts and
// workers from blob: URLs, which the app's policy refuses; this page alone
// is served with a policy that allows them, and makes no connections.
//
// Nothing is computed here. Perspective's SQL builder writes each query;
// the executor below hands it to the app, which asks the node. The node
// runs it as one read-only statement over the table shown, and nothing
// else. Perspective keeps each view as a temporary table; here its
// definition stays in this page, and each read goes out as one
// `WITH <view> AS (<definition>) SELECT …`.

import perspective from "@perspective-dev/client";
import perspective_viewer from "@perspective-dev/viewer";
import "@perspective-dev/viewer-datagrid";
import "@perspective-dev/viewer-charts";
import { DuckDBHandler } from "@perspective-dev/client/virtual_servers/duckdb";
import { tableFromIPC, Type, type DataType, type Table } from "apache-arrow";
import "@oikome/ui/tokens.css";
import "@perspective-dev/viewer/dist/css/themes.css";
import "@perspective-dev/viewer/dist/css/pro.css";
import "./explorer.css";
import VIEWER_WASM from "@perspective-dev/viewer/dist/wasm/perspective-viewer.wasm?inline";
import CLIENT_WASM from "@perspective-dev/client/dist/wasm/perspective-js.wasm?inline";
import SERVER_WASM from "@perspective-dev/server/dist/wasm/perspective-server.wasm?inline";

/** The table every query reads, as the node names it. */
const TABLE = "oikome_table";
/** What of the viewer's state is a view; the rest is this page's own. */
const VIEW_KEYS = [
  "plugin",
  "plugin_config",
  "columns_config",
  "group_by",
  "split_by",
  "sort",
  "filter",
  "columns",
  "aggregates",
  "group_rollup_mode",
  "split_rollup_mode",
];

type FromApp =
  | { oikomeExplorer: 1; type: "open"; config: Record<string, unknown>; fresh: boolean; lang: string; theme: string }
  | { oikomeExplorer: 1; type: "answer"; id: number; bytes?: ArrayBuffer; error?: string };

/** A file inlined in this page's script as a data URL: its bytes. */
function bytesOf(dataUrl: string): Uint8Array<ArrayBuffer> {
  const text = atob(dataUrl.slice(dataUrl.indexOf(",") + 1));
  const out = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) out[i] = text.charCodeAt(i);
  return out;
}

// ---- The bridge to the app: the only way out of this page ----
const app = window.parent;
const tell = (message: Record<string, unknown>) => app.postMessage({ oikomeExplorer: 1, ...message }, location.origin);
let next = 1;
const waiting = new Map<number, (answer: { bytes?: ArrayBuffer; error?: string }) => void>();
let opened: ((message: Extract<FromApp, { type: "open" }>) => void) | undefined;
window.addEventListener("message", (event) => {
  if (event.source !== app || event.origin !== location.origin) return;
  const message = event.data as FromApp;
  if (message?.oikomeExplorer !== 1) return;
  if (message.type === "answer") waiting.get(message.id)?.(message);
  else if (message.type === "open") opened?.(message);
});
function node(sql: string): Promise<Uint8Array> {
  const id = next++;
  return new Promise((resolve, reject) => {
    waiting.set(id, (answer) => {
      waiting.delete(id);
      if (answer.bytes) resolve(new Uint8Array(answer.bytes));
      else reject(new Error(answer.error ?? "no answer"));
    });
    tell({ type: "query", id, sql });
  });
}

// ---- The executor ----
const views = new Map<string, string>(); // a view's name, as Perspective writes it → its definition

function duckdbType(type: DataType): string {
  switch (type.typeId) {
    case Type.Int:
      return (type as unknown as { bitWidth: number }).bitWidth === 64 ? "BIGINT" : "INTEGER";
    case Type.Float:
    case Type.Decimal:
      return "DOUBLE";
    case Type.Bool:
      return "BOOLEAN";
    case Type.Date:
      return "DATE";
    case Type.Timestamp:
      return "TIMESTAMP";
    case Type.Time:
      return "TIME";
    default:
      return "VARCHAR";
  }
}
/** The kept views a statement reads, each after the views it reads itself. */
function withViews(sql: string): string {
  const needed: string[] = [];
  const visit = (text: string) => {
    for (const [id, definition] of views) {
      if (!needed.includes(id) && text.includes(id)) {
        visit(definition);
        needed.push(id);
      }
    }
  };
  visit(sql);
  if (needed.length === 0) return sql;
  const clause = needed.map((id) => `${id} AS (${views.get(id)})`).join(", ");
  return /^WITH\s/i.test(sql) ? `WITH ${clause}, ${sql.slice(5)}` : `WITH ${clause} ${sql}`;
}
/** Where the bracket opened at `open` closes, quoted text aside. */
function closing(text: string, open: number): number {
  let depth = 0;
  for (let i = open; i < text.length; i++) {
    const ch = text[i];
    if (ch === "'" || ch === '"') {
      i = text.indexOf(ch, i + 1);
      if (i < 0) break;
    } else if (ch === "(") depth += 1;
    else if (ch === ")" && --depth === 0) return i;
  }
  throw new Error("a query's brackets do not close");
}
/** The most columns a split makes: one for each value split by. */
const MAX_SPLIT_VALUES = 500;
const PIVOT_SOURCE = "__PSP_PIVOT_SRC__";
/**
 * A split is written by Perspective as DuckDB's own `PIVOT … ON … USING …
 * GROUP BY …`, whose columns depend on the rows. The node reads only plain
 * queries, so the values split by are asked for first and the pivot is
 * written out: one aggregate for each, filtered to its value.
 */
async function unpivoted(sql: string): Promise<string> {
  const pivot = `(PIVOT ${PIVOT_SOURCE} ON `;
  if (!sql.includes(pivot)) return sql;
  const source = `WITH ${PIVOT_SOURCE} AS (`;
  const from = sql.indexOf(source);
  if (from < 0) throw new Error("a split without its rows");
  const open = from + source.length - 1;
  const rows = sql.slice(open + 1, closing(sql, open));
  let out = sql;
  for (let at = out.indexOf(pivot); at >= 0; at = out.indexOf(pivot)) {
    const end = closing(out, at);
    const parts = out.slice(at + 1, end).match(/^PIVOT \S+ ON (.+) USING (first\(.+\)) GROUP BY (.+)$/s);
    if (!parts) throw new Error("a split written in a way this explorer does not read");
    const [, on, value, by] = parts as unknown as [string, string, string, string];
    const answer = await node(
      withViews(
        `WITH ${PIVOT_SOURCE} AS (${rows}) SELECT DISTINCT ${on} AS split_value FROM ${PIVOT_SOURCE} ` +
          `WHERE (${on}) IS NOT NULL ORDER BY 1 LIMIT ${MAX_SPLIT_VALUES + 1}`,
      ),
    );
    const values = tableFromIPC(answer).toArray().map((row) => String((row as { split_value: unknown }).split_value));
    if (values.length > MAX_SPLIT_VALUES) throw new Error(`a split makes at most ${MAX_SPLIT_VALUES} columns`);
    const columns = values.map(
      (v) => `${value} FILTER (WHERE (${on}) = '${v.replace(/'/g, "''")}') AS "${v.replace(/"/g, '""')}"`,
    );
    out = `${out.slice(0, at)}(SELECT ${[by, ...columns].join(", ")} FROM ${PIVOT_SOURCE} GROUP BY ${by})${out.slice(end + 1)}`;
  }
  return out;
}
/** Rows this page answers itself, in the shape Perspective reads. */
function rows(records: Record<string, string | number | boolean>[]) {
  return { toArray: () => records.map((record) => ({ toJSON: () => record })), schema: { fields: [] } };
}
async function fieldsOf(target: string) {
  // A name (the table, or a view kept here) is read as it is; a query in brackets.
  const from = /^[\w".]+$/.test(target) ? target : `(${target})`;
  const answer = await node(withViews(await unpivoted(`SELECT * FROM ${from} AS oikome_described LIMIT 0`)));
  // `rowid` is the order of the table's rows, which the node adds for
  // Perspective to sort plain rows by: not a column to show.
  return tableFromIPC(answer).schema.fields.filter((field) => field.name !== "rowid");
}

const executor = {
  async query(raw: string): Promise<Table | ReturnType<typeof rows>> {
    const sql = raw.replace(/\s+/g, " ").trim();
    let m = sql.match(/^CREATE (?:TEMPORARY TABLE|TABLE|VIEW) (\S+) AS \((.*)\)$/is);
    if (m) {
      views.set(m[1]!, await unpivoted(m[2]!));
      return rows([{ Count: 0 }]);
    }
    m = sql.match(/^DROP (?:TABLE|VIEW) IF EXISTS (\S+)$/i);
    if (m) {
      views.delete(m[1]!);
      return rows([{ Count: 0 }]);
    }
    m = sql.match(/^SELECT COUNT\(\*\) FROM \(DESCRIBE (.+)\)$/i);
    if (m) return rows([{ count: (await fieldsOf(m[1]!)).length }]);
    m = sql.match(/^DESCRIBE (.+)$/i);
    if (m) {
      const fields = await fieldsOf(m[1]!);
      return rows(fields.map((f) => ({ column_name: f.name, column_type: duckdbType(f.type), null: "YES" })));
    }
    // Anything that is not a read is refused here, and again by the node.
    if (!/^(SELECT|WITH)\b/i.test(sql)) throw new Error(`not offered: ${sql.split(" ")[0]}`);
    return tableFromIPC(await node(withViews(await unpivoted(sql))));
  },
  async useUnsafe<T>(run: (bindings: { runQuery(connection: unknown, sql: string): Promise<Uint8Array> }, connection: unknown) => Promise<T>): Promise<T> {
    return run({ runQuery: async (_connection, sql) => node(withViews(await unpivoted(sql.replace(/\s+/g, " ").trim()))) }, null);
  },
};

class Handler extends DuckDBHandler {
  getFeatures() {
    // No expand and collapse (it updates a table) and no free expressions
    // (their functions are Perspective's own, not the node's).
    return { ...super.getFeatures(), expand_collapse: false, expressions: false };
  }
  async getHostedTables() {
    return [TABLE];
  }
}

// ---- The page ----
function viewOf(state: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(VIEW_KEYS.filter((key) => key in state).map((key) => [key, state[key]]));
}

async function main() {
  const start = new Promise<Extract<FromApp, { type: "open" }>>((resolve) => (opened = resolve));
  await perspective_viewer.init({ viewer: bytesOf(VIEWER_WASM), client: bytesOf(CLIENT_WASM), server: bytesOf(SERVER_WASM) } as never);
  await customElements.whenDefined("perspective-viewer");
  const viewer = document.querySelector("perspective-viewer") as HTMLElement & {
    load(client: unknown): Promise<void>;
    restore(config: Record<string, unknown>): Promise<void>;
    save(): Promise<Record<string, unknown>>;
    reset(): Promise<void>;
  };
  // Perspective draws the view; what it shows is chosen in the app's own
  // panel beside this page. Its own settings panel stays closed, and the
  // bar that opens it is left out.
  const hidden = new CSSStyleSheet();
  hidden.replaceSync("#status_bar, #settings_panel { display: none !important; }");
  viewer.shadowRoot?.adoptedStyleSheets.push(hidden);
  const handler = new Handler(executor as never);
  const port = await (perspective as never as { createMessageHandler(h: unknown): Promise<MessagePort> }).createMessageHandler(handler);
  const client = await (perspective as never as { worker(port: Promise<MessagePort>): Promise<unknown> }).worker(Promise.resolve(port));
  await viewer.load(client);

  let applying = false;
  const open = async (message: Extract<FromApp, { type: "open" }>) => {
    document.documentElement.lang = message.lang;
    document.documentElement.dataset.theme = message.theme === "bold" ? "bold" : "ink";
    applying = true;
    try {
      // Another view starts from nothing; a change to the view shown is
      // made in place.
      if (message.fresh) await viewer.reset();
      await viewer.restore({ ...viewOf(message.config), table: TABLE, theme: "Pro Light", settings: false });
      tell({ type: "shown", config: viewOf(await viewer.save()) });
    } catch (error) {
      tell({ type: "failed", message: error instanceof Error ? error.message : String(error) });
    } finally {
      applying = false;
    }
  };
  // What the owner changes in the view itself (a column header clicked to
  // sort by it) is told to the app, which shows it in its panel.
  viewer.addEventListener("perspective-config-update", () => {
    if (applying) return;
    void viewer.save().then((state) => tell({ type: "changed", config: viewOf(state) }));
  });
  opened = (message) => void open(message);
  tell({ type: "ready" });
  await open(await start);
}

main().catch((error) => tell({ type: "failed", message: error instanceof Error ? error.message : String(error) }));
