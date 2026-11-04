// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Home: the data center at a glance (what it holds, where, who reads it),
// what came in last, a photo from what it keeps, and the one thing that
// needs doing.

import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Avatar, Button, Status, Tile, collectionTones, useToast } from "@oikome/ui";
import type { IconName, TileTone } from "@oikome/ui";
import type { AccessItem, HomeCard, NodeInfo, SpaceSummary, StorageSummary } from "@oikome/client";
import { latest, loadPreview } from "../content";
import type { Item, Preview } from "../content";
import { itemDate } from "../covers";
import { AddCardDialog, CardGrid, settle } from "../arrange";
import type { CardChoice } from "../arrange";
import { LatestCard, TotalsCard, hasLatest, hasTotals } from "../cards";
import { useAction, useClient, useLoad } from "../data";
import { useCountUp } from "../motion";
import { PluginCard } from "../plugins";
import { ago, bytes, shortId } from "../format";
import { ScreenHeader } from "../parts";
import { Link, navigate } from "../router";
import { strings } from "../strings";
import { sentence } from "./Activity";

const s = strings.home;
const WEEK_MS = 7 * 24 * 60 * 60 * 1000;
/** Items in Recently Added, and lines of Activity, on Home. */
const RECENT = 5;
const LOG = 5;

const today = new Intl.DateTimeFormat("en", { weekday: "long", month: "long", day: "numeric" });
const short = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const dateShort = (ms: number) => short.format(new Date(ms));

interface Recent {
  space: SpaceSummary;
  view: Preview["shape"]["view"];
  item: Item;
}

const PLACE_ICONS: Record<StorageSummary["type"], IconName> = {
  disk: "computer",
  webdav: "nas",
  folder: "folder",
  drive: "drive",
};

