// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// One item of a collection on its own: a photo large, a note in full, a row
// field by field, with where it is from, where it is kept and who can see
// it. A photo or note opens here from its collection's page, and moves to
// the one before or after with ← →; a search hit opens here too, and so
// does an address that names the item (`?item=`), so each one has a link of
// its own.

import { lazy, Suspense, useEffect, useId, useMemo, useRef } from "react";
import type { ReactNode } from "react";
import { ErrorText, IconButton, Tag } from "@oikome/ui";
import type { Table } from "apache-arrow";
import { itemsOf, shapeOf, tagsOf, withoutTagLines } from "./content";
import type { Item, Shape } from "./content";
import { itemDate } from "./covers";
import { useClient, useLoad } from "./data";
import { columnLabel } from "./format";
import { show } from "./parts";
import { plainClick } from "./router";
import { strings } from "./strings";

const ImageView = lazy(() => import("./ImageView"));
const s = strings.collections;

/** Which item: its collection, table and primary key. */
export interface ItemRef {
  collection: string;
  table: string;
  key: string[];
}

/** The address of an item, on its collection's page. */
/** The folder a path is in, within a folder viewed in place; `""` is its top. */
export function parentOf(path: string): string {
  const at = path.lastIndexOf("/");
  return at < 0 ? "" : path.slice(0, at);
}

/**
 * The address of one folder of a collection viewed in place: each folder
 * has its own, so Back goes up a folder and the top crumb can name them.
 */
export function folderPath(collection: string, folder: string): string {
  const page = `/collections/${encodeURIComponent(collection)}`;
  return folder ? `${page}?${new URLSearchParams({ folder }).toString()}` : page;
}

/**
 * The folder a collection's page shows, from its address: the one named,
 * else the one of the file opened over it.
 */
export function folderOf(query: URLSearchParams): string {
  const named = query.get("folder");
  if (named !== null) return named.replace(/^\/+|\/+$/g, "");
  const item = query.getAll("item");
  return query.get("table") === "files" && item.length === 1 ? parentOf(item[0]!) : "";
}

export function itemPath(ref: ItemRef): string {
  const query = new URLSearchParams({ table: ref.table });
  for (const part of ref.key) query.append("item", part);
  return `/collections/${encodeURIComponent(ref.collection)}?${query.toString()}`;
}

export interface ImageGallery {
  index: number;
  items: { src: string; alt: string }[];
  open: (index: number) => void;
}

/** Moving to the item before or after: where this one is, and how to move. */
export interface ItemNav {
  /** 1-based, among `total`. */
  position: number;
  total: number;
  previous?: () => void;
  next?: () => void;
  gallery?: ImageGallery;
}

/**
 * The item's shape: the page's own for this table when the row has the
 * same columns (one row alone can look like a table where many read as
 * notes), else the row's.
 */
function shapeFor(row: Table, known?: { shape: Shape; columns: string[] }): Shape {
  const names = row.schema.fields.map((f) => f.name);
  if (known && known.columns.length === names.length && known.columns.every((c, i) => c === names[i])) {
    return known.shape;
  }
  return shapeOf(row);
}

