// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Sources: where a collection's data comes from. Every data source is
// offered by an installed plugin; the page has none of its own. File Import
// and RustFS put folders and files into a collection as its files; the
// others fill a table, which the node reads again on a schedule.

import { useEffect, useState } from "react";
import {
  Button,
  ChoiceList,
  ConfirmDialog,
  Dialog,
  ErrorText,
  Icon,
  IconButton,
  Menu,
  Select,
  Status,
  TextField,
  useToast,
} from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { PluginSourceCapability, SourceKind, SourceSummary, SpaceSummary } from "@oikome/client";
import { countOf } from "../covers";
import { inLanguage } from "../plugins";
import { loadPreview, forgetPreviews } from "../content";
import { useAction, useClient, useLoad } from "../data";
import { homeRelative } from "../format";
import { ScreenHeader } from "../parts";
import { Link, navigate } from "../router";
import { FolderBrowser } from "../storage";
import { AddFiles } from "../import";
import { strings } from "../strings";
import { locale } from "../i18n/languages";

const s = strings.sources;

export const SOURCE_ICONS: Record<SourceKind, IconName> = {
  files: "folder",
  rustfs: "bucket",
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
export function statusOf(source: SourceSummary): { kind: "ok" | "bad" | "idle" | "off"; label: string } {
  if (source.paused) return { kind: "off", label: s.paused };
  if (source.failedAtMs != null) return { kind: "bad", label: s.failed };
  if (source.pulledAtMs == null) return { kind: "idle", label: s.waiting };
  return { kind: "ok", label: s.upToDate };
}

/** When it was last read (or failed). */
export function lastText(source: SourceSummary): string {
  if (source.failedAtMs != null && !source.paused) return s.failedAgo(since(source.failedAtMs));
  return source.pulledAtMs != null ? since(source.pulledAtMs) : "—";
}

/** When it is read next. */
export function nextText(source: SourceSummary, now = Date.now()): string {
  if (source.paused) return s.pausedNext;
  if (!source.everySecs) return s.whenAsked;
  const next =
    source.pulledAtMs != null
      ? Math.max(0, Math.round((source.pulledAtMs + source.everySecs * 1000 - now) / 60_000))
      : 0;
  if (next === 0) return s.nextSoon;
  return next < 120 ? s.nextIn(next) : s.nextInHours(Math.round(next / 60));
}

/** What the source's collection holds, from its cover's first read. */
export function Holds({ space }: { space?: SpaceSummary }) {
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
  const remove = useAction();
  const [removing, setRemoving] = useState(false);
  const status = statusOf(source);
  const failed = status.kind === "bad";
  const details = `/sources/${encodeURIComponent(source.name)}`;
  return (
    <tr
      className={failed ? "screen-source is-failed" : "screen-source"}
      onClick={(e) => {
        // The row opens its details; its own links and buttons do their part.
        if (!(e.target as Element).closest("a, button, dialog")) navigate(details);
      }}
    >
      <td className="screen-source__cell">
        <span className="screen-source__what">
          <span className="oik-tile">
            <Icon name={SOURCE_ICONS[source.kind]} />
          </span>
          <span className="screen-source__text">
            <Link to={details} className="screen-source__name">
              <span translate="no" title={source.place}>
                {placeName(source.place)}
              </span>
            </Link>
            <span className="screen-source__meta">
              {s.kinds[source.kind]} → <Link to={`/collections/${encodeURIComponent(source.space)}`}>{source.space}</Link>
            </span>
            {failed && source.failure && <span className="screen-source__failure">{source.failure}</span>}
          </span>
        </span>
      </td>
      <td>
        <Status kind={status.kind}>{status.label}</Status>
      </td>
      <td className="screen-cell-mono screen-source__when">
        <span>{lastText(source)}</span>
        <span className="screen-source__next">{nextText(source)}</span>
      </td>
      <td className="screen-cell-mono">
        <Holds space={space} />
      </td>
      <td className="screen-cell-action">
        <IconButton
          icon="sync"
          label={s.syncNowFor(placeName(source.place))}
          disabled={sync.busy}
          spinning={sync.busy}
          onClick={async () => {
            const done = await sync.run(() => client.pullSource({ source: source.name }));
            forgetPreviews(source.space);
            if (done) toast(s.synced(placeName(source.place)));
            onChange();
          }}
        />
        <Menu
          label={strings.common.moreActions(placeName(source.place))}
          items={[
            { label: s.openDetails, onSelect: () => navigate(details) },
            {
              label: s.showFiles,
              onSelect: async () => {
                await sync.run(() => client.showSourceFiles({ source: source.name }));
              },
            },
            { label: s.openCollection, onSelect: () => navigate(`/collections/${encodeURIComponent(source.space)}`) },
            {
              label: source.paused ? s.resume : s.pause,
              onSelect: async () => {
                const done = await sync.run(() =>
                  client.pauseSource({ source: source.name, paused: !source.paused }),
                );
                if (done) {
                  toast(done.paused ? s.pausedToast(placeName(source.place)) : s.resumed(placeName(source.place)));
                  onChange();
                }
              },
            },
            { label: s.remove, destructive: true, onSelect: () => setRemoving(true) },
          ]}
        />
        {sync.error && <span className="oik-visually-hidden" role="alert">{sync.error}</span>}
        {removing && (
          <ConfirmDialog
            title={s.removeTitle(placeName(source.place))}
            body={s.removeBody(source.space, placeName(source.place))}
            confirmLabel={s.removeYes}
            cancelLabel={strings.common.cancel}
            busy={remove.busy}
            error={remove.error}
            onConfirm={async () => {
              const done = await remove.run(() => client.removeSource({ source: source.name }));
              if (done) {
                setRemoving(false);
                toast(s.removed(placeName(source.place)));
                onChange();
              }
            }}
            onCancel={() => {
              setRemoving(false);
              remove.clear();
            }}
          />
        )}
      </td>
    </tr>
  );
}

/** A data source an installed plugin offers. */
type Offer = PluginSourceCapability & { pluginName: string; pluginId: string; description: string };

function tableName(path: string | undefined): string {
  const stem = path?.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? "";
  const name = stem.toLowerCase().replace(/[^a-z0-9_]+/g, "_").replace(/^_+|_+$/g, "");
  return /^[a-z_][a-z0-9_]*$/.test(name) ? name : "rows";
}

/** Which of the installed data sources to add. */
function ChooseSource({ offers, onClose, onChoose }: { offers: Offer[]; onClose: () => void; onChoose: (offer: Offer) => void }) {
  const [chosen, setChosen] = useState(offers[0] ? `${offers[0].pluginId}:${offers[0].id}` : "");
  const offer = offers.find((x) => `${x.pluginId}:${x.id}` === chosen);
  return (
    <Dialog
      title={s.addTitle}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button variant="primary" disabled={!offer} onClick={() => offer && onChoose(offer)}>
            {strings.onboarding.continue}
          </Button>
        </>
      }
    >
      <ChoiceList
        label={s.choose}
        name="source-offer"
        value={chosen}
        onChange={setChosen}
        choices={offers.map((x) => ({
          value: `${x.pluginId}:${x.id}`,
          title: x.name,
          meta: x.description,
          leading: <Icon name={SOURCE_ICONS[x.kind]} />,
        }))}
      />
    </Dialog>
  );
}

