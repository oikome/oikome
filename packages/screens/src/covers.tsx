// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A collection's cover: what is in it, not what it is made of. Photos show
// a collage, notes their first lines, anything else its first rows.

import { AvatarStack, Icon, Tile } from "@oikome/ui";
import type { TileTone } from "@oikome/ui";
import type { FileFolder, Share, SpaceSummary } from "@oikome/client";
import type { Table } from "apache-arrow";
import { PREVIEW_ROWS, loadPreview, withoutTagLines } from "./content";
import type { Item, Preview } from "./content";
import { useClient, useLoad } from "./data";
import { calendarDate, dateTime, monthAndDay, wallTime } from "./format";
import { SeriesCover } from "./series";
import { numeric, show } from "./parts";
import { Link } from "./router";
import { strings } from "./strings";

const s = strings.collections;
const KIND_ICONS = { photos: "photo", notes: "note", table: "table", series: "activity" } as const;

/** Contact names each collection is shared with now. */
export function sharedNames(shares: Share[] | undefined): Map<string, string[]> {
  const names = new Map<string, string[]>();
  for (const share of shares ?? []) {
    if (share.status !== "active") continue;
    const list = names.get(share.space) ?? [];
    const name = share.contactName ?? strings.common.unknown;
    if (!list.includes(name)) list.push(name);
    names.set(share.space, list);
  }
  return names;
}

/** When an item was made, the way people write it. */
export function itemDate(item: Item): string | undefined {
  if (item.when === undefined) return undefined;
  if (item.whenKind === "day") return calendarDate(item.when);
  return item.whenKind === "wall" ? wallTime(item.when) : dateTime(item.when);
}

/** "6 photos", "7 notes", "10 rows": what a collection holds, when known. */
export function countOf(preview: Preview | undefined): string | undefined {
  // Another data center's tables are not counted just to list them; when
  // the first read held all of a table's rows, that is its count.
  const read = preview?.rows?.numRows;
  const rows = preview?.count ?? preview?.table?.rows ?? (read !== undefined && read < PREVIEW_ROWS ? read : undefined);
  if (rows === null || rows === undefined || rows === 0) return undefined;
  return s.count(Number(rows), preview!.shape.view);
}

