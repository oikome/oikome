// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Putting things in order, as a file manager does: by name, when they last
// changed, size or kind, either way round; shown as covers or a list. How
// the owner likes them is kept in this browser.

import { useCallback, useState } from "react";
import { locale } from "./i18n/languages";

export type SortKey = "name" | "changed" | "size" | "kind";
export const SORT_KEYS: readonly SortKey[] = ["name", "changed", "size", "kind"];

export interface Order {
  key: SortKey;
  descending: boolean;
}

/** Names and kinds start from A; the newest and the largest come first. */
const FIRST_DESCENDING: Record<SortKey, boolean> = { name: false, kind: false, changed: true, size: true };

/** Picking the order in use again turns it round; another starts its own way. */
export function nextOrder(order: Order, key: SortKey): Order {
  return order.key === key ? { key, descending: !order.descending } : { key, descending: FIRST_DESCENDING[key] };
}

export type Layout = "covers" | "list";

/** A choice kept in this browser; private windows and blocked storage just forget it. */
function useKept(key: string, fallback: string, valid: (value: string) => boolean): [string, (value: string) => void] {
  const name = `oikome.${key}`;
  const [value, setValue] = useState(() => {
    try {
      const kept = window.localStorage.getItem(name);
      return kept !== null && valid(kept) ? kept : fallback;
    } catch {
      return fallback;
    }
  });
  const keep = useCallback(
    (next: string) => {
      setValue(next);
      try {
        window.localStorage.setItem(name, next);
      } catch {
        // Not kept: the choice still holds until the page is left.
      }
    },
    [name],
  );
  return [value, keep];
}

/** An order kept in this browser, under `key`. */
export function useOrder(key: string, fallback: Order): [Order, (order: Order) => void] {
  const [kept, keep] = useKept(`${key}.order`, `${fallback.key}:${fallback.descending ? "desc" : "asc"}`, (v) => {
    const [k, d] = v.split(":");
    return SORT_KEYS.includes(k as SortKey) && (d === "asc" || d === "desc");
  });
  const [k, d] = kept.split(":");
  const order = { key: k as SortKey, descending: d === "desc" };
  return [order, (next) => keep(`${next.key}:${next.descending ? "desc" : "asc"}`)];
}

/** Covers or a list, kept in this browser, under `key`. */
export function useLayout(key: string): [Layout, (layout: Layout) => void] {
  const [kept, keep] = useKept(`${key}.layout`, "covers", (v) => v === "covers" || v === "list");
  return [kept as Layout, (next) => keep(next)];
}

let collator: { locale: string; compare: Intl.Collator } | undefined;

/** Words compared as the language in use sorts them; 2 before 10. */
export function compareText(a: string, b: string): number {
  if (collator?.locale !== locale()) {
    collator = { locale: locale(), compare: new Intl.Collator(locale(), { numeric: true, sensitivity: "base" }) };
  }
  return collator.compare.compare(a, b);
}

/**
 * Sort `list` by `order`: `value` gives what it is sorted by, `name` the
 * tie-break. What has no value (a size this computer can't tell) goes
 * last either way round.
 */
export function sortBy<T>(
  list: T[],
  order: Order,
  value: (item: T, key: SortKey) => string | number | null | undefined,
  name: (item: T) => string,
): T[] {
  const way = order.descending ? -1 : 1;
  return [...list].sort((a, b) => {
    const x = value(a, order.key);
    const y = value(b, order.key);
    if (x == null || y == null || x === y) {
      if (x == null && y != null) return 1;
      if (y == null && x != null) return -1;
      return compareText(name(a), name(b));
    }
    const by = typeof x === "number" && typeof y === "number" ? x - y : compareText(String(x), String(y));
    return by * way || compareText(name(a), name(b));
  });
}

/** A column heading that sorts by its column; picked again, the other way round. */
export function SortHeader({
  label,
  sortKey,
  order,
  onSort,
  className,
}: {
  label: string;
  sortKey: SortKey;
  order: Order;
  onSort: (order: Order) => void;
  className?: string;
}) {
  const on = order.key === sortKey;
  return (
    <th scope="col" className={className} aria-sort={on ? (order.descending ? "descending" : "ascending") : undefined}>
      <button
        type="button"
        className={on ? "screen-sorth screen-sorth--on" : "screen-sorth"}
        onClick={() => onSort(nextOrder(order, sortKey))}
      >
        {label}
        {on && <span aria-hidden="true">{order.descending ? "↓" : "↑"}</span>}
      </button>
    </th>
  );
}
