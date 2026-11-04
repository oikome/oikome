// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Plugin cards on Home. A plugin's page runs in a sandboxed frame the node
// serves with no network; this side reads the plugin's collections through
// the node *as the plugin* (so the node's grants decide), turns the rows
// into plain values, and posts them, with the style, to that frame alone.
// The frame can ask for one thing back: to open one of its collections.

import { useEffect, useRef, useState } from "react";
import type { Plugin } from "@oikome/client";
import type { Table } from "apache-arrow";
import { DataType } from "apache-arrow";
import { imageSrc } from "./content";
import { useClient } from "./data";
import { isoTime } from "./format";
import { navigate } from "./router";
import { strings } from "./strings";

const s = strings.plugins;

export type PluginColumnType = "text" | "number" | "date" | "image" | "bool" | "other";

export interface PluginCollection {
  name: string;
  columns: { name: string; type: PluginColumnType }[];
  rows: Record<string, string | number | boolean | null>[];
}

/** A collection's rows as plain values, each column with a type a plugin can rely on. */
export function pluginCollection(name: string, table: Table): PluginCollection {
  const columns = table.schema.fields.map((field, c) => {
    const vector = table.getChildAt(c);
    const values = Array.from({ length: table.numRows }, (_, r) => vector?.get(r) as unknown);
    let type: PluginColumnType = "other";
    if (DataType.isInt(field.type) || DataType.isFloat(field.type) || DataType.isDecimal(field.type)) type = "number";
    else if (DataType.isBool(field.type)) type = "bool";
    else if (DataType.isDate(field.type) || DataType.isTimestamp(field.type)) type = "date";
    else if (DataType.isBinary(field.type) || DataType.isLargeBinary(field.type)) type = "image";
    else if (DataType.isUtf8(field.type) || DataType.isLargeUtf8(field.type)) {
      // Days and times kept as text read as dates when every value is one.
      const present = values.filter((v) => typeof v === "string" && v !== "");
      type = present.length > 0 && present.every((v) => isoTime(v as string)) ? "date" : "text";
    }
    const cell = (v: unknown): string | number | boolean | null => {
      if (v === null || v === undefined) return null;
      if (type === "number") return typeof v === "bigint" ? Number(v) : Number(v);
      if (type === "bool") return Boolean(v);
      if (type === "image") return v instanceof Uint8Array ? (imageSrc(v) ?? null) : null;
      if (type === "date") {
        // ISO, as the bridge promises: "2025-11-07 09:40" reads "2025-11-07T09:40".
        if (typeof v === "string") return v.trim().replace(" ", "T");
        const ms = typeof v === "bigint" ? Number(v) : typeof v === "number" ? v : v instanceof Date ? v.getTime() : NaN;
        return Number.isFinite(ms) ? new Date(ms).toISOString().replace(/\.000Z$/, "Z") : null;
      }
      return String(v);
    };
    return { field, type, values: values.map(cell) };
  });
  return {
    name,
    columns: columns.map(({ field, type }) => ({ name: field.name, type })),
    rows: Array.from({ length: table.numRows }, (_, r) =>
      Object.fromEntries(columns.map(({ field, values }) => [field.name, values[r]!])),
    ),
  };
}

