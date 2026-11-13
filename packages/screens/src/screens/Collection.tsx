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
  Banner,
  Button,
  Card,
  ConfirmDialog,
  EmptyState,
  ErrorText,
  Icon,
  Menu,
  Segmented,
  Select,
  Tag,
  collectionTones,
  useToast,
} from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { FileFolder, FileItem, FileSort, SpaceSummary } from "@oikome/client";
import type { Table } from "apache-arrow";
import {
  chronological,
  fileItems,
  forgetPreviews,
  itemsOf,
  loadPreview,
  shapeOf,
  tagsOf,
  withoutTagLines,
} from "../content";
import type { Item, Shape } from "../content";
import { FolderCover, collectionTitle, countOf, fromOf, itemDate, sharedNames } from "../covers";
import { useAction, useClient, useLoad } from "../data";
import { bytes, dateFromSeconds, dayAndTime, homeRelative, listNames, monthAndDay } from "../format";
import { SORT_KEYS, SortHeader, nextOrder, sortBy, useLayout, useOrder } from "../order";
import type { Layout, SortKey } from "../order";
import { since } from "./Sources";
import { SeriesView } from "../series";
import { RowsTable, ScreenHeader } from "../parts";
import { ItemDialog, folderPath, itemPath } from "../item";
import type { ItemNav } from "../item";
import { Palette } from "../palette";
import { FileView } from "../viewer";
import { AddFiles } from "../import";
import { Link, navigate, plainClick, previousPage } from "../router";
import { strings } from "../strings";
import { ShareDialog } from "./ShareDialog";
import { placeOf, storageLabel } from "./Collections";
import { placePath } from "./Place";

const s = strings.collections;
/** Rows a page reads: fewer of photos, which are large. */
const PAGE_ROWS = 100;
const PAGE_PHOTOS = 24;

/** An item the address names: `?table=…&item=…`, one `item` per key column. */
export interface ItemQuery {
  table: string | null;
  key: string[];
}

