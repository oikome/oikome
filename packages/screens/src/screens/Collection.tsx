// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// One collection on a page of its own: what is in it, shown as what it is
// (photos, notes or a table), who can see it, and where it is kept. One
// shared with you says whom it is from, what you get and until when, and
// can be left.

import { useEffect, useMemo, useState } from "react";
import type { MouseEvent, ReactNode } from "react";
import {
  Avatar,
  AvatarStack,
  Button,
  Card,
  ConfirmDialog,
  EmptyState,
  ErrorText,
  Menu,
  Select,
  Tag,
  collectionTones,
  useToast,
} from "@oikome/ui";
import type { SpaceSummary } from "@oikome/client";
import type { Table } from "apache-arrow";
import { chronological, forgetPreviews, itemsOf, loadPreview, shapeOf, tagsOf, withoutTagLines } from "../content";
import type { Item, Shape } from "../content";
import { countOf, fromOf, itemDate, sharedNames } from "../covers";
import { useAction, useClient, useLoad } from "../data";
import { dateFromSeconds, homeRelative, monthAndDay } from "../format";
import { since } from "./Sources";
import { SeriesView } from "../series";
import { RowsTable, ScreenHeader } from "../parts";
import { ItemDialog, itemPath } from "../item";
import type { ItemNav } from "../item";
import { Palette } from "../palette";
import { Link, navigate, plainClick, previousPage } from "../router";
import { strings } from "../strings";
import { ShareDialog } from "./ShareDialog";

const s = strings.collections;
/** Rows a page reads: fewer of photos, which are large. */
const PAGE_ROWS = 100;
const PAGE_PHOTOS = 24;

/** An item the address names: `?table=…&item=…`, one `item` per key column. */
export interface ItemQuery {
  table: string | null;
  key: string[];
}

