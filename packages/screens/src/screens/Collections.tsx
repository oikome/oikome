// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Collections: every collection as a cover or a row, by place and in the
// order the owner picks, and making a new one.

import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import {
  Button,
  Card,
  Dialog,
  EmptyState,
  ErrorText,
  Icon,
  Menu,
  Segmented,
  Select,
  TextField,
  collectionTones,
  useToast,
} from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { SpaceSummary, StorageSummary } from "@oikome/client";
import { forgetPreviews, loadPreview } from "../content";
import type { View } from "../content";
import { Cover, collectionTitle, fromOf, sharedNames } from "../covers";
import { useAction, useClient, useLoad } from "../data";
import { bytes, dayAndTime, homeRelative } from "../format";
import { SORT_KEYS, SortHeader, nextOrder, sortBy, useLayout, useOrder } from "../order";
import type { Layout, Order } from "../order";
import { ScreenHeader } from "../parts";
import { Link, navigate } from "../router";
import { strings } from "../strings";
import { AddFiles } from "../import";

const s = strings.collections;
/** The node's rule for a name: ASCII letters, digits, _ and -, not led by a digit, 1 to 64. */
const NAME = /^[A-Za-z_-][A-Za-z0-9_-]{0,63}$/;

export const STORAGE_ICONS: Record<StorageSummary["type"], IconName> = {
  disk: "computer",
  webdav: "nas",
  folder: "folder",
  drive: "drive",
  s3: "bucket",
  gdrive: "drive",
  dropbox: "drive",
};

/** What a storage is called: this computer, or the name the owner gave it. */
export function storageLabel(storage: StorageSummary): string {
  return storage.name === null ? s.thisComputer : storage.label;
}

