// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Plugins register cards, sources, preview declarations and MCP clients.
// Installation grants no data and places no card; each capability is
// configured separately by the owner.

import { useEffect, useState } from "react";
import {
  Button,
  Checkbox,
  ConfirmDialog,
  Dialog,
  ErrorText,
  Icon,
  Status,
  Tile,
  useToast,
} from "@oikome/ui";
import type { Plugin, SpaceSummary } from "@oikome/client";
import { loadPreview } from "../content";
import type { View } from "../content";
import { inLanguage, previewColumnTypes } from "../plugins";
import { asSentence, useAction, useClient, useLoad } from "../data";
import type { Describe } from "../data";
import { ScreenHeader } from "../parts";
import { FolderBrowser } from "../storage";
import { strings } from "../strings";
import { bytes, listNames } from "../format";

const s = strings.plugins;

/** Where a plugin to add comes from. */
type Source = { builtIn: string } | { path: string } | { registry: { id: string; version: string } };

/** What a collection holds, as a plugin would see it: its view and its kinds of column. */
interface Holds {
  view: View;
  columns: Set<string>;
}

/** The collections of one's own, each with what it holds. */
function useCollectionViews() {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const [views, setViews] = useState<Map<string, Holds>>();
  useEffect(() => {
    if (!spaces.data) return;
    let current = true;
    const mine = spaces.data.filter((space) => space.kind !== "remote");
    Promise.all(mine.map((space) => loadPreview(client, space).catch(() => undefined))).then((loaded) => {
      if (!current) return;
      const map = new Map<string, Holds>();
      loaded.forEach((preview, i) => {
        const name = mine[i]!.name;
        const columns = preview ? previewColumnTypes(preview) : [];
        map.set(name, { view: preview?.shape.view ?? "table", columns: new Set(columns) });
      });
      setViews(map);
    });
    return () => {
      current = false;
    };
  }, [client, spaces.data]);
  const mine: SpaceSummary[] = (spaces.data ?? []).filter((space) => space.kind !== "remote");
  return { mine, views };
}

/** Whether a collection is the kind a plugin's first card reads. */
function fits(plugin: Plugin, holds: Holds | undefined): boolean {
  if (!holds) return false;
  return (plugin.shape === "any" || plugin.shape === holds.view) && plugin.columns.every((c) => holds.columns.has(c));
}