export function ItemDialog({
  item,
  known,
  cameFrom,
  keptOn,
  who,
  nav,
  onClose,
}: {
  item: ItemRef;
  /** The shape the collection's page read this table as, and its columns. */
  known?: { shape: Shape; columns: string[] };
  cameFrom: ReactNode;
  keptOn: ReactNode;
  who: string;
  nav?: ItemNav;
  onClose: () => void;
}) {
  const client = useClient();
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const row = useLoad(
    () => client.readRow({ space: item.collection, table: item.table, key: item.key }),
    [item.collection, item.table, item.key.join("\u0000")],
    (e) => (e.code === "not_found" ? s.itemGone(item.collection) : undefined),
  );
  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => dialog?.close();
  }, []);

  const shown = useMemo(() => {
    const table = row.data?.table;
    if (!table || table.numRows === 0) return undefined;
    const shape = shapeFor(table, known);
    return { table, shape, it: itemsOf(table, shape)[0]! };
  }, [row.data, known]);

  // Laid out as the page knows the table before the row is read, so the
  // view does not change under the person once it is.
  const view = shown?.shape.view ?? known?.shape.view;
  const photo = view === "photos" && !row.error && (!shown || shown.it.image !== undefined);
  const title = shown ? titleOf(shown.it, shown.shape) : "";
  const facts: [string, ReactNode][] = [];
  const day = shown && itemDate(shown.it);
  if (view === "photos" && shown && (day || shown.it.detail)) {
    facts.push([s.taken, [shown.it.detail, day].filter(Boolean).join(" · ")]);
  } else if (day) {
    facts.push([view === "notes" ? s.written : s.when, day]);
  }
  facts.push([
    s.in_,
    <a
      key="in"
      className="oik-link"
      href={`/collections/${encodeURIComponent(item.collection)}`}
      translate="no"
      onClick={(e) => {
        // It opens on its collection's page: going there is closing it.
        if (!plainClick(e)) return;
        e.preventDefault();
        onClose();
      }}
    >
      {item.collection}
    </a>,
  ]);
  facts.push([s.cameFrom, cameFrom], [s.keptOn, keptOn], [s.whoCanSee, who]);
  const tags = view === "notes" && shown ? tagsOf(shown.it.text) : [];

  // ← → move wherever focus is while it is open; focus stays inside it.
  const moving = useRef(nav);
  moving.current = nav;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey || e.defaultPrevented || document.querySelector(".pswp--open")) return;
      const go = e.key === "ArrowLeft" ? moving.current?.previous : e.key === "ArrowRight" ? moving.current?.next : undefined;
      if (!go) return;
      e.preventDefault();
      go();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => {
    const dialog = ref.current;
    if (dialog?.open && !dialog.contains(document.activeElement)) {
      dialog.querySelector<HTMLButtonElement>(".screen-item__head button")?.focus();
    }
  }, [photo, shown]);

  const move = nav && (
    <div className="screen-item__nav">
      <IconButton icon="back" label={s.previousItem} disabled={!nav.previous} onClick={() => nav.previous?.()} />
      <span className="screen-item__position">{s.position(nav.position, nav.total)}</span>
      <IconButton icon="chevron" label={s.nextItem} disabled={!nav.next} onClick={() => nav.next?.()} />
    </div>
  );
  const head = (
    <header className="screen-item__head">
      <h2 id={titleId} className="screen-item__title">
        {title}
      </h2>
      <IconButton icon="close" label={strings.common.close} filled onClick={onClose} />
    </header>
  );
  const list = (
    <dl className="screen-item__facts">
      {facts.map(([label, value]) => (
        <div key={label}>
          <dt className="oik-label">{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );

  return (
    <dialog
      ref={ref}
      className={photo ? "screen-item screen-item--photo" : "screen-item"}
      aria-labelledby={titleId}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        if (e.target === ref.current) onClose();
      }}
    >
      {photo ? (
        <>
          <div className="screen-item__image">
            {shown?.it.image && (
              <Suspense fallback={<div className="screen-item__skeleton oik-skel" aria-hidden="true" />}>
                <ImageView src={shown.it.image} alt={shown.it.text ?? ""} gallery={nav?.gallery} />
              </Suspense>
            )}
          </div>
          <div className="screen-item__side">
            {head}
            {list}
            {move}
          </div>
        </>
      ) : (
        <>
          {head}
          <div className="screen-item__body">
            {row.error && <ErrorText>{row.error}</ErrorText>}
            {!row.data && !row.error && <div className="screen-item__skeleton oik-skel" aria-hidden="true" />}
            {row.data && !shown && !row.error && <ErrorText>{s.itemGone(item.collection)}</ErrorText>}
            {shown && view === "notes" && <p className="screen-item__note">{withoutTagLines(shown.it.text)}</p>}
            {tags.length > 0 && (
              <div className="screen-tags">
                {tags.map((tag) => (
                  <Tag key={tag}>#{tag}</Tag>
                ))}
              </div>
            )}
            {shown && view !== "notes" && <Fields table={shown.table} />}
            {list}
          </div>
          {move}
        </>
      )}
    </dialog>
  );
}

/** A photo by its caption, a note as a note, anything else as a row. */
function titleOf(it: Item, shape: Shape): string {
  if (shape.view === "photos") return it.text ?? s.kinds.photos;
  if (shape.view === "notes") return s.note;
  return s.row;
}

/** A row's fields, name above value; images are left to the photo view. */
function Fields({ table }: { table: Table }) {
  const fields = table.schema.fields
    .map((field, c) => ({ field, value: table.getChildAt(c)?.get(0) as unknown }))
    .filter(({ value }) => !(value instanceof Uint8Array));
  return (
    <dl className="screen-item__fields">
      {fields.map(({ field, value }) => {
        const text = show(field, value);
        return (
          <div key={field.name}>
            <dt className="oik-label" title={field.name}>
              {columnLabel(field.name)}
            </dt>
            <dd>{text ?? <span className="oik-table__null">—</span>}</dd>
          </div>
        );
      })}
    </dl>
  );
}