export function Collections({ startCreating = false }: { startCreating?: boolean }) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const storages = useLoad(() => client.listStorages(), []);
  const [creating, setCreating] = useState(startCreating);
  const [importing, setImporting] = useState(false);
  useEffect(() => {
    if (startCreating) setCreating(true);
  }, [startCreating]);

  // What to show: everything, one kind, or the folders viewed in place.
  const [show, setShow] = useState<"all" | View | "folder">("all");
  // "all", or a place: "" for this computer, else the storage's name.
  const [place, setPlace] = useState<string>("all");
  const [order, setOrder] = useOrder("collections", { key: "changed", descending: true });
  const [layout, setLayout] = useLayout("collections");
  const [kinds, setKinds] = useState<Map<string, View>>(new Map());
  const shared = sharedNames(shares.data);
  const list = spaces.data ?? [];
  const places = storages.data ?? [];
  // What each holds, from the same first rows its cover reads (the same
  // read: previews are kept). One shared with you may not answer.
  useEffect(() => {
    let current = true;
    for (const space of spaces.data ?? []) {
      loadPreview(client, space)
        .then((preview) => {
          if (current) setKinds((m) => new Map(m).set(space.name, preview.shape.view));
        })
        .catch(() => undefined);
    }
    return () => {
      current = false;
    };
  }, [client, spaces.data]);
  // Readings are a table over time: Tables shows them too.
  const shown = (space: SpaceSummary) => {
    if (show === "folder") return space.kind !== "remote" && space.inPlace !== null;
    const kind = kinds.get(space.name);
    return show === "all" || kind === show || (show === "table" && kind === "series");
  };
  // A place removed since it was picked shows everything again.
  const picked = place !== "all" && places.some((x) => (x.name ?? "") === place) ? place : "all";
  const here = (space: SpaceSummary) => picked === "all" || (placeOf(space) ?? "") === picked;
  const placeName = (name: string | null) => {
    const found = places.find((x) => x.name === name);
    return found ? storageLabel(found) : (name ?? s.thisComputer);
  };
  const kindName = (space: SpaceSummary) => {
    if (space.inPlace) return s.folderKind;
    const kind = kinds.get(space.name);
    return kind ? s.kinds[kind] : undefined;
  };
  const sorted = (from: SpaceSummary[]) =>
    sortBy(
      from,
      order,
      (space, key) =>
        key === "name"
          ? collectionTitle(space)
          : key === "changed"
            ? space.changedAtMs
            : key === "size"
              ? space.sizeBytes
              : kindName(space),
      collectionTitle,
    );
  const mine = list.filter((space) => space.kind !== "remote");
  const fromOthers = list.filter((space) => space.kind === "remote");
  const mineShown = sorted(mine.filter(shown).filter(here));
  // Shared with you is kept on the other side's computer, at no place of yours.
  const theirsShown = picked === "all" ? sorted(fromOthers.filter(shown)) : [];
  const toneOf = collectionTones(list.map((space) => space.name));
  const empty = spaces.data !== undefined && list.length === 0;

  const stopCreating = () => {
    setCreating(false);
    // Drop ?new=1, so a reload does not open the form again.
    if (startCreating) navigate(window.location.pathname, { replace: true });
  };

  const present = (from: SpaceSummary[], theirs: boolean) =>
    layout === "list" ? (
      <CollectionRows
        spaces={from}
        order={order}
        onSort={setOrder}
        kindName={kindName}
        where={(space) =>
          theirs ? (
            s.fromPerson(fromOf(space))
          ) : (
            <>
              {placeName(placeOf(space))}
              {space.inPlace && <span className="screen-muted"> · {s.viewedInPlace}</span>}
            </>
          )
        }
      />
    ) : (
      <div className="screen-covers">
        {from.map((space) => (
          <Cover
            key={space.name}
            space={space}
            tone={toneOf(space.name)}
            names={theirs ? [] : (shared.get(space.name) ?? [])}
            place={theirs ? undefined : placeName(placeOf(space))}
          />
        ))}
      </div>
    );

  return (
    <>
      <ScreenHeader title={s.title} lede={spaces.data && !empty ? s.lede(mine.length, fromOthers.length) : undefined}>
        {/* An empty page offers the same actions once, in its empty state. */}
        {!empty && (
          <Button icon="upload" onClick={() => setImporting(true)}>
            {s.addFiles}
          </Button>
        )}
        {!empty && (
          <Button variant="primary" icon="plus" onClick={() => setCreating(true)}>
            {s.newCollection}
          </Button>
        )}
      </ScreenHeader>

      {creating && <NewCollection onClose={stopCreating} onCreated={spaces.reload} />}
      {importing && <AddFiles onClose={() => setImporting(false)} onAdded={() => spaces.reload()} />}

      {spaces.data && !empty && (
        <div className="screen-toolbar screen-cols-tools">
          {list.length > 1 && (
            <Segmented
              label={s.filter}
              value={show}
              onChange={(value) => setShow(value as "all" | View | "folder")}
              options={(["all", "photos", "notes", "table", "folder"] as const).map((k) => ({
                value: k,
                label: s.filters[k],
              }))}
            />
          )}
          {places.length > 1 && (
            <Menu
              label={s.places}
              trigger={{
                icon: picked === "all" ? "collection" : STORAGE_ICONS[places.find((x) => (x.name ?? "") === picked)!.type],
                text: picked === "all" ? s.allPlaces : placeName(picked || null),
              }}
              items={[
                { label: s.allPlaces, icon: "collection", checked: picked === "all", onSelect: () => setPlace("all") },
                ...places.map((x) => ({
                  label: storageLabel(x),
                  icon: STORAGE_ICONS[x.type],
                  checked: picked === (x.name ?? ""),
                  onSelect: () => setPlace(x.name ?? ""),
                })),
              ]}
            />
          )}
          {list.length > 1 && (
            <Menu
              label={s.sortBy}
              trigger={{ icon: "sort", text: s.sorts[order.key] }}
              items={SORT_KEYS.map((key) => ({
                label: s.sorts[key],
                checked: order.key === key,
                onSelect: () => setOrder(nextOrder(order, key)),
              }))}
            />
          )}
          <Segmented
            label={s.layout}
            value={layout}
            onChange={(value) => setLayout(value as Layout)}
            options={[
              { value: "covers", label: s.layouts.covers, icon: "grid" },
              { value: "list", label: s.layouts.list, icon: "rows" },
            ]}
          />
        </div>
      )}

      {spaces.error && <ErrorText>{spaces.error}</ErrorText>}
      {!spaces.data && !spaces.error && (
        <div className="screen-covers" aria-hidden="true">
          {[0, 1, 2].map((i) => (
            <div key={i} className="screen-cover">
              <div className="screen-cover__art oik-skel" />
            </div>
          ))}
        </div>
      )}
      {empty && (
        <Card>
          <EmptyState
            icon="collection"
            title={s.emptyTitle}
            body={s.emptyBody}
            action={
              <>
                <Button icon="upload" onClick={() => setImporting(true)}>
                  {s.addFiles}
                </Button>
                <Button variant="primary" icon="plus" onClick={() => setCreating(true)}>
                  {s.newCollection}
                </Button>
              </>
            }
          />
        </Card>
      )}

      {mine.length > 0 && (
        <section className="screen-section" aria-labelledby="mine">
          <h2 id="mine" className="oik-label screen-section__title">
            {s.yours}
          </h2>
          {mineShown.length > 0 ? present(mineShown, false) : <p className="screen-none">{s.noneMine}</p>}
        </section>
      )}
      {fromOthers.length > 0 && picked === "all" && (
        <section className="screen-section" aria-labelledby="from-others">
          <h2 id="from-others" className="oik-label screen-section__title">
            {s.sharedWithYou}
          </h2>
          {theirsShown.length > 0 ? present(theirsShown, true) : <p className="screen-none">{s.noneTheirs}</p>}
        </section>
      )}
    </>
  );
}

/** Where a collection is: the storage's name, `null` for this computer. */
export function placeOf(space: SpaceSummary): string | null {
  return space.inPlace ? space.inPlace.place : space.storage;
}

