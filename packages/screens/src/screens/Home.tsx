// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Home: what the data center holds at a glance, what came in last, who
// opened what you shared, and the one thing that needs doing.

import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { Avatar, Button, EmptyState, Icon, Tile, collectionTones } from "@oikome/ui";
import type { IconName, TileTone } from "@oikome/ui";
import type { AccessItem, NodeInfo, SpaceSummary } from "@oikome/client";
import { loadPreview } from "../content";
import type { Item, Preview } from "../content";
import { Cover, itemDate, sharedNames } from "../covers";
import { useClient, useLoad } from "../data";
import { ago } from "../format";
import { Link, navigate } from "../router";
import { strings } from "../strings";
import { sentence } from "./Activity";

const s = strings.home;
const WEEK_MS = 7 * 24 * 60 * 60 * 1000;
/** Items in Recently Added, and covers and log lines on Home. */
const RECENT = 5;
const COVERS = 3;
const LOG = 4;

const today = new Intl.DateTimeFormat("en", { weekday: "long", month: "long", day: "numeric" });

interface Recent {
  space: SpaceSummary;
  view: Preview["shape"]["view"];
  item: Item;
}

export function Home({ info }: { info: NodeInfo }) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const log = useLoad(() => client.listAccess({ limit: 100 }), []);
  const [previews, setPreviews] = useState<Map<string, Preview>>();

  const list = spaces.data ?? [];
  const mine = list.filter((space) => space.kind !== "remote");
  const toneOf = collectionTones(list.map((space) => space.name));
  const shared = sharedNames(shares.data);

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
  const people = new Set((shares.data ?? []).filter((x) => x.status === "active").map((x) => x.did)).size;
  const now = Date.now();
  const reads = log.data?.filter((x) => x.outcome === "allowed" && now - x.atMs < WEEK_MS).length;

  // The newest few things across collections, by their own dates.
  const recent: Recent[] = previews
    ? mine
        .flatMap((space) => {
          const preview = previews.get(space.name);
          return (preview?.items ?? []).map((item) => ({ space, view: preview!.shape.view, item }));
        })
        .sort((a, b) => (b.item.when ?? 0) - (a.item.when ?? 0))
        .slice(0, RECENT)
    : [];

  return (
    <>
      <header className="screen-header screen-header--home">
        <div>
          <p className="screen-today">{today.format(new Date())}</p>
          <h1 className="screen-title">{s.greeting(new Date().getHours())}</h1>
          <p className="screen-muted">{s.lede(info.name || strings.defaultName)}</p>
        </div>
        <div className="screen-header__actions">
          <span className="screen-live">
            <span className="screen-live__dot" aria-hidden="true" />
            {strings.nav.running}
          </span>
          <Button icon="plus" onClick={() => navigate("/collections?new=1")}>
            {strings.collections.newCollection}
          </Button>
        </div>
      </header>

      <div className="screen-stats">
        <Stat icon="collection" tone="honey" label={s.statCollections} value={spaces.data ? mine.length : undefined} />
        <Stat icon="table" tone="sage" label={s.statItems} value={items} />
        <Stat icon="person-plus" tone="periwinkle" label={s.statPeople} value={shares.data ? people : undefined} />
        <Stat icon="activity" tone="lavender" label={s.statReads} value={reads} />
      </div>

      <div className="screen-home">
        <div className="screen-home__main">
          <Panel title={s.recent} link={{ to: "/collections", label: s.seeAll }}>
            {!previews ? (
              <div className="screen-panel__skeleton oik-skel" aria-hidden="true" />
            ) : recent.length === 0 ? (
              <p className="screen-panel__empty">{s.nothingRecent}</p>
            ) : (
              <ul className="screen-recent">
                {recent.map((r, i) => (
                  <RecentRow key={i} recent={r} tone={toneOf(r.space.name)} />
                ))}
              </ul>
            )}
          </Panel>
        </div>

        <aside className="screen-home__side">
          {!info.recoveryKeySaved && (
            <section className="screen-night" aria-labelledby="backup">
              <Icon name="key" />
              <h2 id="backup" className="screen-night__title">
                {strings.backup.bannerTitle}
              </h2>
              <p className="screen-night__body">{strings.backup.bannerBody}</p>
              <button type="button" className="screen-night__button" onClick={() => navigate("/settings?backup=1")}>
                {strings.backup.backUpNow}
              </button>
            </section>
          )}
          <Panel title={s.activity} footer={<Link to="/activity">{s.allActivity}</Link>}>
            {!log.data ? (
              <div className="screen-panel__skeleton oik-skel" aria-hidden="true" />
            ) : log.data.length === 0 ? (
              <p className="screen-panel__empty">{s.noActivity}</p>
            ) : (
              <ul className="screen-log">
                {log.data.slice(0, LOG).map((item) => (
                  <LogRow key={item.id} item={item} />
                ))}
              </ul>
            )}
          </Panel>
        </aside>

        <div className="screen-home__wide">
          <Panel title={s.collections} link={{ to: "/collections", label: s.seeAll }}>
            {spaces.data && mine.length === 0 ? (
              <EmptyState
                icon="collection"
                title={strings.collections.emptyTitle}
                body={strings.collections.emptyBody}
                action={
                  <Button variant="primary" icon="plus" onClick={() => navigate("/collections?new=1")}>
                    {strings.collections.newCollection}
                  </Button>
                }
              />
            ) : (
              <div className="screen-covers screen-covers--compact">
                {mine.slice(0, COVERS).map((space) => (
                  <Cover
                    key={space.name}
                    space={space}
                    tone={toneOf(space.name)}
                    names={shared.get(space.name) ?? []}
                  />
                ))}
              </div>
            )}
          </Panel>
        </div>
      </div>
    </>
  );
}

