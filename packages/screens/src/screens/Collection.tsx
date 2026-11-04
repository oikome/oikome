// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// One collection on a page of its own: what is in it, shown as what it is
// (photos, notes or a table), who can see it, and where it is kept. One
// shared with you says whom it is from, what you get and until when, and
// can be left.

import { useEffect, useMemo, useState } from "react";
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
  collectionTones,
  useToast,
} from "@oikome/ui";
import type { SpaceSummary } from "@oikome/client";
import type { Table } from "apache-arrow";
import { chronological, forgetPreviews, itemsOf, loadPreview, shapeOf } from "../content";
import type { Item, Shape } from "../content";
import { countOf, fromOf, itemDate, sharedNames } from "../covers";
import { useAction, useClient, useLoad } from "../data";
import { dateFromSeconds, homeRelative, monthAndDay } from "../format";
import { since } from "./Sources";
import { SeriesView } from "../series";
import { RowsTable, ScreenHeader } from "../parts";
import { Link, navigate } from "../router";
import { strings } from "../strings";
import { ShareDialog } from "./ShareDialog";

const s = strings.collections;
/** Rows a page reads: fewer of photos, which are large. */
const PAGE_ROWS = 100;
const PAGE_PHOTOS = 24;

export function Collection({ name }: { name: string }) {
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

      {sync.error && <ErrorText>{sync.error}</ErrorText>}
      <Contents key={`${space.name}:${contentsKey}`} space={space} />
    </>
  );
}

function Contents({ space }: { space: SpaceSummary }) {
  const client = useClient();
  // A shared collection is read from its owner's computer, which can be off
  // or may have stopped sharing; say so instead of the node's message.
  const unreadable = space.kind === "remote" ? () => s.cantRead : undefined;
  const tables = useLoad(() => client.listTables({ space: space.name }), [space.name], unreadable);
  const [table, setTable] = useState<string>();
  const [offset, setOffset] = useState(0);
  const [pageSize, setPageSize] = useState(PAGE_ROWS);

  useEffect(() => {
    const first = tables.data?.[0]?.name;
    if (first && !table) setTable(first);
  }, [tables.data, table]);

  const rows = useLoad(
    async () =>
      table
        ? client.readRows({ space: space.name, table, limit: pageSize, offset })
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
  const total = Number(tables.data?.find((t) => t.name === table)?.rows ?? 0);
  const [atLatest, setAtLatest] = useState(false);
  useEffect(() => {
    if (shape?.view === "series" && !atLatest && offset === 0 && total > pageSize) {
      setAtLatest(true);
      setOffset(total - pageSize);
    }
  }, [shape, atLatest, offset, total, pageSize]);
  const count = rows.data?.table.numRows ?? 0;

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
      {rows.data && shape && table && <View table={rows.data.table} shape={shape} label={table} />}
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
    </div>
  );
}

function View({ table, shape, label }: { table: Table; shape: Shape; label: string }) {
  // Photos and notes read in the order they happened, as a diary does.
  const items = useMemo(() => chronological(itemsOf(table, shape)), [table, shape]);
  if (shape.view === "photos") {
    return (
      <div className="screen-photos" aria-label={label}>
        {items.map((item, i) => (
          <Photo key={i} item={item} />
        ))}
      </div>
    );
  }
  if (shape.view === "notes") {
    return (
      <div className="screen-notes" aria-label={label}>
        {items.map((item, i) => (
          <article key={i} className="screen-note">
            {itemDate(item) && <span className="screen-note__date">{itemDate(item)}</span>}
            <p className="screen-note__text">{item.text}</p>
          </article>
        ))}
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

function Photo({ item }: { item: Item }) {
  const day = item.when === undefined ? undefined : monthAndDay(item.when, item.whenKind !== "moment");
  const meta = [item.detail, day].filter(Boolean).join(" · ");
  return (
    <figure className="screen-photo">
      {item.image ? (
        <img src={item.image} alt={item.text ?? ""} decoding="async" />
      ) : (
        <div className="screen-photo__missing" />
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
