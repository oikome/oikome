// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A collection's cover: what is in it, not what it is made of. Photos show
// a collage, notes their first lines, anything else its first rows.

import { AvatarStack, Tile } from "@oikome/ui";
import type { TileTone } from "@oikome/ui";
import type { Share, SpaceSummary } from "@oikome/client";
import type { Table } from "apache-arrow";
import { loadPreview } from "./content";
import type { Item, Preview } from "./content";
import { useClient, useLoad } from "./data";
import { calendarDate, columnLabel, dateTime, wallTime } from "./format";
import { numeric, show } from "./parts";
import { Link } from "./router";
import { strings } from "./strings";

const s = strings.collections;

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
  const rows = preview?.table?.rows;
  if (rows === null || rows === undefined || rows === 0) return undefined;
  return s.count(Number(rows), preview!.shape.view);
}

function MiniTable({ table }: { table: Table }) {
  const fields = table.schema.fields.slice(0, 3);
  const rows = Math.min(table.numRows, 4);
  return (
    <table className="screen-cover__table" aria-hidden="true">
      <thead>
        <tr>
          {fields.map((f) => (
            <th key={f.name} className={numeric(f) ? "oik-num" : undefined}>
              {columnLabel(f.name)}
            </th>
          ))}
        </tr>
      </thead>
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

function Art({ space, tone, preview }: { space: SpaceSummary; tone: TileTone; preview?: Preview }) {
  if (space.kind === "remote") {
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
  return (
    <div aria-hidden="true" className="screen-cover__art screen-cover__art--table">
      {preview.rows && <MiniTable table={preview.rows} />}
    </div>
  );
}

/** A collection as a cover; the whole card opens it. */
export function Cover({
  space,
  tone,
  names,
}: {
  space: SpaceSummary;
  tone: TileTone;
  names: string[];
}) {
  const client = useClient();
  const preview = useLoad(() => loadPreview(client, space), [space.name]);
  const meta = [
    countOf(preview.data),
    space.kind === "remote" ? s.fromElsewhere : names.length > 0 ? s.sharedWith(names) : s.notShared,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <Link to={`/collections/${encodeURIComponent(space.name)}`} className="screen-cover">
      <Art space={space} tone={tone} preview={preview.data} />
      <span className="screen-cover__caption">
        <span className="screen-cover__line">
          <span className="screen-cover__name">{space.name}</span>
          {space.example && <span className="screen-tag">{s.example}</span>}
          {space.kind !== "remote" && (
            <span className="screen-cover__who" aria-hidden="true">
              <AvatarStack names={names} label={s.whoSentence(names)} />
            </span>
          )}
        </span>
        <span className="screen-cover__meta">{meta}</span>
      </span>
    </Link>
  );
}
