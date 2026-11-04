// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The Oikome app: the shell around the screens, and which screen a path
// shows. Every platform (the web now, desktop and phone later) renders this
// with its own NodeClient.

import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { Button, Card, EmptyState, ErrorText, Icon, Status, ToastProvider } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { NodeClient, NodeInfo, StorageSummary } from "@oikome/client";
import { ClientProvider, useClient, useLoad } from "./data";
import { bytes } from "./format";
import { Palette } from "./palette";
import { BackLink } from "./parts";
import { Link, navigate, useLocation } from "./router";
import { Activity } from "./screens/Activity";
import { Sources } from "./screens/Sources";
import { Collection } from "./screens/Collection";
import { Collections } from "./screens/Collections";
import { Home } from "./screens/Home";
import { Onboarding } from "./screens/Onboarding";
import { Settings } from "./screens/Settings";
import { Sharing } from "./screens/Sharing";
import { strings } from "./strings";

export function App({ client }: { client: NodeClient }) {
  return (
    <ClientProvider client={client}>
      <ToastProvider>
        <Root />
      </ToastProvider>
    </ClientProvider>
  );
}

interface NavItem {
  to: string;
  label: string;
  icon: IconName;
}

const NAV: NavItem[] = [
  { to: "/", label: strings.nav.home, icon: "home" },
  { to: "/collections", label: strings.nav.collections, icon: "collection" },
  { to: "/sources", label: strings.nav.sources, icon: "download" },
  { to: "/sharing", label: strings.nav.sharing, icon: "person-plus" },
  { to: "/activity", label: strings.nav.activity, icon: "activity" },
];
/** At the foot of the sidebar, as settings usually are. */
const FOOT: NavItem[] = [{ to: "/settings", label: strings.nav.settings, icon: "sliders" }];

/** The section a path belongs to: Home only for "/" itself. */
function sectionOf(path: string): NavItem | undefined {
  return [...NAV, ...FOOT].find((item) =>
    item.to === "/" ? path === "/" : path === item.to || path.startsWith(`${item.to}/`),
  );
}

function Root() {
  const client = useClient();
  const { path, query } = useLocation();
  const node = useLoad(async () => {
    const [info, spaces] = await Promise.all([client.nodeInfo(), client.listSpaces()]);
    return { info, firstRun: !info.name && !info.recoveryKeySaved && spaces.length === 0 };
  }, []);
  const [info, setInfo] = useState<NodeInfo>();
  const [onboarding, setOnboarding] = useState<boolean>();

  useEffect(() => {
    if (!node.data) return;
    setInfo(node.data.info);
    setOnboarding((current) => current ?? node.data?.firstRun);
  }, [node.data]);

  if (node.error) {
    return (
      <main className="screen-onboarding">
        <Card className="screen-onboarding__card">
          <ErrorText>{node.error}</ErrorText>
          <Button onClick={node.reload}>{strings.common.retry}</Button>
        </Card>
      </main>
    );
  }
  // The first answer takes a moment on a local node: show the bare ground
  // rather than flashing a "Loading" line.
  if (!info || onboarding === undefined) return null;
  if (onboarding || path === "/onboarding") {
    return (
      <Onboarding
        info={info}
        onInfo={setInfo}
        onFinished={() => {
          setOnboarding(false);
          navigate("/", { replace: true });
        }}
      />
    );
  }

  const section = sectionOf(path);
  let screen: ReactNode;
  if (path === "/") {
    screen = <Home info={info} onInfo={setInfo} />;
  } else if (path === "/collections") {
    screen = <Collections startCreating={query.get("new") === "1"} />;
  } else if (path.startsWith("/collections/")) {
    let name = path.slice("/collections/".length);
    try {
      name = decodeURIComponent(name);
    } catch {
      // A malformed address names no collection; the page says so.
    }
    screen = <Collection key={name} name={name} />;
  } else if (path === "/sharing") {
    screen = (
      <Sharing info={info} startAdding={query.get("add") === "1"} startSharing={query.get("share") === "1"} />
    );
  } else if (path === "/sources") {
    screen = <Sources startAdding={query.get("add") === "1"} />;
  } else if (path === "/activity") {
    screen = <Activity />;
  } else if (path === "/settings") {
    screen = <Settings info={info} onInfo={setInfo} backup={query.get("backup") === "1"} />;
  } else {
    screen = (
      <Card>
        <EmptyState
          title={strings.notFound.title}
          action={<Link to="/">{strings.nav.home}</Link>}
        />
      </Card>
    );
  }

  const sub = path.startsWith("/collections/") ? decodeSafe(path.slice("/collections/".length)) : undefined;
  return (
    <Shell info={info} current={section?.to} path={path} sub={sub}>
      {screen}
    </Shell>
  );
}