/** Bring in what a plugin's source reads into a table: a file it knows. */
export function AddSource({
  offer,
  onClose,
  onAdded,
}: {
  offer: Offer;
  onClose: () => void;
  onAdded: (source: SourceSummary) => void;
}) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const kind = offer.kind;
  const [path, setPath] = useState<string>();
  const [space, setSpace] = useState<string>();
  const [table, setTable] = useState("");
  const [browsing, setBrowsing] = useState(false);
  const add = useAction();
  const csv = kind === "csv";
  const file = csv || kind === "flomo-export";
  const eligible = (spaces.data ?? []).filter((space) =>
    file ? space.kind === "transactional" && !space.inPlace : space.inPlace || (space.kind === "analytical" && !space.inPlace),
  );
  const destination = eligible.some((item) => item.name === space) ? space : eligible[0]?.name;
  const destinationTable = table || tableName(path);
  return (
    <>
      <Dialog
        title={offer.name}
        closeLabel={strings.common.close}
        onClose={onClose}
        footer={
          <>
            <Button onClick={onClose}>{strings.common.cancel}</Button>
            <Button
              variant="primary"
              disabled={!path || !destination || add.busy}
              onClick={async () => {
                if (!path || !destination) return;
                const added = await add.run(() => client.addSource({ kind, path, space: destination, table: destinationTable }));
                if (added) {
                  forgetPreviews(added.space);
                  onAdded(added);
                }
              }}
            >
              {add.busy ? s.reading : s.add}
            </Button>
          </>
        }
      >
        <p className="oik-dialog__text">{offer.description}</p>
        <Select
          label={s.targetCollection}
          value={destination ?? ""}
          onChange={setSpace}
          options={eligible.map((space) => ({ value: space.name, label: space.name }))}
        />
        {csv && (
          <TextField
            label={s.targetTable}
            name="target-table"
            hint={s.targetHint}
            value={destinationTable}
            onChange={setTable}
            mono
          />
        )}
        {spaces.data && eligible.length === 0 && <p className="screen-help">{s.noTarget}</p>}
        <div className="screen-source-pick">
          <span className="screen-path" translate="no" title={path}>
            {path ? homeRelative(path) : file ? s.noFile : s.noFolder}
          </span>
          <Button onClick={() => setBrowsing(true)}>{file ? s.chooseFile : s.chooseFolder}</Button>
        </div>
        {add.busy ? (
          <span className="screen-progress" role="progressbar" aria-label={s.reading}>
            <i />
          </span>
        ) : (
          <p className="screen-help">{s.addHelp}</p>
        )}
        {add.error && <ErrorText>{add.error}</ErrorText>}
      </Dialog>
      {browsing && (
        <FolderBrowser
          title={file ? s.chooseFile : s.chooseFolder}
          csv={csv}
          extensions={kind === "flomo-export" ? ["html", "zip"] : undefined}
          hint={(here) =>
            csv ? s.pickCsv : file ? s.addHelp : s.holdsNotes(here.notes)
          }
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
  const plugins = useLoad(async () => (await client.listPlugins()).map(inLanguage), []);
  const [adding, setAdding] = useState<Offer | "choose" | undefined>(startAdding ? "choose" : undefined);
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
    placeName(a.place).localeCompare(placeName(b.place), locale(), { sensitivity: "base" }),
  );
  const paused = list.filter((x) => x.paused).length;
  const failing = list.filter((x) => !x.paused && x.failedAtMs != null).length;
  const offers: Offer[] = (plugins.data ?? [])
    .filter((plugin) => plugin.installed)
    .flatMap((plugin) =>
      plugin.capabilities.sources.map((source) => ({
        ...source,
        pluginName: plugin.name,
        pluginId: plugin.id,
        description: plugin.description,
      })),
    );
  const reload = () => {
    sources.reload();
    spaces.reload();
  };

  return (
    <>
      <ScreenHeader title={s.title} lede={sources.data ? s.lede(list.length, failing, paused) : s.ledeShort}>
        <Button variant="primary" icon="plus" disabled={offers.length === 0} onClick={() => setAdding("choose")}>
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
                  space={spaces.data?.find((x) => x.name === source.space)}
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
        {offers.map((offer) => (
          <button
            key={`${offer.pluginId}:${offer.id}`}
            type="button"
            className="screen-source-kind"
            onClick={() => setAdding(offer)}
          >
            <span className="oik-tile">
              <Icon name={SOURCE_ICONS[offer.kind]} />
            </span>
            <span className="screen-source__text">
              <span className="screen-source__name">{offer.name}</span>
              <span className="screen-source__meta">{offer.description}</span>
            </span>
          </button>
        ))}
      </div>

      {adding === "choose" && (
        <ChooseSource offers={offers} onClose={() => setAdding(undefined)} onChoose={setAdding} />
      )}
      {adding && adding !== "choose" && (adding.kind === "files" || adding.kind === "rustfs") && (
        <AddFiles
          origin={adding.kind === "files" ? "local" : "rustfs"}
          title={adding.name}
          onClose={() => setAdding(undefined)}
          onAdded={reload}
        />
      )}
      {adding && adding !== "choose" && adding.kind !== "files" && adding.kind !== "rustfs" && (
        <AddSource
          offer={adding}
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
