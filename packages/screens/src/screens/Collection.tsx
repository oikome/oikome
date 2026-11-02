// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// One collection on a page of its own: what is in it, shown as what it is
// (photos, notes or a table), who can see it, and where it is kept.

import { useEffect, useMemo, useState } from "react";
import {
  AvatarStack,
  Button,
  Card,
  ConfirmDialog,
  EmptyState,
  ErrorText,
  Menu,
  Select,
  Tile,
  collectionTones,
  useToast,
} from "@oikome/ui";
import type { SpaceSummary } from "@oikome/client";
import type { Table } from "apache-arrow";
import { chronological, forgetPreviews, itemsOf, shapeOf } from "../content";
import type { Item, Shape } from "../content";
import { itemDate, sharedNames } from "../covers";
import { useAction, useClient, useLoad } from "../data";
import { BackLink, RowsTable } from "../parts";
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
  const [confirming, setConfirming] = useState(false);
  const [sharing, setSharing] = useState(false);
  const remove = useAction();

  const list = spaces.data ?? [];
  const space = list.find((x) => x.name === name);
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

  return (
    <>
      <BackLink parent="/collections" parentLabel={s.back} />
      <header className="screen-header screen-header--collection">
        <Tile icon="collection" tone={toneOf(space.name)} large />
        <div className="screen-collection__name">
          <h1 className="screen-title">{space.name}</h1>
          {space.example && <span className="screen-tag">{s.example}</span>}
        </div>
        <div className="screen-header__actions">
          {local && (
            <Button variant="primary" onClick={() => setSharing(true)}>
              {s.share}
            </Button>
          )}
          <Menu
            label={strings.common.moreActions(space.name)}
            items={[{ label: s.remove, destructive: true, onSelect: () => setConfirming(true) }]}
          />
        </div>
      </header>

      <p className="screen-who">
        {local ? (
          <>
            <AvatarStack names={names} label={s.whoSentence(names)} />
            <span>{s.whoSentence(names)}</span>
            <span aria-hidden="true">·</span>
            <span>{s.storedOn(space.storage)}</span>
            <Link to="/sharing">{s.manage}</Link>
          </>
        ) : (
          <span>{s.fromElsewhere}</span>
        )}
      </p>

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
          title={s.removeTitle(space.name)}
          body={s.removeBody}
          confirmLabel={s.removeYes}
          cancelLabel={strings.common.cancel}
          busy={remove.busy}
          error={remove.error}
          onConfirm={async () => {
            const done = await remove.run(() => client.dropSpace({ space: space.name }));
            if (done) {
              toast(strings.toast.removed(space.name));
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

      <Contents key={space.name} space={space} />
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
  return (
    <div className="screen-table-card">
      <RowsTable table={table} label={label} />
    </div>
  );
}

function Photo({ item }: { item: Item }) {
  const meta = [itemDate(item), item.detail].filter(Boolean).join(" · ");
  return (
    <figure className="screen-photo">
      {item.image ? (
        <img src={item.image} alt={item.text ?? ""} decoding="async" />
      ) : (
        <div className="screen-photo__missing" />
      )}
      <figcaption>
        {item.text && <span className="screen-photo__title">{item.text}</span>}
        {meta && <span className="screen-photo__meta">{meta}</span>}
        {item.credit && <span className="screen-photo__credit">{item.credit}</span>}
      </figcaption>
    </figure>
  );
}