function decodeSafe(text: string): string {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

const PLACE_KEY = "oikome.place";

/** The place the sidebar's card shows, as this browser last chose it. */
function rememberedPlace(): string | null {
  try {
    return localStorage.getItem(PLACE_KEY);
  } catch {
    return null;
  }
}

function placeLabel(place: StorageSummary): string {
  return place.name === null ? strings.storage.thisComputer : place.label;
}

/** How full a place is, 0 to 1, where it says. */
function usedShare(place: StorageSummary): number | undefined {
  return place.totalBytes && place.freeBytes !== null && place.totalBytes > 0
    ? (place.totalBytes - place.freeBytes) / place.totalBytes
    : undefined;
}

/** Where collections are kept, under the pages; picking one shows its room below. */
function PlaceList({
  places,
  current,
  onPick,
}: {
  places: StorageSummary[];
  current?: StorageSummary;
  onPick: (place: StorageSummary) => void;
}) {
  if (places.length === 0) return null;
  return (
    <div className="screen-places">
      <span className="oik-label screen-places__title">{strings.nav.places}</span>
      <ul className="screen-places__list">
        {places.map((place) => (
          <li key={place.name ?? ""}>
            <button
              type="button"
              className="screen-place"
              aria-pressed={place === current}
              onClick={() => onPick(place)}
            >
              <Status kind={place.spaces > 0 ? "ok" : "idle"}>{placeLabel(place)}</Status>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** The room on the place picked: used of total, and a bar. */
function RoomCard({ place }: { place?: StorageSummary }) {
  if (!place) return null;
  const used = usedShare(place);
  return (
    <div className="screen-room" aria-live="polite">
      <div className="screen-room__head">
        <span className="oik-label">{placeLabel(place)}</span>
        <span className="screen-room__figure">
          {place.totalBytes != null && place.freeBytes != null
            ? strings.storage.usedOf(bytes(place.totalBytes - place.freeBytes), bytes(place.totalBytes))
            : strings.settings.storageCount(place.spaces)}
        </span>
      </div>
      {used !== undefined ? (
        <div className="screen-bar" role="img" aria-label={strings.storage.usedShare(Math.round(used * 100))}>
          <i key={place.name ?? ""} style={{ width: `${Math.max(2, used * 100)}%` }} />
        </div>
      ) : (
        <span className="screen-room__note">
          {place.encrypted ? strings.settings.encrypted : strings.storage.noRoom}
        </span>
      )}
    </div>
  );
}

function NavList({
  items,
  current,
  counts = {},
}: {
  items: NavItem[];
  current?: string;
  counts?: Record<string, number | undefined>;
}) {
  return (
    <ul className="screen-nav">
      {items.map((item) => (
        <li key={item.to}>
          <Link to={item.to} className="screen-nav__item" current={item.to === current}>
            <Icon name={item.icon} />
            {item.label}
            {counts[item.to] !== undefined && <span className="screen-nav__count">{counts[item.to]}</span>}
          </Link>
        </li>
      ))}
    </ul>
  );
}

function Shell({
  info,
  current,
  path,
  sub,
  children,
}: {
  info: NodeInfo;
  current?: string;
  path: string;
  /** A collection's name, when its page is open. */
  sub?: string;
  children: ReactNode;
}) {
  const client = useClient();
  const name = info.name || strings.defaultName;
  const host = info.host || name;
  const sectionLabel = [...NAV, ...FOOT].find((item) => item.to === current)?.label;
  const spaces = useLoad(() => client.listSpaces(), [path]);
  const sources = useLoad(() => client.listSources(), [path]);
  const storages = useLoad(() => client.listStorages(), [path]);
  const [picked, setPicked] = useState<string | null>(rememberedPlace);
  const [searching, setSearching] = useState(false);
  const places = storages.data ?? [];
  const place = places.find((x) => (x.name ?? "") === picked) ?? places.find((x) => x.isDefault) ?? places[0];
  const pick = (next: StorageSummary) => {
    const key = next.name ?? "";
    setPicked(key);
    try {
      localStorage.setItem(PLACE_KEY, key);
    } catch {
      // Not remembered in a private window: the card still switches.
    }
  };
  // ⌘K (Ctrl+K) opens the palette from anywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setSearching(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  useEffect(() => {
    const label = [...NAV, ...FOOT].find((item) => item.to === current)?.label;
    document.title = label ? `${label} · ${strings.brand}` : strings.brand;
  }, [current]);

  return (
    <div className="screen-shell">
      <a className="screen-skip" href="#main">
        {strings.common.skip}
      </a>
      <aside className="screen-sidebar">
        <div className="screen-sidebar__inner">
          <div className="screen-brand">
            <span className="screen-brand__mark" aria-hidden="true">
              <Icon name="mark" />
            </span>
            <span className="screen-brand__text">
              <span className="screen-brand__name">{strings.brand}</span>
              <span className="screen-brand__sub" translate="no">
                {host}
              </span>
            </span>
          </div>
          <button type="button" className="screen-search" onClick={() => setSearching(true)}>
            <Icon name="search" />
            <span>{strings.palette.open_}</span>
            <kbd>⌘K</kbd>
          </button>
          <nav className="screen-sidebar__nav" aria-label={strings.nav.label}>
            <NavList
              items={NAV}
              current={current}
              counts={{ "/collections": spaces.data?.length, "/sources": sources.data?.length || undefined }}
            />
          </nav>
          <PlaceList places={places} current={place} onPick={pick} />
          <div className="screen-sidebar__foot">
            <nav aria-label={strings.nav.more}>
              <NavList items={FOOT} current={current} />
            </nav>
            <RoomCard place={place} />
            {/* The data center this page is served by: it runs here, on this computer. */}
            <div className="screen-node">
              <span className="screen-node__mark" aria-hidden="true">
                {name.trim().charAt(0).toUpperCase()}
              </span>
              <span className="screen-node__text">
                <span className="screen-node__name">{name}</span>
                <Status kind="ok">{strings.nav.runningShort}</Status>
              </span>
            </div>
          </div>
        </div>
      </aside>
      <main id="main" tabIndex={-1} className="screen-main">
        <div className="screen-top">
          <nav className="screen-crumb" aria-label={strings.nav.crumb}>
            <span className="screen-crumb__root" translate="no">
              {host}
            </span>
            {sectionLabel && current !== "/" && (
              <>
                <span className="screen-crumb__sep" aria-hidden="true">/</span>
                {sub ? (
                  <BackLink bare parent={current ?? "/"} parentLabel={sectionLabel} />
                ) : (
                  <span className="screen-crumb__here" aria-current="page">{sectionLabel}</span>
                )}
              </>
            )}
            {current === "/" && (
              <>
                <span className="screen-crumb__sep" aria-hidden="true">/</span>
                <span className="screen-crumb__here" aria-current="page">{strings.nav.home}</span>
              </>
            )}
            {sub && (
              <>
                <span className="screen-crumb__sep" aria-hidden="true">/</span>
                <span className="screen-crumb__here screen-crumb__here--mono" aria-current="page" translate="no">
                  {sub}
                </span>
              </>
            )}
          </nav>
          <button
            type="button"
            className="oik-icon-button screen-top__search"
            aria-label={strings.palette.title}
            title={strings.palette.title}
            onClick={() => setSearching(true)}
          >
            <Icon name="search" />
          </button>
        </div>
        {/* Keyed by the page, so each page comes in as the prototype's do. */}
        <div key={path} className="screen-page">
          {children}
        </div>
      </main>
      {searching && <Palette onClose={() => setSearching(false)} />}
    </div>
  );
}
