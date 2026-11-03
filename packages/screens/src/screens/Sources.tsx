// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Sources: the folders and files a collection's data comes from, like a
// notes app's vault. They stay the owner's, editable with any app; the node
// reads them again every minute, and Sync Now reads one at once.

import { useEffect, useState } from "react";
import {
  Button,
  ChoiceList,
  Dialog,
  ErrorText,
  Icon,
  IconButton,
  Menu,
  Status,
  useToast,
} from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { SourceKind, SourceSummary, SpaceSummary } from "@oikome/client";
import { countOf } from "../covers";
import { loadPreview, forgetPreviews } from "../content";
import { useAction, useClient, useLoad } from "../data";
import { homeRelative } from "../format";
import { ScreenHeader } from "../parts";
import { Link, navigate } from "../router";
import { FolderBrowser } from "../storage";
import { strings } from "../strings";

const s = strings.sources;

export const SOURCE_ICONS: Record<SourceKind, IconName> = {
  photos: "photo",
  notes: "note",
  csv: "table",
  "flomo-export": "note",
  "flomo-mcp": "note",
};

/** "4 min ago", from milliseconds. */
export function since(ms: number, now = Date.now()): string {
  const minutes = Math.round((now - ms) / 60_000);
  if (minutes < 1) return s.justNow;
  if (minutes < 60) return s.minAgo(minutes);
  const hours = Math.round(minutes / 60);
  if (hours < 24) return s.hoursAgo(hours);
  return s.daysAgo(Math.round(hours / 24));
}

/** The file or folder a source reads, by its last name. */
export function placeName(place: string): string {
  return place.split(/[\\/]/).filter(Boolean).pop() ?? place;
}

/** How a source is doing, in one word and a glyph. */
export function statusOf(source: SourceSummary): { kind: "ok" | "bad" | "idle"; label: string } {
  if (source.failedAtMs != null) return { kind: "bad", label: s.failed };
  if (source.pulledAtMs == null) return { kind: "idle", label: s.waiting };
  return { kind: "ok", label: s.upToDate };
}

/** What the source's collection holds, from its cover's first read. */
function Holds({ space }: { space?: SpaceSummary }) {
  const client = useClient();
  const preview = useLoad(async () => (space ? loadPreview(client, space) : undefined), [space?.name]);
  return <>{countOf(preview.data) ?? "—"}</>;
}

function Row({
  source,
  space,
  onChange,
}: {
  source: SourceSummary;
  space?: SpaceSummary;
  onChange: () => void;
}) {
  const client = useClient();
  const toast = useToast();
  const sync = useAction();
  const status = statusOf(source);
  const failed = status.kind === "bad";
  const next =
    source.everySecs && source.pulledAtMs
      ? Math.max(0, Math.round((source.pulledAtMs + source.everySecs * 1000 - Date.now()) / 60_000))
      : undefined;
  return (
    <tr className={failed ? "screen-source is-failed" : "screen-source"}>
      <td>
        <span className="screen-source__what">
          <span className="oik-tile">
            <Icon name={SOURCE_ICONS[source.kind]} />
          </span>
          <span className="screen-source__text">
            <span className="screen-source__name" translate="no" title={source.place}>
              {placeName(source.place)}
            </span>
            <span className="screen-source__meta">
              {s.kinds[source.kind]} → <Link to={`/collections/${encodeURIComponent(source.name)}`}>{source.name}</Link>
            </span>
            {failed && source.failure && <span className="screen-source__failure">{source.failure}</span>}
          </span>
        </span>
      </td>
      <td>
        <Status kind={status.kind}>{status.label}</Status>
      </td>
      <td className="screen-cell-mono screen-source__when">
        <span>
          {failed && source.failedAtMs != null
            ? s.failedAgo(since(source.failedAtMs))
            : source.pulledAtMs != null
              ? since(source.pulledAtMs)
              : "—"}
        </span>
        <span className="screen-source__next">
          {source.everySecs ? (next === undefined || next === 0 ? s.nextSoon : s.nextIn(next)) : s.whenAsked}
        </span>
      </td>
      <td className="screen-cell-mono">
        <Holds space={space} />
      </td>
      <td className="screen-cell-action">
        <IconButton
          icon="sync"
          label={s.syncNowFor(placeName(source.place))}
          disabled={sync.busy}
          onClick={async () => {
            const done = await sync.run(() => client.pullSource({ source: source.name }));
            forgetPreviews(source.name);
            if (done) toast(s.synced(placeName(source.place)));
            onChange();
          }}
        />
        <Menu
          label={strings.common.moreActions(placeName(source.place))}
          items={[
            {
              label: s.showFiles,
              onSelect: async () => {
                await sync.run(() => client.showSourceFiles({ source: source.name }));
              },
            },
            { label: s.openCollection, onSelect: () => navigate(`/collections/${encodeURIComponent(source.name)}`) },
            {
              label: s.stop,
              destructive: true,
              onSelect: async () => {
                const done = await sync.run(() => client.removeSource({ source: source.name }));
                if (done) {
                  toast(s.stopped(placeName(source.place)));
                  onChange();
                }
              },
            },
          ]}
        />
        {sync.error && <span className="oik-visually-hidden" role="alert">{sync.error}</span>}
      </td>
    </tr>
  );
}