export function Collection({ name, item, folder: openFolder = "" }: { name: string; item?: ItemQuery; folder?: string }) {
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
  const [importing, setImporting] = useState(false);
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
  const folder = space.inPlace;
  const folderPlace = folder ? storages.data?.find((x) => x.name === folder.place) : undefined;
  // Where a folder viewed in place is: its path, on this computer or a NAS.
  const folderWhere = folder
    ? folder.place
      ? `${storages.data?.find((x) => x.name === folder.place)?.label ?? folder.place}: ${folder.path || "/"}`
      : homeRelative(folder.path)
    : undefined;
  const keptPlace = storages.data?.find((x) => x.name === placeOf(space));
  const keptOnLabel = folder
    ? folderPlace
      ? storageLabel(folderPlace)
      : (folder.place ?? s.thisComputer)
    : place?.encrypted
      ? `${where} · ${strings.settings.encrypted}`
      : where;
  const keptOn = keptPlace ? <Link to={placePath(keptPlace)}>{keptOnLabel}</Link> : keptOnLabel;
  const readAgain = async () => {
    const done = await sync.run(() => client.refreshSpace({ space: space.name }));
    forgetPreviews(space.name);
    spaces.reload();
    preview.reload();
    setContentsKey((k) => k + 1);
    if (done) toast(s.readNow(space.name));
  };
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
        mono={!folder}
        label={
          folder
            ? s.viewedOn(folderPlace ? storageLabel(folderPlace) : (folder.place ?? s.thisComputer))
            : local
              ? (kind ? s.kinds[kind] : s.fromYouShort)
              : s.sharedWithYouFrom(from)
        }
        title={<span translate="no">{collectionTitle(space)}</span>}
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
        {folder && !folder.place && (
          <Button icon="upload" onClick={() => setImporting(true)}>
            {s.addFiles}
          </Button>
        )}
        {folder && (
          <Button icon="sync" disabled={sync.busy} spinning={sync.busy} onClick={readAgain}>
            {s.readAgain}
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
              ...(folder && !folder.place
                ? [
                    {
                      label: strings.sources.showFiles,
                      onSelect: () => void sync.run(() => client.showFolder({ space: space.name })),
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
            ) : folder ? (
              <dd className="screen-facts__from">
                <span title={folder.path} translate="no">
                  {folderWhere}
                </span>
                <span className="screen-facts__note">
                  {[s.readWhereItIs, folder.scannedAtMs != null ? s.readAgo(since(folder.scannedAtMs)) : null]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              </dd>
            ) : (
              <dd>{space.example ? s.fromExample : s.fromYou}</dd>
            )}
          </div>
          <div>
            <dt className="oik-label">{s.factStored}</dt>
            <dd>{keptOn}</dd>
          </div>
          <div>
            <dt className="oik-label">{s.factWho}</dt>
            <dd className="screen-facts__who">
              <AvatarStack names={names} label={s.whoSentence(names)} you={strings.common.you} />
              <span>{listNames([strings.common.you, ...names])}</span>
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
      {importing && (
        <AddFiles
          target={{
            space: space.name,
            folder: folder ? openFolder : undefined,
            label: folder && openFolder ? `${space.name}/${openFolder}` : space.name,
          }}
          onClose={() => setImporting(false)}
          onAdded={() => {
            spaces.reload();
            preview.reload();
            setContentsKey((key) => key + 1);
          }}
        />
      )}

      {sync.error && <ErrorText>{sync.error}</ErrorText>}
      {(() => {
        const facts = {
          cameFrom: source
            ? homeRelative(source.place)
            : folderWhere
              ? folderWhere
              : space.example
                ? s.fromExample
                : s.fromYou,
          cameFromPath: folder
            ? (path: string) => {
                const root = folder.path.replace(/[\\/]+$/, "");
                const full = `${root}/${path}`;
                const shown = folder.place
                  ? `${folderPlace ? storageLabel(folderPlace) : folder.place}: ${full}`
                  : homeRelative(full);
                return (
                  <span title={full} translate="no">
                    {shown}
                  </span>
                );
              }
            : undefined,
          keptOn,
          who: listNames([strings.common.you, ...names]),
        };
        return folder ? (
          <FolderContents
            key={`${space.name}:${contentsKey}`}
            space={space}
            item={item}
            facts={facts}
            folder={openFolder}
            placeLabel={folderPlace ? storageLabel(folderPlace) : (folder.place ?? s.thisComputer)}
          />
        ) : (
          <Contents key={`${space.name}:${contentsKey}`} space={space} item={item} facts={facts} onImport={() => setImporting(true)} />
        );
      })()}
    </>
  );
}

/** What an item's own view says of its collection. */
interface Facts {
  cameFrom: ReactNode;
  cameFromPath?: (path: string) => ReactNode;
  keptOn: ReactNode;
  who: string;
}

function Contents({ space, item, facts, onImport }: { space: SpaceSummary; item?: ItemQuery; facts: Facts; onImport: () => void }) {
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
        <EmptyState
          icon="table"
          title={s.nothingYetTitle}
          body={s.nothingYetBody}
          action={
            <Button icon="upload" onClick={onImport}>
              {s.addFiles}
            </Button>
          }
        />
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
  /** The rows read, for a table or readings; photos and notes need only items. */
  table?: Table;
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
  if (!table) return null;
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

/** What a file is, in a word: a data file by its format. */
function kindLabel(name: string, kind: string): string {
  if (kind !== "table") return s.fileKinds[kind] ?? s.fileKinds.other!;
  const extension = name.split(".").pop()?.toLowerCase() ?? "";
  return extension === "parquet" ? "Parquet" : extension.startsWith("json") || extension === "ndjson" ? "JSON" : "CSV";
}

/** Files of a folder viewed in place are read a page at a time. */
const FOLDER_PAGE = 120;

/** The node's names for the orders a folder's files come in. */
const FILE_SORTS: Record<SortKey, FileSort> = { name: "name", changed: "modified", size: "size", kind: "kind" };

/** One thing in a folder, as a tile or a row: a folder, else a file. */
type Entry =
  | { folder: FileFolder }
  | { file: FileItem; item?: Item };

/**
 * The first rows of a CSV or TSV file, from the lines its index keeps: as
 * many as a tile shows, three columns each.
 */
function headRows(name: string, text: string): string[][] {
  const separator = name.toLowerCase().endsWith(".tsv") ? "\t" : ",";
  return text
    .split("\n")
    .filter((line) => line.trim())
    .slice(0, 3)
    .map((line) => {
      const cells: string[] = [];
      let cell = "";
      let quoted = false;
      for (let i = 0; i < line.length; i++) {
        const c = line[i]!;
        if (quoted) {
          if (c === '"' && line[i + 1] === '"') {
            cell += '"';
            i++;
          } else if (c === '"') quoted = false;
          else cell += c;
        } else if (c === '"') quoted = true;
        else if (c === separator) {
          cells.push(cell);
          cell = "";
        } else cell += c;
      }
      cells.push(cell);
      return cells.slice(0, 3);
    });
}

/**
 * A folder viewed in place, browsed as in a file manager: its folders
 * first, as covers like the collections outside, then its files, each
 * showing some of itself, in the order picked, as covers or a list. Each
 * folder has its own address, named in the top crumb. Every file opens at
 * its own address and ← → move through all the folder's files; a data file
 * opens as a table read where it is.
 */
function FolderContents({
  space,
  item,
  facts,
  folder,
  placeLabel,
}: {
  space: SpaceSummary;
  item?: ItemQuery;
  facts: Facts;
  /** The folder open, from the address; `""` is the top. */
  folder: string;
  /** The place it is on, named. */
  placeLabel: string;
}) {
  const client = useClient();
  const opened = item?.table === "files" && item.key.length === 1 ? item.key[0] : undefined;
  const [limit, setLimit] = useState(FOLDER_PAGE);
  // Another folder starts at its first page.
  useEffect(() => setLimit(FOLDER_PAGE), [folder]);
  const [order, setOrder] = useOrder("folder", { key: "name", descending: false });
  const [layout, setLayout] = useLayout("folder");
  const listing = useLoad(
    () =>
      client.listFiles({
        space: space.name,
        folder,
        limit,
        sort: FILE_SORTS[order.key],
        descending: order.descending,
      }),
    [space.name, folder, limit, order.key, order.descending],
  );
  const files = listing.data?.files ?? [];
  // Every file opens over the page; a photo's item carries its thumbnail.
  const openable = useMemo(() => fileItems(client, space.name, files), [client, space.name, files]);
  const itemOf = new Map(openable.map((it) => [it.path, it]));
  // Folders first, in the same order; all of one kind, they go by name then.
  const folders = sortBy(
    listing.data?.folders ?? [],
    order,
    (f, key) => (key === "changed" ? f.modifiedMs : key === "size" ? Number(f.size) : key === "name" ? f.name : null),
    (f) => f.name,
  );
  const entries: Entry[] = [
    ...folders.map((f) => ({ folder: f })),
    ...files.map((f) => ({ file: f, item: itemOf.get(f.path) })),
  ];
  const linkOf = (it: Item) =>
    it.path ? itemPath({ collection: space.name, table: "files", key: [it.path] }) : undefined;
  const open = (it: Item | undefined, replace: boolean) => {
    const to = it && linkOf(it);
    if (to) navigate(to, { replace, keepScroll: true });
  };
  // Into a folder: an address of its own, so Back comes out again.
  const into = (name: string) => navigate(folderPath(space.name, folder ? `${folder}/${name}` : name));
  const here = opened ? openable.findIndex((it) => it.path === opened) : -1;
  const nav: ItemNav | undefined =
    here >= 0
      ? {
          position: here + 1,
          total: openable.length,
          previous: here > 0 ? () => open(openable[here - 1], true) : undefined,
          next: here < openable.length - 1 ? () => open(openable[here + 1], true) : undefined,
        }
      : undefined;
  const offline = space.inPlace?.reachable === false;

  // A plain click opens the file over the page; others keep the browser's.
  const opener = (it: Item) => (e: MouseEvent<HTMLAnchorElement>) => {
    if (!plainClick(e)) return;
    e.preventDefault();
    open(it, false);
  };
  const nameOf = (entry: Entry) => ("folder" in entry ? entry.folder.name : entry.file.name);
  const iconOf = (entry: Entry): IconName =>
    "folder" in entry
      ? "folder"
      : entry.file.kind === "image"
        ? "photo"
        : entry.file.kind === "note"
          ? "note"
          : entry.file.kind === "table"
            ? "table"
            : "collection";
  const metaOf = (entry: Entry) =>
    "folder" in entry ? s.folderFiles(Number(entry.folder.files)) : bytes(Number(entry.file.size));
  const changedOf = (entry: Entry) => {
    const ms = "folder" in entry ? entry.folder.modifiedMs : entry.file.modifiedMs;
    return ms != null ? dayAndTime(Number(ms)) : "—";
  };

  return (
    <div className="screen-contents">
      {offline && <Banner title={s.offlineTitle(placeLabel)} body={s.offlineBody} />}
      <div className="screen-toolbar screen-folder-bar">
        <span className="screen-folder-bar__tools">
          <Menu
            label={s.sortBy}
            trigger={{ icon: "sort", text: s.sorts[order.key] }}
            items={SORT_KEYS.map((key) => ({
              label: s.sorts[key],
              checked: order.key === key,
              onSelect: () => {
                setOrder(nextOrder(order, key));
                setLimit(FOLDER_PAGE);
              },
            }))}
          />
          <Segmented
            label={s.layout}
            value={layout}
            onChange={(value) => setLayout(value as Layout)}
            options={[
              { value: "covers", label: s.layouts.covers, icon: "grid" },
              { value: "list", label: s.layouts.list, icon: "rows" },
            ]}
          />
        </span>
      </div>
      {listing.error && <ErrorText>{listing.error}</ErrorText>}
      {!listing.data && !listing.error && <div className="screen-page-skeleton oik-skel" aria-hidden="true" />}
      {listing.data && entries.length > 0 && layout === "covers" && (
        <>
          {folders.length > 0 && (
            <div className="screen-covers screen-subfolders">
              {folders.map((sub) => (
                <FolderCover
                  key={sub.name}
                  folder={sub}
                  items={fileItems(client, space.name, sub.cover)}
                  onOpen={() => into(sub.name)}
                />
              ))}
            </div>
          )}
          {files.length > 0 && (
            <ul className="screen-ftiles" aria-label={s.files}>
              {entries.flatMap((entry) => {
                if (!("file" in entry)) return [];
                const name = entry.file.name;
                const rows = entry.file.kind === "table" && entry.file.text ? headRows(name, entry.file.text) : [];
                // A photo shows itself, a note its first lines and a CSV its
                // first rows, as a file manager previews them.
                const art = entry.item?.image ? (
                  <span className="screen-ftile__art">
                    <img src={entry.item.image} alt="" decoding="async" loading="lazy" />
                  </span>
                ) : entry.file.kind === "note" && entry.item?.text ? (
                  <span className="screen-ftile__art screen-ftile__art--paper">
                    <span className="screen-ftile__text">{withoutTagLines(entry.item.text)}</span>
                  </span>
                ) : rows.length > 0 ? (
                  <span className="screen-ftile__art screen-ftile__art--rows">
                    <table className="screen-cover__table" aria-hidden="true">
                      <tbody>
                        {rows.map((row, r) => (
                          <tr key={r}>
                            {row.map((value, c) => (
                              <td key={c}>{value || "—"}</td>
                            ))}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </span>
                ) : (
                  <span className="screen-ftile__art screen-ftile__art--icon">
                    <Icon name={iconOf(entry)} />
                  </span>
                );
                const body = (
                  <>
                    {art}
                    <span className="screen-ftile__name" translate="no" title={name}>
                      {name}
                    </span>
                    <span className="screen-ftile__meta">{metaOf(entry)}</span>
                  </>
                );
                const href = entry.item ? linkOf(entry.item) : undefined;
                return [
                  <li key={entry.file.path}>
                    {href && entry.item ? (
                      <a className="screen-ftile" href={href} onClick={opener(entry.item)}>
                        {body}
                      </a>
                    ) : (
                      <div className="screen-ftile screen-ftile--still">{body}</div>
                    )}
                  </li>,
                ];
              })}
            </ul>
          )}
        </>
      )}
      {listing.data && entries.length > 0 && layout === "list" && (
        <div className="screen-table-card oik-table-wrap">
          <table className="oik-table screen-rows-table" aria-label={s.files}>
            <thead>
              <tr>
                <SortHeader label={s.sorts.name} sortKey="name" order={order} onSort={setOrder} />
                <SortHeader
                  label={s.sorts.kind}
                  sortKey="kind"
                  order={order}
                  onSort={setOrder}
                  className="screen-wide-only"
                />
                <SortHeader label={s.sorts.size} sortKey="size" order={order} onSort={setOrder} className="oik-num" />
                <SortHeader
                  label={s.colModified}
                  sortKey="changed"
                  order={order}
                  onSort={setOrder}
                  className="screen-wide-only"
                />
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => {
                const name = nameOf(entry);
                const it = "file" in entry ? entry.item : undefined;
                const href = it ? linkOf(it) : undefined;
                const go = "folder" in entry ? () => into(name) : it ? () => open(it, false) : undefined;
                return (
                  <tr
                    key={"folder" in entry ? `d:${name}` : `f:${entry.file.path}`}
                    className={go ? "screen-rows-table__row" : undefined}
                    onClick={(e) => {
                      if (go && !(e.target as HTMLElement).closest("a, button")) go();
                    }}
                  >
                    <td>
                      <span className="screen-rows-table__name">
                        {it?.image ? (
                          <img className="screen-rows-table__thumb" src={it.image} alt="" decoding="async" loading="lazy" />
                        ) : (
                          <Icon name={iconOf(entry)} />
                        )}
                        {"folder" in entry ? (
                          <button type="button" className="screen-rows-table__link screen-rows-table__button" onClick={go} translate="no">
                            {name}
                          </button>
                        ) : href && it ? (
                          <a className="screen-rows-table__link" href={href} onClick={opener(it)} translate="no">
                            {name}
                          </a>
                        ) : (
                          <span translate="no">{name}</span>
                        )}
                      </span>
                    </td>
                    <td className="screen-wide-only">
                      {"folder" in entry ? s.folderKind : kindLabel(entry.file.name, entry.file.kind)}
                    </td>
                    <td className="oik-num oik-table__one-line">{metaOf(entry)}</td>
                    <td className="screen-wide-only oik-table__one-line">{changedOf(entry)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {listing.data && entries.length === 0 && <p className="screen-none">{s.emptyFolder}</p>}
      {listing.data && Number(listing.data.total) > files.length && (
        <div className="screen-pager">
          <span className="screen-muted">{s.folderFiles(Number(listing.data.total))}</span>
          <Button variant="quiet" onClick={() => setLimit((n) => n + FOLDER_PAGE)}>
            {s.showMore}
          </Button>
        </div>
      )}
      {opened && (
        <FileView
          space={space.name}
          path={opened}
          file={files.find((file) => file.path === opened)}
          facts={facts}
          placeLabel={placeLabel}
          local={!space.inPlace?.place}
          nav={nav}
          onClose={() => {
            // Back to the folder it was opened from.
            const page = `/collections/${encodeURIComponent(space.name)}`;
            if (previousPage()?.split("?")[0] === page) window.history.back();
            else navigate(folderPath(space.name, folder), { replace: true, keepScroll: true });
          }}
        />
      )}
    </div>
  );
}
