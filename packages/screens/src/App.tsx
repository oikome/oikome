// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The Oikome app: the shell around the screens, and which screen a path
// shows. Every platform (the web now, desktop and phone later) renders this
// with its own NodeClient.

import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { Banner, Button, Card, EmptyState, ErrorText, Icon } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { NodeClient, NodeInfo } from "@oikome/client";
import { ClientProvider, useClient, useLoad } from "./data";
import { BannerContext } from "./parts";
import { Link, navigate, useLocation } from "./router";
import { Activity } from "./screens/Activity";
import { Collections } from "./screens/Collections";
import { Onboarding } from "./screens/Onboarding";
import { Settings } from "./screens/Settings";
import { Sharing } from "./screens/Sharing";
import { Sql } from "./screens/Sql";
import { strings } from "./strings";

export function App({ client }: { client: NodeClient }) {
  return (
    <ClientProvider client={client}>
      <Root />
    </ClientProvider>
  );
}

const NAV: { to: string; label: string; icon: IconName }[] = [
  { to: "/collections", label: strings.nav.collections, icon: "collection" },
  { to: "/sharing", label: strings.nav.sharing, icon: "person-plus" },
  { to: "/activity", label: strings.nav.activity, icon: "activity" },
  { to: "/settings", label: strings.nav.settings, icon: "sliders" },
];

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

  useEffect(() => {
    if (path === "/" && onboarding === false) navigate("/collections", { replace: true });
  }, [path, onboarding]);

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
  if (!info || onboarding === undefined) {
    return <p className="screen-muted screen-onboarding">{strings.common.loading}</p>;
  }
  if (onboarding || path === "/onboarding") {
    return (
      <Onboarding
        info={info}
        onInfo={setInfo}
        onFinished={() => {
          setOnboarding(false);
          navigate("/collections", { replace: true });
        }}
      />
    );
  }

  const section = NAV.find((item) => path === item.to || path.startsWith(`${item.to}/`));
  let screen: ReactNode;
  if (path === "/collections" || path.startsWith("/collections/")) {
    const name = path.slice("/collections/".length);
    screen = <Collections selected={name ? decodeURIComponent(name) : undefined} />;
  } else if (path === "/sharing") {
    screen = <Sharing info={info} preselect={query.get("share") ?? undefined} />;
  } else if (path === "/activity") {
    screen = <Activity />;
  } else if (path === "/settings") {
    screen = <Settings info={info} onInfo={setInfo} backup={query.get("backup") === "1"} />;
  } else if (path === "/settings/advanced") {
    screen = <Sql />;
  } else if (path === "/") {
    screen = null;
  } else {
    screen = (
      <Card>
        <EmptyState
          title={strings.notFound.title}
          action={<Link to="/collections">{strings.nav.collections}</Link>}
        />
      </Card>
    );
  }

  return (
    <Shell info={info} current={section?.to} showBackup={!info.recoveryKeySaved && path !== "/settings"}>
      {screen}
    </Shell>
  );
}

function Shell({
  info,
  current,
  showBackup,
  children,
}: {
  info: NodeInfo;
  current?: string;
  showBackup: boolean;
  children: ReactNode;
}) {
  useEffect(() => {
    const label = NAV.find((item) => item.to === current)?.label;
    document.title = label ? `${label} · ${strings.brand}` : strings.brand;
  }, [current]);

  const banner = showBackup ? (
    <Banner
      title={strings.backup.bannerTitle}
      body={strings.backup.bannerBody}
      action={<Button onClick={() => navigate("/settings?backup=1")}>{strings.backup.backUpNow}</Button>}
    />
  ) : null;

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
          <span>
            <span className="screen-brand__name">{strings.brand}</span>
            <span className="screen-brand__subtitle">{info.name || strings.brandSubtitle}</span>
          </span>
        </div>
        <nav aria-label={strings.nav.label}>
          <ul className="screen-nav">
            {NAV.map((item) => (
              <li key={item.to}>
                <Link to={item.to} className="screen-nav__item" current={item.to === current}>
                  <Icon name={item.icon} />
                  {item.label}
                </Link>
              </li>
            ))}
          </ul>
        </nav>
        </div>
      </aside>
      <main id="main" tabIndex={-1} className="screen-main">
        <BannerContext.Provider value={banner}>{children}</BannerContext.Provider>
      </main>
    </div>
  );
}