export function Collection({ name, item }: { name: string; item?: ItemQuery }) {
  const client = useClient();
  const toast = useToast();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const contacts = useLoad(() => client.listContacts(), []);
  const storages = useLoad(() => client.listStorages(), []);
  const sources = useLoad(() => client.listSources(), []);
  const [confirming, setConfirming] = useState(false);
  const [contentsKey, setContentsKey] = useState(0);
  const sync = useAction();
  const [sharing, setSharing] = useState(false);
  const [searching, setSearching] = useState(false);
  const remove = useAction();

  const list = spaces.data ?? [];
  const space = list.find((x) => x.name === name);
  // What it holds, from the same first rows its cover reads; one shared
  // with you is read from its owner's computer, which may be off.
  const preview = useLoad(async () => (space ? loadPreview(client, space) : undefined), [space?.name, space?.kind]);
  const mine = list.filter((x) => x.kind !== "remote");
  const toneOf = collectionTones(list.map((x) => x.name));
  const names = sharedNames(shares.data).get(name) ?? [];
  const reload = () => {
    spaces.reload();
    shares.reload();
  };

  if (spaces.error) return <ErrorText>{spaces.error}</ErrorText>;
  if (!spaces.data) return <div className="screen-page-skeleton oik-skel" aria-hidden="true" />;
  if (!space) {
    return (
      <Card>
        <EmptyState
          title={strings.notFound.title}
          action={<Link to="/collections">{strings.nav.collections}</Link>}
        />
      </Card>
    );
  }
  const local = space.kind !== "remote";
  const from = fromOf(space);
  const kind = preview.data?.shape.view;
  const place = storages.data?.find((x) => x.name === space.storage);
  const where = space.storage === null ? s.thisComputer : (place?.label ?? space.storage);
  const source = sources.data?.find((x) => x.name === space.name);
  const syncNow = async () => {
    if (!source) return;
    const done = await sync.run(() => client.pullSource({ source: source.name }));
    forgetPreviews(space.name);
    sources.reload();
    preview.reload();
    setContentsKey((k) => k + 1);
    if (done) toast(strings.sources.synced(space.name));
  };

  return (
    <>
      <ScreenHeader
        mono
        label={
          local
            ? [kind ? s.kinds[kind] : null, space.example ? s.fromExampleShort : s.fromYouShort].filter(Boolean).join(" · ")
            : s.sharedWithYouFrom(from)
        }
        title={space.name}
      >
        {local && (
          <Button icon="search" onClick={() => setSearching(true)}>
            {s.searchIn}
          </Button>
        )}
        {local ? (
          <Button variant="primary" onClick={() => setSharing(true)}>
            {s.share}
          </Button>
        ) : (
          <Button onClick={() => setConfirming(true)}>{s.leave}</Button>
        )}
        {source && (
          <Button icon="sync" disabled={sync.busy} spinning={sync.busy} onClick={syncNow}>
            {strings.sources.syncNow}
          </Button>
        )}
        {local && (
          <Menu
            label={strings.common.moreActions(space.name)}
            items={[
              ...(source
                ? [
                    {
                      label: strings.sources.showFiles,
                      onSelect: () => void sync.run(() => client.showSourceFiles({ source: source.name })),
                    },
                  ]
                : []),
              { label: s.remove, destructive: true, onSelect: () => setConfirming(true) },
            ]}
          />
        )}
      </ScreenHeader>

      {local ? (
        <dl className="oik-card screen-facts">
          <div>
            <dt className="oik-label">{s.factHolds}</dt>
            <dd>{countOf(preview.data) ?? s.noData}</dd>
          </div>
          <div>
            <dt className="oik-label">{s.factFrom}</dt>
            {source ? (
              <dd className="screen-facts__from">
                <Link to={`/sources/${encodeURIComponent(source.name)}`}>
                  <span title={source.place} translate="no">
                    {homeRelative(source.place)}
                  </span>
                </Link>
                <span className="screen-facts__note">
                  {source.failedAtMs != null
                    ? strings.sources.failedAgo(since(source.failedAtMs))
                    : source.pulledAtMs != null
                      ? strings.sources.syncedAgo(since(source.pulledAtMs))
                      : strings.sources.waiting}
                </span>
              </dd>
            ) : (
              <dd>{space.example ? s.fromExample : s.fromYou}</dd>
            )}
          </div>
          <div>
            <dt className="oik-label">{s.factStored}</dt>
            <dd>{place?.encrypted ? `${where} · ${strings.settings.encrypted}` : where}</dd>
          </div>
          <div>
            <dt className="oik-label">{s.factWho}</dt>
            <dd className="screen-facts__who">
              <AvatarStack names={names} label={s.whoSentence(names)} />
              <span>{[strings.common.you, ...names].join(", ")}</span>
              <Link to="/sharing">{s.manage}</Link>
            </dd>
          </div>
        </dl>
      ) : (
        <dl className="oik-card screen-facts">
          <div>
            <dt className="oik-label">{s.factHolds}</dt>
            <dd>{countOf(preview.data) ?? "—"}</dd>
          </div>
          <div>
            <dt className="oik-label">{s.factSharedBy}</dt>
            <dd className="screen-facts__who">
              <Avatar name={from} small />
              <span>{from}</span>
            </dd>
          </div>
          <div>
            <dt className="oik-label">{s.factYouGet}</dt>
            <dd>{s.youGetLive}</dd>
          </div>
          <div>
            <dt className="oik-label">{s.factUntil}</dt>
            <dd>{space.expiresAt != null ? dateFromSeconds(space.expiresAt) : "—"}</dd>
          </div>
        </dl>
      )}

      {sharing && (
        <ShareDialog
          collection={space.name}
          spaces={mine}
          contacts={contacts.data ?? []}
          shares={shares.data ?? []}
          toneOf={toneOf}
          onClose={() => setSharing(false)}
          onChange={reload}
        />
      )}
      {confirming && (
        <ConfirmDialog
          title={local ? s.removeTitle(space.name) : s.leaveTitle(space.name)}
          body={local ? s.removeBody : s.leaveBody(from)}
          confirmLabel={local ? s.removeYes : s.leave}
          cancelLabel={strings.common.cancel}
          busy={remove.busy}
          error={remove.error}
          onConfirm={async () => {
            const done = await remove.run(() => client.dropSpace({ space: space.name }));
            if (done) {
              toast(local ? strings.toast.removed(space.name) : s.left(space.name));
              forgetPreviews(space.name);
              // The removed collection's page is gone: Back skips it.
              navigate("/collections", { replace: true });
            }
          }}
          onCancel={() => {
            setConfirming(false);
            remove.clear();
          }}
        />
      )}

      {searching && <Palette scope={space.name} onClose={() => setSearching(false)} />}

      {sync.error && <ErrorText>{sync.error}</ErrorText>}
      <Contents
        key={`${space.name}:${contentsKey}`}
        space={space}
        item={item}
        facts={{
          cameFrom: source ? homeRelative(source.place) : space.example ? s.fromExample : s.fromYou,
          keptOn: place?.encrypted ? `${where} · ${strings.settings.encrypted}` : where,
          who: [strings.common.you, ...names].join(", "),
        }}
      />
    </>
  );
}