export function Plugins() {
  const client = useClient();
  const plugins = useLoad(async () => (await client.listPlugins()).map(inLanguage), []);
  const registry = useLoad(
    async () => (await client.listRegistryPlugins()).map((entry) => ({ ...entry, plugin: inLanguage(entry.plugin) })),
    [],
  );
  const { mine, views } = useCollectionViews();
  const [adding, setAdding] = useState<{ plugin: Plugin; source: Source }>();
  const [browsing, setBrowsing] = useState(false);
  const [editing, setEditing] = useState<Plugin>();
  const inspect = useAction();

  const installed = (plugins.data ?? []).filter((p) => p.installed);
  const offered = (plugins.data ?? []).filter((p) => !p.installed && p.builtIn);
  // What is installed, by what it offers; a package offering several is in each.
  const groups = (
    [
      { key: "sources", title: s.groupSources, icon: "download", plugins: installed.filter((p) => p.capabilities.sources.length > 0) },
      { key: "cards", title: s.groupCards, icon: "home", plugins: installed.filter((p) => p.capabilities.cards.length > 0) },
      { key: "clients", title: s.groupClients, icon: "ai", plugins: installed.filter((p) => p.capabilities.mcpClients.length > 0) },
    ] as const
  ).filter((group) => group.plugins.length > 0);
  const registryOffered = (registry.data ?? []).filter(
    (entry) => !(plugins.data ?? []).some((plugin) => plugin.builtIn && plugin.id === entry.plugin.id),
  );

  return (
    <>
      <ScreenHeader title={s.title} lede={s.lede}>
        <Button icon="folder" onClick={() => setBrowsing(true)}>
          {s.addFromFolder}
        </Button>
      </ScreenHeader>
      {(plugins.error ?? inspect.error) && <ErrorText>{plugins.error ?? inspect.error}</ErrorText>}

      {plugins.data && installed.length === 0 && <p className="screen-panel__empty">{s.none}</p>}
      {groups.map((group) => (
        <section key={group.key} className="screen-section" aria-labelledby={`plugins-${group.key}`}>
          <h2 id={`plugins-${group.key}`} className="oik-label screen-section-label">
            {group.title}
          </h2>
          <div className="oik-table-wrap screen-table-in-card">
            <table className="oik-table screen-plugins" aria-labelledby={`plugins-${group.key}`}>
              <thead>
                <tr>
                  <th scope="col">{s.colPlugin}</th>
                  {group.key === "cards" ? (
                    <>
                      <th scope="col">{s.colReads}</th>
                      <th scope="col">{s.colInternet}</th>
                    </>
                  ) : (
                    <th scope="col">{s.colWhat}</th>
                  )}
                  <th scope="col">{s.colStatus}</th>
                  <th scope="col">
                    <span className="oik-visually-hidden">{s.colActions}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {group.plugins.map((plugin) => (
                  <tr key={plugin.id}>
                    <td>
                      <span className="screen-plugin-name">
                        <Tile icon={group.icon} tone="stone" />
                        <span className="screen-plugin-name__text">
                          <span className="screen-plugin-name__name">{plugin.name}</span>
                          <span className="screen-plugin-name__by">{s.by(plugin.by)}</span>
                        </span>
                      </span>
                    </td>
                    {group.key === "cards" ? (
                      <>
                        <td className="screen-cell-mono" translate="no">
                          {plugin.collections.length > 0 ? listNames(plugin.collections) : s.nothing}
                        </td>
                        <td>
                          <span className="screen-plugin-net">
                            <Icon name="lock" />
                            {s.noInternet}
                          </span>
                        </td>
                      </>
                    ) : (
                      <td className="screen-plugin-what">{plugin.description}</td>
                    )}
                    <td>
                      <Status kind="ok">{s.installed}</Status>
                    </td>
                    <td className="screen-cell-action">
                      <Button variant="quiet" aria-label={s.settingsFor(plugin.name)} onClick={() => setEditing(plugin)}>
                        {s.settings}
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      ))}

      {offered.length > 0 && (
        <section className="screen-section" aria-labelledby="from-oikome">
          <h2 id="from-oikome" className="oik-label screen-section-label">
            {s.fromOikome}
          </h2>
          <div className="screen-plugin-offers">
            {offered.map((plugin) => (
              <article key={plugin.id} className="oik-card screen-plugin-offer">
                <span className="screen-plugin-name">
                  <Tile icon="plugin" tone="sage" />
                  <span className="screen-plugin-name__text">
                    <span className="screen-plugin-name__name">{plugin.name}</span>
                    <span className="screen-plugin-name__by">{s.by(plugin.by)}</span>
                  </span>
                </span>
                <p className="screen-plugin-offer__what">{plugin.description}</p>
                {plugin.capabilities.cards.length > 0 && (
                  <p className="screen-plugin-offer__reads">{s.wants(plugin.reads)}</p>
                )}
                <span className="screen-plugin-offer__foot">
                  <span className="screen-plugin-net">
                    <Icon name="lock" />
                    {s.noInternet}
                  </span>
                  <Button aria-label={s.addFor(plugin.name)} onClick={() => setAdding({ plugin, source: { builtIn: plugin.id } })}>
                    {s.add}
                  </Button>
                </span>
              </article>
            ))}
          </div>
        </section>
      )}

      {(!registry.data || registry.error || registryOffered.length > 0) && (
      <section className="screen-section" aria-labelledby="official-repository">
        <h2 id="official-repository" className="oik-label screen-section-label">
          {s.repository}
        </h2>
        <p className="screen-help">{s.repositoryHint}</p>
        {registry.error ? (
          <p className="screen-help screen-help--error">
            {registry.error}{" "}
            <Button variant="quiet" onClick={registry.reload}>
              {strings.common.retry}
            </Button>
          </p>
        ) : registry.data ? (
          <div className="screen-plugin-offers">
            {registryOffered.map((entry) => (
              <article key={`${entry.plugin.id}:${entry.plugin.version}`} className="oik-card screen-plugin-offer">
                <span className="screen-plugin-name">
                  <Tile icon="plugin" tone="sage" />
                  <span className="screen-plugin-name__text">
                    <span className="screen-plugin-name__name">{entry.plugin.name}</span>
                    <span className="screen-plugin-name__by">{s.by(entry.plugin.by)}</span>
                  </span>
                </span>
                <p className="screen-plugin-offer__what">{entry.plugin.description}</p>
                <span className="screen-plugin-offer__foot">
                  <span className="screen-plugin-net">{bytes(entry.bytes)}</span>
                  {entry.plugin.installed ? (
                    <Status kind="ok">{s.installed}</Status>
                  ) : (
                    <Button
                      aria-label={s.addFromRepository(entry.plugin.name)}
                      onClick={() =>
                        setAdding({
                          plugin: entry.plugin,
                          source: { registry: { id: entry.plugin.id, version: entry.plugin.version } },
                        })
                      }
                    >
                      {s.add}
                    </Button>
                  )}
                </span>
              </article>
            ))}
          </div>
        ) : (
          <span className="oik-skel oik-skel--line" aria-hidden="true" />
        )}
      </section>
      )}

      {browsing && (
        <FolderBrowser
          title={s.chooseFolder}
          hint={() => s.folderHint}
          onClose={() => setBrowsing(false)}
          onChoose={async (path) => {
            setBrowsing(false);
            const plugin = await inspect.run(async () => inLanguage(await client.inspectPlugin({ path })), asSentence);
            if (plugin) setAdding({ plugin, source: { path } });
          }}
        />
      )}
      {adding && (
        <AddPlugin
          plugin={adding.plugin}
          source={adding.source}
          onClose={() => setAdding(undefined)}
          onAdded={() => {
            setAdding(undefined);
            plugins.reload();
            registry.reload();
          }}
        />
      )}
      {editing && (
        <PluginSettings
          plugin={editing}
          mine={mine}
          views={views}
          onClose={() => setEditing(undefined)}
          onChanged={() => {
            setEditing(undefined);
            plugins.reload();
          }}
        />
      )}
    </>
  );
}

/** What a card reads, and which collections fit: the owner chooses. */
function Collections({
  plugin,
  mine,
  views,
  chosen,
  onChange,
}: {
  plugin: Plugin;
  mine: SpaceSummary[];
  views?: Map<string, Holds>;
  chosen: Set<string>;
  onChange: (chosen: Set<string>) => void;
}) {
  const offered = mine.filter((space) => fits(plugin, views?.get(space.name)) || chosen.has(space.name));
  return (
    <fieldset className="screen-plugin-reads">
      <legend className="oik-field__label">{s.readsLabel}</legend>
      <p className="screen-help">{s.wants(plugin.reads)}</p>
      {!views ? (
        <span className="oik-skel oik-skel--line" aria-hidden="true" />
      ) : offered.length === 0 ? (
        <p className="screen-help">{s.noMatch}</p>
      ) : (
        <ul className="screen-lend-people">
          {offered.map((space) => (
            <li key={space.name}>
              <Checkbox
                label={<span className="screen-cell-mono">{space.name}</span>}
                checked={chosen.has(space.name)}
                onChange={(on) => {
                  const next = new Set(chosen);
                  if (on) next.add(space.name);
                  else next.delete(space.name);
                  onChange(next);
                }}
              />
            </li>
          ))}
        </ul>
      )}
    </fieldset>
  );
}

function NoInternet() {
  return (
    <div className="screen-lend-field">
      <span className="oik-field__label">{s.internetLabel}</span>
      <span className="screen-plugin-net screen-plugin-net--long">
        <Icon name="lock" />
        {s.cantReachInternet}
      </span>
    </div>
  );
}

function Heading({ plugin }: { plugin: Plugin }) {
  return (
    <>
      <span className="screen-plugin-name">
        <Tile icon="plugin" tone="sage" />
        <span className="screen-plugin-name__text">
          <span className="screen-plugin-name__name">{plugin.name}</span>
          <span className="screen-plugin-name__by">{s.signed(plugin.by, plugin.version)}</span>
        </span>
      </span>
      {plugin.description && <p className="oik-dialog__text">{plugin.description}</p>}
    </>
  );
}

const describe: Describe = (e) => (e.code === "invalid" || e.code === "not_found" ? asSentence(e) : undefined);

function AddPlugin({
  plugin,
  source,
  onClose,
  onAdded,
}: {
  plugin: Plugin;
  source: Source;
  onClose: () => void;
  onAdded: (plugin: Plugin) => void;
}) {
  const client = useClient();
  const toast = useToast();
  const add = useAction();
  return (
    <Dialog
      title={s.addTitle(plugin.name)}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button
            variant="primary"
            disabled={add.busy}
            onClick={async () => {
              const added = await add.run(
                () =>
                  "builtIn" in source
                    ? client.installPlugin({ builtIn: source.builtIn })
                    : "path" in source
                      ? client.installPlugin({ path: source.path })
                      : client.installRegistryPlugin(source.registry),
                describe,
              );
              if (added) {
                toast(s.added(inLanguage(added).name));
                onAdded(inLanguage(added));
              }
            }}
          >
            {s.add}
          </Button>
        </>
      }
    >
      <Heading plugin={plugin} />
      <p className="screen-help">{s.installHint}</p>
      <NoInternet />
      {add.error && <ErrorText>{add.error}</ErrorText>}
    </Dialog>
  );
}

function PluginSettings({
  plugin,
  mine,
  views,
  onClose,
  onChanged,
}: {
  plugin: Plugin;
  mine: SpaceSummary[];
  views?: Map<string, Holds>;
  onClose: () => void;
  onChanged: () => void;
}) {
  const client = useClient();
  const toast = useToast();
  const save = useAction();
  const remove = useAction();
  const [chosen, setChosen] = useState(() => new Set(plugin.collections));
  const [removing, setRemoving] = useState(false);
  const hasCards = plugin.capabilities.cards.length > 0;
  if (removing) {
    return (
      <ConfirmDialog
        title={s.removeTitle(plugin.name)}
        body={s.removeBody}
        confirmLabel={strings.sharing.remove}
        cancelLabel={strings.common.cancel}
        busy={remove.busy}
        error={remove.error}
        onConfirm={async () => {
          const left = await remove.run(() => client.removePlugin({ id: plugin.id }));
          if (left) {
            toast(s.removed(plugin.name));
            onChanged();
          }
        }}
        onCancel={() => {
          setRemoving(false);
          remove.clear();
        }}
      />
    );
  }
  return (
    <Dialog
      title={plugin.name}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button variant="destructive" className="screen-dialog-start" onClick={() => setRemoving(true)}>
            {s.remove}
          </Button>
          <Button onClick={onClose}>{hasCards ? strings.common.cancel : strings.common.close}</Button>
          {hasCards && (
            <Button
              variant="primary"
              disabled={save.busy}
              onClick={async () => {
                const saved = await save.run(
                  () => client.setPluginCollections({ id: plugin.id, collections: [...chosen] }),
                  describe,
                );
                if (saved) {
                  toast(s.saved(plugin.name));
                  onChanged();
                }
              }}
            >
              {strings.common.save}
            </Button>
          )}
        </>
      }
    >
      <Heading plugin={plugin} />
      {hasCards && <Collections plugin={plugin} mine={mine} views={views} chosen={chosen} onChange={setChosen} />}
      <NoInternet />
      {save.error && <ErrorText>{save.error}</ErrorText>}
    </Dialog>
  );
}