/** The page's style for a plugin: its theme, its colors and type, and the app's fonts. */
function pluginTheme() {
  const root = document.documentElement;
  const css = getComputedStyle(root);
  const pick = (name: string) => css.getPropertyValue(name).trim();
  const vars: Record<string, string> = {};
  for (const [to, from] of [
    ["--oik-bg", "--bg"],
    ["--oik-raised", "--raised"],
    ["--oik-sunk", "--sunk"],
    ["--oik-ink", "--ink"],
    ["--oik-ink2", "--ink2"],
    ["--oik-ink3", "--ink3"],
    ["--oik-line", "--line"],
    ["--oik-line2", "--line2"],
    ["--oik-accent", "--accent"],
    ["--oik-on-accent", "--on-accent"],
    ["--oik-signal", "--signal"],
    ["--oik-font", "--font"],
    ["--oik-mono", "--mono"],
    ["--oik-display", "--display"],
    ["--oik-radius", "--r-card"],
  ] as const) {
    const value = pick(from);
    if (value) vars[to] = value;
  }
  // The app's own fonts, by their files here, so the plugin can load them.
  const fonts: { family: string; url: string; weight: string; style: string }[] = [];
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      continue;
    }
    for (const rule of Array.from(rules)) {
      if (!(rule instanceof CSSFontFaceRule)) continue;
      const url = /url\(["']?([^"')]+\.woff2)["']?\)/.exec(rule.style.getPropertyValue("src"))?.[1];
      const family = rule.style.getPropertyValue("font-family").replace(/["']/g, "").trim();
      if (!url || !family) continue;
      const path = new URL(url, window.location.href);
      if (path.origin !== window.location.origin) continue;
      fonts.push({
        family,
        url: path.pathname,
        weight: rule.style.getPropertyValue("font-weight") || "400",
        style: rule.style.getPropertyValue("font-style") || "normal",
      });
    }
  }
  return { name: root.dataset.theme === "bold" ? "bold" : "ink", vars, fonts };
}

/** Today in the owner's calendar, as ISO. */
function today(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** A plugin's card: its name and maker, and its page in a sandboxed frame. */
export function PluginCard({ plugin }: { plugin: Plugin }) {
  const client = useClient();
  const frame = useRef<HTMLIFrameElement>(null);
  const [problem, setProblem] = useState<string>();

  useEffect(() => {
    let current = true;
    const post = (message: object) => frame.current?.contentWindow?.postMessage({ oikome: 1, ...message }, "*");
    const send = async () => {
      const collections: PluginCollection[] = [];
      for (const name of plugin.collections) {
        try {
          const { table } = await client.pluginRead({ plugin: plugin.id, space: name });
          collections.push(pluginCollection(name, table));
        } catch {
          // A collection it can't read now (removed, or holding nothing yet) is left out.
        }
      }
      if (!current) return;
      setProblem(plugin.collections.length > 0 && collections.length === 0 ? s.nothingToRead : undefined);
      post({ type: "data", collections, theme: pluginTheme(), today: today() });
    };
    const onMessage = (event: MessageEvent) => {
      // Only this card's frame, and only what the bridge says.
      if (!frame.current || event.source !== frame.current.contentWindow) return;
      const message = event.data as { oikome?: unknown; type?: unknown; collection?: unknown } | null;
      if (!message || message.oikome !== 1) return;
      if (message.type === "ready") void send();
      if (message.type === "open" && typeof message.collection === "string" && plugin.collections.includes(message.collection)) {
        navigate(`/collections/${encodeURIComponent(message.collection)}`);
      }
    };
    window.addEventListener("message", onMessage);
    // A change of style reaches the card at once.
    const watch = new MutationObserver(() => post({ type: "theme", theme: pluginTheme() }));
    watch.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => {
      current = false;
      window.removeEventListener("message", onMessage);
      watch.disconnect();
    };
  }, [client, plugin.id, plugin.collections.join("\n")]);

  return (
    <section className="oik-card screen-panel screen-plugin" aria-label={plugin.name}>
      <header className="screen-panel__head">
        <h2 className="oik-label">{plugin.name}</h2>
        <span className="screen-plugin__by">{s.byPlugin(plugin.by)}</span>
      </header>
      {problem ? (
        <p className="screen-panel__empty">{problem}</p>
      ) : (
        <iframe
          ref={frame}
          className="screen-plugin__frame"
          title={plugin.name}
          src={`/plugins/${encodeURIComponent(plugin.id)}/${plugin.entry}`}
          sandbox="allow-scripts"
          allow=""
          referrerPolicy="no-referrer"
          loading="lazy"
          style={{ height: plugin.height }}
        />
      )}
    </section>
  );
}