/** What an item's own view says of its collection. */
interface Facts {
  cameFrom: ReactNode;
  keptOn: string;
  who: string;
}

function Contents({ space, item, facts }: { space: SpaceSummary; item?: ItemQuery; facts: Facts }) {
  const client = useClient();
  // A shared collection is read from its owner's computer, which can be off
  // or may have stopped sharing; say so instead of the node's message.
  const unreadable = space.kind === "remote" ? () => s.cantRead : undefined;
  const tables = useLoad(() => client.listTables({ space: space.name }), [space.name], unreadable);
  // An address that names an item opens its table.
  const [table, setTable] = useState<string | undefined>(item?.table ?? undefined);
  const [offset, setOffset] = useState(0);
  const [pageSize, setPageSize] = useState(PAGE_ROWS);

  useEffect(() => {
    if (!tables.data) return;
    if (!table || !tables.data.some((t) => t.name === table)) setTable(tables.data[0]?.name);
  }, [tables.data, table]);

  const rows = useLoad(
    async () =>
      table
        ? { ...(await client.readRows({ space: space.name, table, limit: pageSize, offset })), at: offset }
        : undefined,
    [space.name, table, offset, pageSize],
    unreadable,
  );
  const shape = useMemo(() => (rows.data ? shapeOf(rows.data.table) : undefined), [rows.data]);
  // Photos are read a page of fewer at a time; the first read tells.
  useEffect(() => {
    if (shape?.view === "photos" && pageSize !== PAGE_PHOTOS && offset === 0) setPageSize(PAGE_PHOTOS);
  }, [shape, pageSize, offset]);
  // Readings open at the latest ones; Previous goes back in time.
  const summary = tables.data?.find((t) => t.name === table);
  const total = Number(summary?.rows ?? 0);
  const [atLatest, setAtLatest] = useState(false);
  useEffect(() => {
    if (shape?.view === "series" && !atLatest && offset === 0 && total > pageSize) {
      setAtLatest(true);
      setOffset(total - pageSize);
    }
  }, [shape, atLatest, offset, total, pageSize]);
  const count = rows.data?.table.numRows ?? 0;

  // Photos and notes read in the order they happened, as a diary does.
  const items = useMemo(() => {
    if (!rows.data || !shape) return [];
    const all = itemsOf(rows.data.table, shape);
    return shape.view === "photos" || shape.view === "notes" ? chronological(all) : all;
  }, [rows.data, shape]);
  // Each item of a table with a key has an address of its own.
  const keyOf = (it: Item): string[] | undefined => {
    const columns = summary?.key ?? [];
    if (!rows.data || columns.length === 0 || space.kind === "remote") return undefined;
    const parts = columns.map((c) => rows.data!.table.getChild(c)?.get(it.row) as unknown);
    if (parts.some((v) => v === null || v === undefined || v instanceof Uint8Array)) return undefined;
    return parts.map((v) => String(v));
  };
  const linkOf = (it: Item): string | undefined => {
    const key = keyOf(it);
    return key && table ? itemPath({ collection: space.name, table, key }) : undefined;
  };
  const open = (it: Item | undefined, replace: boolean) => {
    const to = it && linkOf(it);
    if (to) navigate(to, { replace, keepScroll: true });
  };

  // Past the first or last item of a page, the next page opens at its first
  // (or the one before at its last) once it has been read.
  const [turning, setTurning] = useState<{ at: number; to: "first" | "last" }>();
  useEffect(() => {
    if (!turning || rows.data?.at !== turning.at) return;
    setTurning(undefined);
    open(turning.to === "first" ? items[0] : items[items.length - 1], true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [turning, rows.data, items]);
  const turn = (at: number, to: "first" | "last") => {
    setTurning({ at, to });
    setOffset(at);
  };

  // The item the address names: moved through when it is on this page.
  const itemTable = item && item.key.length > 0 && space.kind !== "remote" ? (item.table ?? tables.data?.[0]?.name) : undefined;
  const here =
    itemTable === table && (shape?.view === "photos" || shape?.view === "notes")
      ? items.findIndex((it) => {
          const key = keyOf(it);
          return key !== undefined && key.length === item!.key.length && key.every((k, i) => k === item!.key[i]);
        })
      : -1;
  const more = count === pageSize && (total === 0 || offset + count < total);
  const nav: ItemNav | undefined =
    here >= 0
      ? {
          position: offset + here + 1,
          total: Math.max(total, offset + count),
          previous: here > 0 ? () => open(items[here - 1], true) : offset > 0 ? () => turn(Math.max(0, offset - pageSize), "last") : undefined,
          next: here < items.length - 1 ? () => open(items[here + 1], true) : more ? () => turn(offset + pageSize, "first") : undefined,
        }
      : undefined;

  if (tables.error) return <ErrorText>{tables.error}</ErrorText>;
  if (tables.data && tables.data.length === 0) {
    return (
      <Card>
        <EmptyState icon="table" title={s.nothingYetTitle} body={s.nothingYetBody} />
      </Card>
    );
  }
  return (
    <div className="screen-contents">
      {tables.data && tables.data.length > 1 && table && (
        <div className="screen-toolbar">
          <Select
            inline
            label={s.tableLabel}
            value={table}
            onChange={(value) => {
              setTable(value);
              setOffset(0);
              setPageSize(PAGE_ROWS);
            }}
            options={tables.data.map((t) => ({ value: t.name, label: t.name }))}
          />
        </div>
      )}
      {rows.error && <ErrorText>{rows.error}</ErrorText>}
      {!rows.data && !rows.error && <div className="screen-page-skeleton oik-skel" aria-hidden="true" />}
      {rows.data && shape && table && (
        <View table={rows.data.table} shape={shape} items={items} label={table} linkOf={linkOf} onOpen={(it) => open(it, false)} />
      )}
      {rows.data && count === 0 && <p className="screen-muted">{s.noRows}</p>}
      {/* Paging only when there is more than one page. */}
      {(offset > 0 || count === pageSize) && (
        <div className="screen-pager">
          <span className="screen-muted">{s.rows(offset + 1, offset + count)}</span>
          <span>
            <Button
              variant="quiet"
              disabled={offset === 0}
              onClick={() => setOffset(Math.max(0, offset - pageSize))}
            >
              {s.previous}
            </Button>
            <Button variant="quiet" disabled={count < pageSize} onClick={() => setOffset(offset + pageSize)}>
              {s.next}
            </Button>
          </span>
        </div>
      )}
      {itemTable && (
        <ItemDialog
          item={{ collection: space.name, table: itemTable, key: item!.key }}
          known={
            rows.data && shape && itemTable === table
              ? { shape, columns: rows.data.table.schema.fields.map((f) => f.name) }
              : undefined
          }
          cameFrom={facts.cameFrom}
          keptOn={facts.keptOn}
          who={facts.who}
          nav={nav}
          onClose={() => {
            // Back where it was opened from on this page; else this page,
            // without the item, in its place.
            const page = `/collections/${encodeURIComponent(space.name)}`;
            if (previousPage()?.split("?")[0] === page) window.history.back();
            else navigate(page, { replace: true, keepScroll: true });
          }}
        />
      )}
    </div>
  );
}

function View({
  table,
  shape,
  items,
  label,
  linkOf,
  onOpen,
}: {
  table: Table;
  shape: Shape;
  items: Item[];
  label: string;
  linkOf: (item: Item) => string | undefined;
  onOpen: (item: Item) => void;
}) {
  // A plain click opens the item over the page; others keep the browser's.
  const opener = (it: Item) => (e: MouseEvent<HTMLAnchorElement>) => {
    if (!plainClick(e)) return;
    e.preventDefault();
    onOpen(it);
  };
  if (shape.view === "photos") {
    return (
      <div className="screen-photos" aria-label={label}>
        {items.map((it) => (
          <Photo key={it.row} item={it} href={linkOf(it)} onOpen={opener(it)} />
        ))}
      </div>
    );
  }
  if (shape.view === "notes") {
    return (
      <div className="screen-notes" aria-label={label}>
        {items.map((it) => {
          const href = linkOf(it);
          const tags = tagsOf(it.text);
          const body = (
            <>
              {itemDate(it) && <span className="screen-note__date">{itemDate(it)}</span>}
              <p className="screen-note__text">{withoutTagLines(it.text)}</p>
              {tags.length > 0 && (
                <span className="screen-tags">
                  {tags.map((tag) => (
                    <Tag key={tag}>#{tag}</Tag>
                  ))}
                </span>
              )}
            </>
          );
          return (
            <article key={it.row} className="screen-note">
              {href ? (
                <a className="screen-note__link" href={href} onClick={opener(it)}>
                  {body}
                </a>
              ) : (
                body
              )}
            </article>
          );
        })}
      </div>
    );
  }
  if (shape.view === "series") {
    return (
      <>
        <SeriesView table={table} shape={shape} />
        <div className="screen-table-card">
          <RowsTable table={table} label={label} />
        </div>
      </>
    );
  }
  return (
    <div className="screen-table-card">
      <RowsTable table={table} label={label} />
    </div>
  );
}

function Photo({
  item,
  href,
  onOpen,
}: {
  item: Item;
  href?: string;
  onOpen: (e: MouseEvent<HTMLAnchorElement>) => void;
}) {
  const day = item.when === undefined ? undefined : monthAndDay(item.when, item.whenKind !== "moment");
  const meta = [item.detail, day].filter(Boolean).join(" · ");
  const picture = item.image ? (
    <img src={item.image} alt={item.text ?? ""} decoding="async" />
  ) : (
    <div className="screen-photo__missing" />
  );
  return (
    <figure className="screen-photo">
      {href ? (
        <a className="screen-photo__link" href={href} onClick={onOpen} aria-label={s.open(item.text ?? s.kinds.photos)}>
          {picture}
        </a>
      ) : (
        picture
      )}
      <figcaption>
        <span className="screen-photo__line">
          {item.text && <span className="screen-photo__title">{item.text}</span>}
          {meta && (
            <span className="screen-photo__meta" title={itemDate(item)}>
              {meta}
            </span>
          )}
        </span>
        {item.credit && <span className="screen-photo__credit">{item.credit}</span>}
      </figcaption>
    </figure>
  );
}