function Stat({
  icon,
  tone,
  label,
  value,
}: {
  icon: IconName;
  tone: TileTone;
  label: string;
  value: number | undefined;
}) {
  return (
    <div className="screen-stat">
      <Tile icon={icon} tone={tone} />
      <span className="screen-stat__label">{label}</span>
      {value === undefined ? (
        <span className="screen-stat__value oik-skel" aria-hidden="true" />
      ) : (
        <span className="screen-stat__value">{value.toLocaleString("en")}</span>
      )}
    </div>
  );
}

/** A card with a title row: a link on the right, and an optional footer. */
function Panel({
  title,
  link,
  footer,
  children,
}: {
  title: string;
  link?: { to: string; label: string };
  footer?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="screen-panel" aria-label={title}>
      <header className="screen-panel__head">
        <h2 className="screen-panel__title">{title}</h2>
        {link && (
          <Link to={link.to} className="screen-panel__link">
            {link.label}
          </Link>
        )}
      </header>
      <div className="screen-panel__body">{children}</div>
      {footer && <footer className="screen-panel__foot">{footer}</footer>}
    </section>
  );
}

function RecentRow({ recent, tone }: { recent: Recent; tone: TileTone }) {
  const { space, view, item } = recent;
  const title = item.text ?? space.name;
  const meta = [space.name, itemDate(item)].filter(Boolean).join(" · ");
  return (
    <li>
      <Link to={`/collections/${encodeURIComponent(space.name)}`} className="screen-recent__row">
        {item.image ? (
          <img className="screen-recent__thumb" src={item.image} alt="" decoding="async" />
        ) : (
          <Tile icon={view === "notes" ? "note" : "table"} tone={tone} />
        )}
        <span className="screen-recent__text">
          <span className={`screen-recent__title${view === "notes" ? " screen-recent__title--note" : ""}`}>
            {title}
          </span>
          <span className="screen-recent__meta">{meta}</span>
        </span>
        {view === "table" && item.detail && <span className="screen-recent__value">{item.detail}</span>}
      </Link>
    </li>
  );
}

function LogRow({ item }: { item: AccessItem }) {
  return (
    <li className="screen-log__row">
      <Avatar name={item.contactName ?? "?"} small />
      <span className="screen-log__text">
        <span>{sentence(item)}</span>
        <span className="screen-log__meta">{ago(item.atMs)}</span>
      </span>
      <span
        className={`screen-badge screen-badge--${item.outcome === "allowed" ? "ok" : "bad"}`}
      >
        {item.outcome === "allowed" ? strings.activity.allowed : strings.activity.refused}
      </span>
    </li>
  );
}
