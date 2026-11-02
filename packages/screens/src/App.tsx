// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The Oikome app: the shell around the screens, and which screen a path
// shows. Every platform (the web now, desktop and phone later) renders this
// with its own NodeClient.

import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { Button, Card, EmptyState, ErrorText, Icon, ToastProvider } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { NodeClient, NodeInfo } from "@oikome/client";
import { ClientProvider, useClient, useLoad } from "./data";
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

  return (
    <Shell info={info} current={section?.to}>
      {screen}
    </Shell>
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
  children,
}: {
  info: NodeInfo;
  current?: string;
  children: ReactNode;
}) {
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
            <span className="screen-brand__name">{strings.brand}</span>
          </div>
          <nav className="screen-sidebar__nav" aria-label={strings.nav.label}>
            <NavList items={NAV} current={current} />
            <NavList items={FOOT} current={current} />
          </nav>
          {/* The data center this page is served by: it runs here, on this computer. */}
          <div className="screen-node">
            <span className="screen-node__mark" aria-hidden="true">
              {(info.name || strings.defaultName).trim().charAt(0).toUpperCase()}
            </span>
            <span className="screen-node__text">
              <span className="screen-node__name">{info.name || strings.defaultName}</span>
              <span className="screen-node__state">
                <span className="screen-live__dot" aria-hidden="true" />
                {strings.nav.running}
              </span>
            </span>
          </div>
        </div>
      </aside>
      <main id="main" tabIndex={-1} className="screen-main">
        {children}
      </main>
    </div>
  );
}