/** Collections as a list: a row each, sorted by a column heading. */
function CollectionRows({
  spaces,
  order,
  onSort,
  kindName,
  where,
}: {
  spaces: SpaceSummary[];
  order: Order;
  onSort: (order: Order) => void;
  kindName: (space: SpaceSummary) => string | undefined;
  where: (space: SpaceSummary) => ReactNode;
}) {
  return (
    <div className="screen-table-card oik-table-wrap">
      <table className="oik-table screen-rows-table">
        <thead>
          <tr>
            <SortHeader label={s.sorts.name} sortKey="name" order={order} onSort={onSort} />
            <SortHeader label={s.sorts.kind} sortKey="kind" order={order} onSort={onSort} className="screen-wide-only" />
            <th scope="col" className="screen-wide-only">
              {s.colPlace}
            </th>
            <SortHeader label={s.sorts.size} sortKey="size" order={order} onSort={onSort} className="oik-num" />
            <SortHeader
              label={s.sorts.changed}
              sortKey="changed"
              order={order}
              onSort={onSort}
              className="screen-wide-only"
            />
          </tr>
        </thead>
        <tbody>
          {spaces.map((space) => {
            const to = `/collections/${encodeURIComponent(space.name)}`;
            const kind = kindName(space);
            return (
              // The whole row opens it; its name is the link, for the keyboard.
              <tr
                key={space.name}
                className="screen-rows-table__row"
                onClick={(e) => {
                  if (!(e.target as HTMLElement).closest("a")) navigate(to);
                }}
              >
                <td>
                  <span className="screen-rows-table__name">
                    <Icon name={space.kind === "remote" ? "person-plus" : space.inPlace ? "folder" : "collection"} />
                    <Link to={to} className="screen-rows-table__link">
                      <span className={space.inPlace ? undefined : "screen-rows-table__mono"} translate="no">
                        {collectionTitle(space)}
                      </span>
                    </Link>
                  </span>
                </td>
                <td className="screen-wide-only">{kind ?? "—"}</td>
                <td className="screen-wide-only">{where(space)}</td>
                <td className="oik-num oik-table__one-line">
                  {space.sizeBytes != null ? bytes(Number(space.sizeBytes)) : "—"}
                </td>
                <td className="screen-wide-only oik-table__one-line">
                  {space.changedAtMs != null ? dayAndTime(Number(space.changedAtMs)) : "—"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

/** Making a collection is a short task in a dialog: a name, and where it is kept. */
function NewCollection({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const client = useClient();
  const storages = useLoad(() => client.listStorages(), []);
  const [name, setName] = useState("");
  // "" is this computer, the default.
  const [storage, setStorage] = useState("");
  const create = useAction();
  const toast = useToast();
  const choices = storages.data ?? [];
  const here = choices.find((x) => x.name === null);
  // The default place is chosen already.
  useEffect(() => {
    const preferred = storages.data?.find((x) => x.isDefault);
    if (preferred) setStorage(preferred.name ?? "");
  }, [storages.data]);

  return (
    <Dialog
      title={s.newCollection}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button type="submit" form="new-collection" variant="primary" disabled={create.busy}>
            {s.create}
          </Button>
        </>
      }
    >
      <form
        id="new-collection"
        onSubmit={async (e) => {
          e.preventDefault();
          const wanted = name.trim();
          if (!wanted) {
            create.fail(strings.common.nameRequired);
            return;
          }
          if (!NAME.test(wanted)) {
            create.fail(s.nameInvalid);
            return;
          }
          const space = await create.run(
            () => client.createSpace({ name: wanted, storage: storage || undefined }),
            (err) => (err.code === "conflict" ? s.nameTaken(wanted) : undefined),
          );
          if (space) {
            toast(strings.toast.created(space.name));
            forgetPreviews();
            onCreated();
            onClose();
            navigate(`/collections/${encodeURIComponent(space.name)}`);
          }
        }}
      >
        <TextField
          label={s.nameLabel}
          name="collection-name"
          hint={s.nameHint}
          placeholder={s.namePlaceholder}
          value={name}
          onChange={(value) => {
            setName(value);
            create.clear();
          }}
          error={create.error}
          autoFocus
        />
        {/* The default is already chosen; a choice appears once there is one to make. */}
        {choices.length > 1 ? (
          <Select
            label={s.storeOn}
            value={storage}
            onChange={setStorage}
            leading={
              <Icon name={STORAGE_ICONS[choices.find((x) => (x.name ?? "") === storage)?.type ?? "disk"]} />
            }
            options={choices.map((x) => ({ value: x.name ?? "", label: storageLabel(x) }))}
          />
        ) : (
          <div className="oik-field">
            <span className="oik-field__label">{s.storeOn}</span>
            <div className="screen-storage-fixed">
              <Icon name="computer" />
              <span>{s.thisComputer}</span>
              {here && (
                <span className="screen-path" title={here.place} translate="no">
                  {homeRelative(here.place)}
                </span>
              )}
            </div>
            <span className="oik-field__hint">{s.storeHint}</span>
          </div>
        )}
      </form>
    </Dialog>
  );
}