const KINDS: { kind: SourceKind; icon: IconName }[] = [
  { kind: "photos", icon: "photo" },
  { kind: "notes", icon: "note" },
  { kind: "csv", icon: "table" },
];

/** Bring in a folder of photos or notes, or a CSV file. */
export function AddSource({
  start,
  onClose,
  onAdded,
}: {
  start: SourceKind;
  onClose: () => void;
  onAdded: (source: SourceSummary) => void;
}) {
  const client = useClient();
  const [kind, setKind] = useState<SourceKind>(start);
  const [path, setPath] = useState<string>();
  const [browsing, setBrowsing] = useState(false);
  const add = useAction();
  const csv = kind === "csv";
  return (
    <>
      <Dialog
        title={s.addTitle}
        closeLabel={strings.common.close}
        onClose={onClose}
        footer={
          <>
            <Button onClick={onClose}>{strings.common.cancel}</Button>
            <Button
              variant="primary"
              disabled={!path || add.busy}
              onClick={async () => {
                if (!path) return;
                const added = await add.run(() => client.addSource({ kind, path }));
                if (added) onAdded(added);
              }}
            >
              {add.busy ? s.reading : s.add}
            </Button>
          </>
        }
      >
        <ChoiceList
          label={s.addWhat}
          name="source-kind"
          value={kind}
          onChange={(value) => {
            setKind(value as SourceKind);
            setPath(undefined);
            add.clear();
          }}
          choices={KINDS.map(({ kind: k, icon }) => ({
            value: k,
            title: s.kinds[k],
            meta: s.kindHelp[k],
            leading: <Icon name={icon} />,
          }))}
        />
        <div className="screen-source-pick">
          <span className="screen-path" translate="no" title={path}>
            {path ? homeRelative(path) : csv ? s.noFile : s.noFolder}
          </span>
          <Button onClick={() => setBrowsing(true)}>{csv ? s.chooseFile : s.chooseFolder}</Button>
        </div>
        <p className="screen-help">{s.addHelp}</p>
        {add.error && <ErrorText>{add.error}</ErrorText>}
      </Dialog>
      {browsing && (
        <FolderBrowser
          title={csv ? s.chooseFile : s.chooseFolder}
          csv={csv}
          hint={(here) => (csv ? s.pickCsv : kind === "photos" ? s.holdsPhotos(here.photos) : s.holdsNotes(here.notes))}
          onClose={() => setBrowsing(false)}
          onChoose={(picked) => {
            setPath(picked);
            setBrowsing(false);
            add.clear();
          }}
        />
      )}
    </>
  );
}

export function Sources({ startAdding = false }: { startAdding?: boolean }) {
  const client = useClient();
  const toast = useToast();
  const sources = useLoad(() => client.listSources(), []);
  const spaces = useLoad(() => client.listSpaces(), []);
  const [adding, setAdding] = useState<SourceKind | undefined>(startAdding ? "photos" : undefined);
  useEffect(() => {
    if (startAdding) navigate("/sources", { replace: true });
  }, [startAdding]);
  // Read again while the page is open: the node reads sources every minute.
  useEffect(() => {
    const timer = window.setInterval(() => sources.reload(), 30_000);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // In the order they read: by the file or folder's name.
  const list = [...(sources.data ?? [])].sort((a, b) =>
    placeName(a.place).localeCompare(placeName(b.place), "en", { sensitivity: "base" }),
  );
  const failing = list.filter((x) => x.failedAtMs != null).length;
  const reload = () => {
    sources.reload();
    spaces.reload();
  };

  return (
    <>
      <ScreenHeader title={s.title} lede={sources.data ? s.lede(list.length, failing) : s.ledeShort}>
        <Button variant="primary" icon="plus" onClick={() => setAdding("photos")}>
          {s.addTitle}
        </Button>
      </ScreenHeader>
      {sources.error && <ErrorText>{sources.error}</ErrorText>}
      {sources.data && list.length > 0 && (
        <div className="oik-table-wrap screen-table-in-card">
          <table className="oik-table screen-sources" aria-label={s.title}>
            <thead>
              <tr>
                <th scope="col">{s.colSource}</th>
                <th scope="col">{s.colStatus}</th>
                <th scope="col">{s.colWhen}</th>
                <th scope="col">{s.colHolds}</th>
                <th scope="col">
                  <span className="oik-visually-hidden">{s.colActions}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {list.map((source) => (
                <Row
                  key={source.name}
                  source={source}
                  space={spaces.data?.find((x) => x.name === source.name)}
                  onChange={reload}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
      {sources.data && list.length === 0 && <p className="screen-panel__empty">{s.none}</p>}

      <h2 className="oik-label screen-section-label">{s.more}</h2>
      <div className="screen-source-kinds">
        {KINDS.map(({ kind, icon }) => (
          <button key={kind} type="button" className="screen-source-kind" onClick={() => setAdding(kind)}>
            <span className="oik-tile">
              <Icon name={icon} />
            </span>
            <span className="screen-source__text">
              <span className="screen-source__name">{s.kinds[kind]}</span>
              <span className="screen-source__meta">{s.kindHelp[kind]}</span>
            </span>
          </button>
        ))}
      </div>

      {adding && (
        <AddSource
          start={adding}
          onClose={() => setAdding(undefined)}
          onAdded={(added) => {
            setAdding(undefined);
            toast(s.added(added.name));
            reload();
          }}
        />
      )}
    </>
  );
}
