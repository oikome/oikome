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
import { BackLink } from "./parts";
import { Link, navigate, useLocation } from "./router";
import { Activity } from "./screens/Activity";
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
    screen = <Home info={info} />;
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
    screen = <Sharing info={info} startAdding={query.get("add") === "1"} />;
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

/**
 * Where collections are kept, at the foot of the sidebar: every place in a
 * list, and the room on the one picked. Any place can be picked, so every
 * place's room can be seen.
 */
function Places({ path }: { path: string }) {
  const client = useClient();
  const storages = useLoad(() => client.listStorages(), [path]);
  const [picked, setPicked] = useState<string | null>(rememberedPlace);
  const list = storages.data ?? [];
  const current = list.find((x) => (x.name ?? "") === picked) ?? list.find((x) => x.isDefault) ?? list[0];
  if (!current) return null;
  const pick = (place: StorageSummary) => {
    const key = place.name ?? "";
    setPicked(key);
    try {
      localStorage.setItem(PLACE_KEY, key);
    } catch {
      // Not remembered in a private window: the card still switches.
    }
  };
  const used =
    current.totalBytes && current.freeBytes !== null && current.totalBytes > 0
      ? (current.totalBytes - current.freeBytes) / current.totalBytes
      : undefined;
  return (
    <>
      <div className="screen-places">
        <span className="oik-label screen-places__title">{strings.nav.places}</span>
        <ul className="screen-places__list">
          {list.map((place) => (
            <li key={place.name ?? ""}>
              <button
                type="button"
                className="screen-place"
                aria-pressed={place === current}
                onClick={() => pick(place)}
              >
                <Status kind={place.spaces > 0 ? "ok" : "idle"}>{placeLabel(place)}</Status>
              </button>
            </li>
          ))}
        </ul>
      </div>
      <div className="screen-room" aria-live="polite">
        <div className="screen-room__head">
          <span className="oik-label">{placeLabel(current)}</span>
          {current.totalBytes && current.freeBytes !== null ? (
            <span className="screen-room__figure">
              {strings.storage.freeOf(bytes(current.freeBytes), bytes(current.totalBytes))}
            </span>
          ) : (
            <span className="screen-room__figure">{strings.settings.storageCount(current.spaces)}</span>
          )}
        </div>
        {used !== undefined ? (
          <div className="screen-bar" role="img" aria-label={strings.storage.usedShare(Math.round(used * 100))}>
            <i key={current.name ?? ""} style={{ width: `${Math.max(2, used * 100)}%` }} />
          </div>
        ) : (
          <span className="screen-room__note">{current.encrypted ? strings.settings.encrypted : strings.storage.noRoom}</span>
        )}
      </div>
    </>
  );
}

function NavList({ items, current }: { items: NavItem[]; current?: string }) {
  return (
    <ul className="screen-nav">
      {items.map((item) => (
        <li key={item.to}>
          <Link to={item.to} className="screen-nav__item" current={item.to === current}>
            <Icon name={item.icon} />
            {item.label}
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
  const name = info.name || strings.defaultName;
  const sectionLabel = [...NAV, ...FOOT].find((item) => item.to === current)?.label;
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
              <span className="screen-brand__sub">{name}</span>
            </span>
          </div>
          <nav className="screen-sidebar__nav" aria-label={strings.nav.label}>
            <NavList items={NAV} current={current} />
          </nav>
          <Places path={path} />
          <div className="screen-sidebar__foot">
            <nav aria-label={strings.nav.more}>
              <NavList items={FOOT} current={current} />
            </nav>
            {/* The data center this page is served by: it runs here, on this computer. */}
            <div className="screen-node">
              <span className="screen-node__mark" aria-hidden="true">
                {name.trim().charAt(0).toUpperCase()}
              </span>
              <span className="screen-node__text">
                <span className="screen-node__name">{name}</span>
                <Status kind="ok">{strings.nav.running}</Status>
              </span>
            </div>
          </div>
        </div>
      </aside>
      <main id="main" tabIndex={-1} className="screen-main">
        <div className="screen-top">
          <nav className="screen-crumb" aria-label={strings.nav.crumb}>
            <span className="screen-crumb__root">{name}</span>
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
        </div>
        <div className="screen-page">{children}</div>
      </main>
    </div>
  );
}