export function Home({ info, onInfo }: { info: NodeInfo; onInfo: (info: NodeInfo) => void }) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const storages = useLoad(() => client.listStorages(), []);
  const sources = useLoad(() => client.listSources(), []);
  const plugins = useLoad(() => client.listPlugins(), []);
  // People's reads: a plugin's reads of its own cards are in Activity, not here.
  const access = useLoad(() => client.listAccess({ limit: 100 }), []);
  const log = { ...access, data: access.data?.filter((item) => item.method !== "plugin_read") };
  const [previews, setPreviews] = useState<Map<string, Preview>>();
  const [editing, setEditing] = useState(false);
  const [adding, setAdding] = useState(false);
  // The arrangement as last changed here, until the node has it.
  const [placed, setPlaced] = useState<HomeCard[]>();
  const saves = useRef<{ sending: boolean; next?: HomeCard[] }>({ sending: false });
  const arrange = useAction();
  const toast = useToast();

  const list = spaces.data ?? [];
  const mine = list.filter((space) => space.kind !== "remote");
  const toneOf = collectionTones(list.map((space) => space.name));

  useEffect(() => {
    if (!spaces.data) return;
    let current = true;
    const local = spaces.data.filter((space) => space.kind !== "remote");
    Promise.all(local.map((space) => loadPreview(client, space).catch(() => undefined))).then(
      (loaded) => {
        if (!current) return;
        const map = new Map<string, Preview>();
        loaded.forEach((preview, i) => preview && map.set(local[i]!.name, preview));
        setPreviews(map);
      },
    );
    return () => {
      current = false;
    };
  }, [client, spaces.data]);

  const items = previews
    ? [...previews.values()].reduce((n, p) => n + Number(p.table?.rows ?? 0), 0)
    : undefined;
  const active = (shares.data ?? []).filter((x) => x.status === "active");
  const people = new Set(active.map((x) => x.did)).size;
  const activeShares = active.length;
  const now = Date.now();
  const reads = log.data?.filter((x) => x.outcome === "allowed" && now - x.atMs < WEEK_MS).length;
  const places = storages.data ?? [];
  const sourceList = sources.data ?? [];
  const failing = sourceList.filter((x) => x.failedAtMs != null).length;
  const preferred = places.find((x) => x.isDefault);

  // The newest few things across collections, by their own dates.
  const all: Recent[] = previews
    ? mine.flatMap((space) => {
        const preview = previews.get(space.name);
        if (!preview) return [];
        // Readings come every hour: only the latest one stands for them.
        const items = preview.shape.view === "series" ? [latest(preview.items)].filter((x) => x !== undefined) : preview.items;
        return items.map((item) => ({ space, view: preview.shape.view, item }));
      })
    : [];
  const recent = [...all].sort((a, b) => (b.item.when ?? 0) - (a.item.when ?? 0)).slice(0, RECENT);
  // One photo from what is kept: the latest one.
  const photo = latest(all.filter((r) => r.item.image).map((r) => r.item));
  const photoFrom = all.find((r) => r.item === photo)?.space;


  // Home's cards: the owner's arrangement, or a first one made from what is
  // kept. Widths are columns of twelve; each row of the first one fills.
  const installed = (plugins.data ?? []).filter((plugin) => plugin.installed);
  const totals = mine.filter((space) => hasTotals(previews?.get(space.name)));
  const readings = mine.filter((space) => hasLatest(previews?.get(space.name)));
  const available: CardChoice[] = [
    { id: "recent", label: s.cardRecent, icon: "note", width: 8 },
    { id: "photo", label: s.cardPhoto, icon: "photo", width: 4 },
    ...totals.map((space): CardChoice => ({
      id: `totals:${space.name}`,
      label: s.cardTotals(space.name),
      icon: "table",
      width: 7,
    })),
    ...readings.map((space): CardChoice => ({
      id: `latest:${space.name}`,
      label: s.cardLatest(space.name),
      icon: "activity",
      width: 5,
    })),
    { id: "activity", label: s.cardActivity, icon: "person-plus", width: 7 },
    { id: "places", label: s.cardPlaces, icon: "nas", width: 5 },
    ...installed.map((plugin): CardChoice => ({
      id: `plugin:${plugin.id}`,
      label: plugin.name,
      icon: "plugin",
      width: plugin.width,
    })),
  ];
  const choiceOf = (id: string) => available.find((card) => card.id === id);
  const first = [
    "recent",
    "photo",
    ...(totals[0] ? [`totals:${totals[0].name}`] : []),
    ...(readings[0] ? [`latest:${readings[0].name}`] : []),
    "activity",
    "places",
    ...installed.map((plugin) => `plugin:${plugin.id}`),
  ].map((id): HomeCard => ({ id, width: null }));
  const order: HomeCard[] = placed ?? info.homeCards ?? first;
  const shown = settle(
    order.flatMap((card) => {
      const choice = choiceOf(card.id);
      return choice ? [{ id: card.id, width: card.width, own: choice.width }] : [];
    }),
  );
  const spaceOf = (id: string) => mine.find((space) => space.name === id.slice(id.indexOf(":") + 1));

  /**
   * Keep a change at once, shown before the node answers. Changes go to the
   * node one at a time, the newest last; one it turns down puts back what
   * it has.
   */
  const save = async (cards: HomeCard[]) => {
    setPlaced(cards);
    saves.current.next = cards;
    if (saves.current.sending) return;
    saves.current.sending = true;
    let kept: NodeInfo | undefined;
    let next: HomeCard[] | undefined;
    while ((next = saves.current.next)) {
      const cards = next;
      saves.current.next = undefined;
      kept = await arrange.run(() => client.setHomeCards({ cards }));
      if (!kept) {
        saves.current.next = undefined;
        break;
      }
    }
    saves.current.sending = false;
    if (kept) onInfo(kept);
    setPlaced(undefined);
  };
  // A change the node turned down goes back, and says why.
  const failed = arrange.error;
  const clearFailure = arrange.clear;
  useEffect(() => {
    if (!failed) return;
    toast(failed);
    clearFailure();
  }, [failed, toast, clearFailure]);

  // Escape ends arranging, as Done does.
  useEffect(() => {
    if (!editing || adding) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setEditing(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [editing, adding]);
  const renderCard = (id: string): ReactNode => {
    if (id === "recent") {
      return (
        <Panel title={s.recent} link={{ to: "/collections", label: s.allCollections }}>
                {!previews ? (
                  <div className="screen-panel__skeleton oik-skel" aria-hidden="true" />
                ) : recent.length === 0 ? (
                  <p className="screen-panel__empty">{s.nothingRecent}</p>
                ) : (
                  <ul className="screen-list">
                    {recent.map((r, i) => (
                      <RecentRow key={i} recent={r} tone={toneOf(r.space.name)} />
                    ))}
                  </ul>
                )}
              </Panel>
      );
    }
    if (id === "photo") {
      return photo?.image && photoFrom ? (
              <Link
                to={`/collections/${encodeURIComponent(photoFrom.name)}`}
                className="oik-card screen-memory screen-lift"
              >
                <img src={photo.image} alt="" decoding="async" />
                <span className="screen-memory__cap">
                  <span className="oik-label">{s.fromPhotos(photoFrom.name)}</span>
                  <span className="screen-memory__title">{photo.text ?? photoFrom.name}</span>
                  {itemDate(photo) && <span className="screen-memory__meta">{itemDate(photo)}</span>}
                </span>
              </Link>
            ) : (
              <Panel title={s.backedUp}>
                <p className="screen-panel__empty">
                  {info.recoveryKeySaved ? s.keySaved : strings.backup.bannerBody}
                </p>
              </Panel>
            );
    }
    if (id === "activity") {
      return (
        <Panel title={s.activity} link={{ to: "/activity", label: s.allActivity }}>
                {!log.data ? (
                  <div className="screen-panel__skeleton oik-skel" aria-hidden="true" />
                ) : log.data.length === 0 ? (
                  <p className="screen-panel__empty">{s.noActivity}</p>
                ) : (
                  <ul className="screen-list">
                    {log.data.slice(0, LOG).map((item) => (
                      <LogRow key={item.id} item={item} />
                    ))}
                  </ul>
                )}
              </Panel>
      );
    }
    if (id === "places") {
      return (
        <Panel title={strings.nav.places} link={{ to: "/settings", label: s.manage }}>
                <ul className="screen-list">
                  {places.map((place) => (
                    <PlaceRow key={place.name ?? ""} place={place} />
                  ))}
                </ul>
              </Panel>
      );
    }
    if (id.startsWith("plugin:")) {
      const plugin = installed.find((p) => `plugin:${p.id}` === id);
      return plugin ? <PluginCard plugin={plugin} /> : null;
    }
    const space = spaceOf(id);
    if (!space) return null;
    if (id.startsWith("totals:")) return <TotalsCard space={space} />;
    if (id.startsWith("latest:")) return <LatestCard space={space} preview={previews?.get(space.name)} />;
    return null;
  };

  return (
    <>
      <ScreenHeader
        label={today.format(new Date())}
        title={s.greeting(new Date().getHours())}
        lede={info.recoveryKeySaved ? s.allWell : s.allWellBut}
      >
        {editing ? (
          <Button variant="primary" icon="check" onClick={() => setEditing(false)}>
            {s.arrangeDone}
          </Button>
        ) : (
          <Button icon="drag" disabled={!previews || !plugins.data} onClick={() => setEditing(true)}>
            {s.arrange}
          </Button>
        )}
        <Button
          variant={editing ? "secondary" : "primary"}
          icon="plus"
          onClick={() => navigate("/collections?new=1")}
        >
          {strings.collections.newCollection}
        </Button>
      </ScreenHeader>

      <section className="oik-card screen-hero" aria-label={s.center}>
        <div className="screen-hero__text">
          <span className="oik-label">{s.center}</span>
          <div className="screen-hero__title">
            <Status kind="ok">{s.running}</Status>
            <span className="screen-hero__id" translate="no">
              {[info.host, `${shortId(info.did).slice(0, 4)}…${shortId(info.did).slice(-4)}`].filter(Boolean).join(" · ")}
            </span>
          </div>
          <span className="screen-hero__meta">
            {[
              s.centerMeta(mine.length, items, places.length),
              info.recoveryKeySavedAtMs ? s.backedUpOn(dateShort(info.recoveryKeySavedAtMs)) : s.notBackedUp,
            ].join(" · ")}
          </span>
        </div>
        <ol className="screen-pipe">
          {sourceList.length > 0 ? (
            <Stage
              done={failing === 0}
              bad={failing > 0}
              label={strings.nav.sources}
              value={`${sourceList.length - failing} / ${sourceList.length}`}
            />
          ) : (
            <Stage done label={s.statCollections} value={String(mine.length)} />
          )}
          <Stage done label={s.stored} value={s.placesCount(places.length)} />
          <Stage done={people > 0} half={people === 0} label={s.shared} value={s.peopleCount(people)} />
          <Stage
            done={info.recoveryKeySaved}
            bad={!info.recoveryKeySaved}
            label={s.backedUp}
            value={info.recoveryKeySavedAtMs ? dateShort(info.recoveryKeySavedAtMs) : s.notYet}
          />
        </ol>
      </section>

      {!info.recoveryKeySaved && (
        <section className="oik-card screen-callout" aria-labelledby="backup">
          <Tile icon="key" tone="apricot" />
          <div className="screen-callout__text">
            <h2 id="backup" className="screen-callout__title">
              {strings.backup.bannerTitle}
            </h2>
            <p>{strings.backup.bannerBody}</p>
          </div>
          <Button variant="primary" onClick={() => navigate("/settings?backup=1")}>
            {strings.backup.backUpNow}
          </Button>
        </section>
      )}

      <div className="screen-kpis">
        <Figure
          to="/collections"
          icon="collection"
          tone="rose"
          label={s.statItems}
          count={items}
          note={s.inCollections(mine.length)}
        />
        <Figure
          to="/activity"
          icon="person-plus"
          tone="sage"
          label={s.statReads}
          count={reads}
          note={shares.data ? s.activeShares(activeShares) : undefined}
        />
        <Figure
          to="/settings"
          icon={preferred ? PLACE_ICONS[preferred.type] : "computer"}
          tone="lavender"
          label={s.statStored(preferred ? placeLabel(preferred) : strings.storage.thisComputer)}
          value={
            storages.data
              ? preferred?.freeBytes != null && preferred.totalBytes != null
                ? bytes(preferred.totalBytes - preferred.freeBytes)
                : strings.settings.storageCount(preferred?.spaces ?? 0)
              : undefined
          }
          note={preferred?.freeBytes != null ? s.freeShort(bytes(preferred.freeBytes)) : undefined}
        />
      </div>

      <CardGrid
        cards={shown}
        editing={editing}
        labelOf={(id) => choiceOf(id)?.label ?? id}
        render={renderCard}
        onChange={(cards) => void save(cards)}
        onAdd={() => setAdding(true)}
      />
      {adding && (
        <AddCardDialog
          choices={available}
          shown={shown.map((card) => card.id)}
          onClose={() => setAdding(false)}
          onAdd={(choice) => {
            void save([...shown, { id: choice.id, width: choice.width }]);
            toast(s.addedCard(choice.label));
          }}
        />
      )}
    </>
  );
}

function placeLabel(place: StorageSummary): string {
  return place.name === null ? strings.storage.thisComputer : place.label;
}

function Stage({
  label,
  value,
  done,
  half,
  bad,
}: {
  label: string;
  value: string;
  done?: boolean;
  half?: boolean;
  bad?: boolean;
}) {
  return (
    <li className="screen-pipe__stage">
      <i className={bad ? "is-bad" : half ? "is-half" : done ? "is-done" : ""} aria-hidden="true" />
      <span className="screen-pipe__label">{label}</span>
      <span className="screen-pipe__value">{value}</span>
    </li>
  );
}

/** A figure at the top of Home; it opens the page that says more. A count counts up as it arrives. */
function Figure({
  to,
  icon,
  tone,
  label,
  value,
  count,
  note,
}: {
  to: string;
  icon: IconName;
  tone: TileTone;
  label: string;
  value?: string;
  count?: number;
  note?: string;
}) {
  const counted = useCountUp(count);
  const shown = counted !== undefined ? counted.toLocaleString("en") : value;
  return (
    <Link to={to} className="oik-card screen-figure screen-lift">
      <span className="screen-figure__top">
        <Tile icon={icon} tone={tone} />
        {note && <span className="screen-figure__note">{note}</span>}
      </span>
      <span className="oik-label">{label}</span>
      {shown === undefined ? (
        <span className="screen-figure__value oik-skel" aria-hidden="true" />
      ) : (
        <span className="screen-figure__value">{shown}</span>
      )}
    </Link>
  );
}

/** A card with a head: a label, a link on the right. */
function Panel({
  title,
  link,
  className,
  children,
}: {
  title: string;
  link?: { to: string; label: string };
  className?: string;
  children: ReactNode;
}) {
  return (
    <section className={`oik-card screen-panel ${className ?? ""}`} aria-label={title}>
      <header className="screen-panel__head">
        <h2 className="oik-label">{title}</h2>
        {link && (
          <Link to={link.to} className="oik-link">
            {link.label}
          </Link>
        )}
      </header>
      {children}
    </section>
  );
}

function RecentRow({ recent, tone }: { recent: Recent; tone: TileTone }) {
  const { space, view, item } = recent;
  const meta = [space.name, itemDate(item)].filter(Boolean).join(" · ");
  return (
    <li>
      <Link to={`/collections/${encodeURIComponent(space.name)}`} className="screen-list__row">
        {item.image ? (
          <img className="screen-list__thumb" src={item.image} alt="" decoding="async" />
        ) : (
          <Tile icon={view === "notes" ? "note" : view === "series" ? "activity" : "table"} tone={tone} />
        )}
        <span className="screen-list__text">
          <span className="screen-list__title">{item.text ?? space.name}</span>
          <span className="screen-list__meta">{meta}</span>
        </span>
        {view === "table" && item.detail && <span className="screen-list__value">{item.detail}</span>}
      </Link>
    </li>
  );
}

function LogRow({ item }: { item: AccessItem }) {
  return (
    <li className="screen-list__row">
      <Avatar name={item.contactName ?? "?"} small />
      <span className="screen-list__text">
        <span className="screen-list__title">{sentence(item)}</span>
        <span className="screen-list__meta">{ago(item.atMs)}</span>
      </span>
      <Status kind={item.outcome === "allowed" ? "ok" : "bad"}>
        {item.outcome === "allowed" ? strings.activity.allowed : strings.activity.refused}
      </Status>
    </li>
  );
}

function PlaceRow({ place }: { place: StorageSummary }) {
  const used =
    place.totalBytes && place.freeBytes != null
      ? (place.totalBytes - place.freeBytes) / place.totalBytes
      : undefined;
  const meta = [strings.settings.storageCount(place.spaces), place.encrypted ? strings.settings.encrypted : null]
    .filter(Boolean)
    .join(" · ");
  return (
    <li className="screen-list__row">
      <Tile icon={PLACE_ICONS[place.type]} tone="stone" />
      <span className="screen-list__text">
        <span className="screen-list__title">
          {placeLabel(place)}
          {place.isDefault && <span className="oik-tag screen-tag-inline">{strings.storage.isDefault}</span>}
        </span>
        <span className="screen-list__meta">{meta}</span>
      </span>
      <span className="screen-room-mini">
        <span className="screen-room-mini__figure">
          {place.freeBytes != null ? s.freeShort(bytes(place.freeBytes)) : "—"}
        </span>
        {used !== undefined && (
          <span className="screen-bar">
            <i style={{ width: `${Math.max(2, used * 100)}%` }} />
          </span>
        )}
      </span>
    </li>
  );
}