function MiniTable({ table }: { table: Table }) {
  const fields = table.schema.fields.slice(0, 3);
  const rows = Math.min(table.numRows, 4);
  return (
    <table className="screen-cover__table" aria-hidden="true">
      <tbody>
        {Array.from({ length: rows }, (_, r) => (
          <tr key={r}>
            {fields.map((f, c) => (
              <td key={f.name} className={numeric(f) ? "oik-num" : undefined}>
                {show(f, table.getChildAt(c)?.get(r)) ?? "—"}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function Art({
  tone,
  preview,
  failed,
}: {
  tone: TileTone;
  preview?: Preview;
  /** It could not be read: shared from a data center that is off, or no longer shared. */
  failed?: boolean;
}) {
  if (failed) {
    return (
      <div aria-hidden="true" className={`screen-cover__art screen-cover__art--plain screen-cover__art--${tone}`}>
        <Tile icon="collection" tone={tone} large />
      </div>
    );
  }
  if (!preview) return <div aria-hidden="true" className="screen-cover__art oik-skel" />;
  const { shape, items } = preview;
  if (items.length === 0) {
    return (
      <div aria-hidden="true" className="screen-cover__art screen-cover__art--plain">
        <Tile icon="collection" tone={tone} large />
        <span className="screen-cover__empty">{s.noData}</span>
      </div>
    );
  }
  if (shape.view === "photos") {
    const photos = items.filter((item) => item.image).slice(0, 3);
    return (
      <div aria-hidden="true" className={`screen-cover__art screen-collage screen-collage--${photos.length}`}>
        {photos.map((item, i) => (
          <img key={i} src={item.image} alt="" decoding="async" />
        ))}
      </div>
    );
  }
  if (shape.view === "notes") {
    const first = items[0]!;
    const more = Number(preview.table?.rows ?? items.length) - 1;
    return (
      <div aria-hidden="true" className="screen-cover__art screen-paper">
        {itemDate(first) && <span className="screen-paper__date">{itemDate(first)}</span>}
        <p className="screen-paper__text">{first.text}</p>
        {more > 0 && <span className="screen-paper__more">{s.moreNotes(more)}</span>}
      </div>
    );
  }
  if (shape.view === "series" && preview.rows) return <SeriesCover table={preview.rows} shape={shape} />;
  return (
    <div aria-hidden="true" className="screen-cover__art screen-cover__art--table">
      {preview.rows && <MiniTable table={preview.rows} />}
    </div>
  );
}

/**
 * What a collection is called: its name, or for a folder viewed in place
 * the folder's own name, in whatever script it is written.
 */
export function collectionTitle(space: SpaceSummary): string {
  const path = space.inPlace?.path.replace(/[\\/]+$/, "");
  return (path && path.split(/[\\/]/).pop()) || space.name;
}

/** Whom a collection shared with you is from: their contact name, else words for it. */
export function fromOf(space: SpaceSummary): string {
  return space.fromName ?? s.fromElsewhere;
}

/** A collection as a cover; the whole card opens it. */
export function Cover({
  space,
  tone,
  names,
  place,
}: {
  space: SpaceSummary;
  tone: TileTone;
  names: string[];
  /** Where it is kept, named: said first for a folder viewed in place. */
  place?: string;
}) {
  const client = useClient();
  const preview = useLoad(() => loadPreview(client, space), [space.name]);
  const from = fromOf(space);
  // A folder viewed where it is looks like one: so its label says, and
  // its line starts with where it is.
  const folder = space.kind !== "remote" && space.inPlace !== null;
  const meta = [
    folder ? place : undefined,
    countOf(preview.data),
    ...(space.kind === "remote"
      ? [s.fromPerson(from), s.viewLive]
      : [names.length > 0 ? s.sharedWith(names) : s.notShared]),
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <Link to={`/collections/${encodeURIComponent(space.name)}`} className="screen-cover">
      <span className="screen-cover__frame">
        <Art tone={tone} preview={preview.data} failed={preview.error !== undefined} />
        <span className="screen-cover__chip" aria-hidden="true">
          <Icon
            name={
              folder ? "folder" : space.kind === "remote" ? "person-plus" : KIND_ICONS[preview.data?.shape.view ?? "table"]
            }
          />
          {folder ? s.folderKind : space.kind === "remote" ? from : s.kinds[preview.data?.shape.view ?? "table"]}
        </span>
      </span>
      <span className="screen-cover__caption">
        <span className="screen-cover__line">
          {folder ? (
            <span className="screen-cover__name screen-cover__name--folder" translate="no">
              <Icon name="folder" />
              <span className="screen-cover__title">{collectionTitle(space)}</span>
            </span>
          ) : (
            <span className="screen-cover__name" translate="no">
              {collectionTitle(space)}
            </span>
          )}
          {space.kind !== "remote" && (
            <span className="screen-cover__who" aria-hidden="true">
              <AvatarStack names={names} label={s.whoSentence(names)} you={strings.common.you} />
            </span>
          )}
        </span>
        <span className="screen-cover__meta">{meta}</span>
      </span>
    </Link>
  );
}

/**
 * A folder inside a folder viewed in place, as a collection of its own: its
 * cover shows its newest photos or its newest note, the folder sign only
 * beside its name. The whole card opens it.
 */
export function FolderCover({
  folder,
  items,
  onOpen,
}: {
  folder: FileFolder;
  /** What its cover shows, as items: photos with their thumbnails, or a note. */
  items: Item[];
  onOpen: () => void;
}) {
  const photos = items.filter((item) => item.image).slice(0, 3);
  const note = photos.length === 0 ? items.find((item) => item.text) : undefined;
  const holds =
    folder.photos > 0
      ? s.count(Number(folder.photos), "photos")
      : folder.notes > 0
        ? s.count(Number(folder.notes), "notes")
        : s.folderFiles(Number(folder.files));
  const meta = [holds, folder.modifiedMs != null ? monthAndDay(Number(folder.modifiedMs)) : undefined]
    .filter(Boolean)
    .join(" · ");
  return (
    <button type="button" className="screen-cover screen-cover--button" onClick={onOpen}>
      <span className="screen-cover__frame">
        {photos.length > 0 ? (
          <span aria-hidden="true" className={`screen-cover__art screen-collage screen-collage--${photos.length}`}>
            {photos.map((item, i) => (
              <img key={i} src={item.image} alt="" decoding="async" loading="lazy" />
            ))}
          </span>
        ) : note ? (
          <span aria-hidden="true" className="screen-cover__art screen-paper">
            {itemDate(note) && <span className="screen-paper__date">{itemDate(note)}</span>}
            <span className="screen-paper__text">{withoutTagLines(note.text)}</span>
            {folder.notes > 1 && <span className="screen-paper__more">{s.moreNotes(Number(folder.notes) - 1)}</span>}
          </span>
        ) : (
          <span aria-hidden="true" className="screen-cover__art screen-cover__art--plain">
            <span className="screen-cover__empty">{s.folderFiles(Number(folder.files))}</span>
          </span>
        )}
      </span>
      <span className="screen-cover__caption">
        <span className="screen-cover__line">
          <span className="screen-cover__name screen-cover__name--folder" translate="no">
            <Icon name="folder" />
            <span className="screen-cover__title">{folder.name}</span>
          </span>
        </span>
        <span className="screen-cover__meta">{meta}</span>
      </span>
    </button>
  );
}
