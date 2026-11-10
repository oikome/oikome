// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Plugins: the cards others (and Oikome) made for Home, what each may read,
// and that none reaches the internet. Adding one shows what it wants to
// read and which collections fit; the owner chooses, and the node grants
// exactly those to the plugin.

import { useEffect, useMemo, useState } from "react";
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
import type { NodeInfo, Plugin, SpaceSummary } from "@oikome/client";
import { loadPreview } from "../content";
import type { View } from "../content";
import { inLanguage, pluginCollection } from "../plugins";
import { asSentence, useAction, useClient, useLoad } from "../data";
import type { Describe } from "../data";
import { ScreenHeader } from "../parts";
import { FolderBrowser } from "../storage";
import { strings } from "../strings";
import { listNames } from "../format";

const s = strings.plugins;

/** Where a plugin to add comes from. */
type Source = { builtIn: string } | { path: string };

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
        const columns = preview?.rows ? pluginCollection(name, preview.rows).columns.map((c) => c.type) : [];
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

/** Whether a collection is the kind a plugin reads, with the columns it needs. */
function fits(plugin: Plugin, holds: Holds | undefined): boolean {
  if (!holds) return false;
  return (plugin.shape === "any" || plugin.shape === holds.view) && plugin.columns.every((c) => holds.columns.has(c));
}

/** Home's cards with this plugin's added, when the owner arranged Home. */
function withCard(info: NodeInfo, plugin: Plugin) {
  const id = `plugin:${plugin.id}`;
  if (!info.homeCards || info.homeCards.some((card) => card.id === id)) return undefined;
  return [...info.homeCards, { id, width: plugin.width }];
}

function onHome(info: NodeInfo, plugin: Plugin): boolean {
  // Before Home is arranged, every plugin's card is on it.
  return !info.homeCards || info.homeCards.some((card) => card.id === `plugin:${plugin.id}`);
}

export function Plugins({ info, onInfo }: { info: NodeInfo; onInfo: (info: NodeInfo) => void }) {
  const client = useClient();
  const plugins = useLoad(async () => (await client.listPlugins()).map(inLanguage), []);
  const { mine, views } = useCollectionViews();
  const [adding, setAdding] = useState<{ plugin: Plugin; source: Source }>();
  const [browsing, setBrowsing] = useState(false);
  const [editing, setEditing] = useState<Plugin>();
  const inspect = useAction();

  const installed = (plugins.data ?? []).filter((p) => p.installed);
  const offered = (plugins.data ?? []).filter((p) => !p.installed && p.builtIn);

  return (
    <>
      <ScreenHeader title={s.title} lede={s.lede}>
        <Button icon="folder" onClick={() => setBrowsing(true)}>
          {s.addFromFolder}
        </Button>
      </ScreenHeader>
      {(plugins.error ?? inspect.error) && <ErrorText>{plugins.error ?? inspect.error}</ErrorText>}

      {plugins.data && installed.length === 0 && <p className="screen-panel__empty">{s.none}</p>}
      {installed.length > 0 && (
        <div className="oik-table-wrap screen-table-in-card">
          <table className="oik-table screen-plugins" aria-label={s.title}>
            <thead>
              <tr>
                <th scope="col">{s.colPlugin}</th>
                <th scope="col">{s.colReads}</th>
                <th scope="col">{s.colInternet}</th>
                <th scope="col">{s.colStatus}</th>
                <th scope="col">
                  <span className="oik-visually-hidden">{s.colActions}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {installed.map((plugin) => (
                <tr key={plugin.id}>
                  <td>
                    <span className="screen-plugin-name">
                      <Tile icon="plugin" tone="stone" />
                      <span className="screen-plugin-name__text">
                        <span className="screen-plugin-name__name">{plugin.name}</span>
                        <span className="screen-plugin-name__by">{s.by(plugin.by)}</span>
                      </span>
                    </span>
                  </td>
                  <td className="screen-cell-mono" translate="no">
                    {plugin.collections.length > 0 ? listNames(plugin.collections) : s.nothing}
                  </td>
                  <td>
                    <span className="screen-plugin-net">
                      <Icon name="lock" />
                      {s.noInternet}
                    </span>
                  </td>
                  <td>
                    <Status kind={onHome(info, plugin) ? "ok" : "idle"}>
                      {onHome(info, plugin) ? s.onHome : s.notOnHome}
                    </Status>
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
      )}

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
                <p className="screen-plugin-offer__reads">{s.wants(plugin.reads)}</p>
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
          mine={mine}
          views={views}
          onClose={() => setAdding(undefined)}
          onAdded={async (plugin) => {
            setAdding(undefined);
            plugins.reload();
            const cards = withCard(info, plugin);
            if (cards) {
              const updated = await client.setHomeCards({ cards }).catch(() => undefined);
              if (updated) onInfo(updated);
            }
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

/** Its name and maker, what it reads, and which collections fit: the owner chooses. */
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
  // The ones that fit, and any it may read already.
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
  mine,
  views,
  onClose,
  onAdded,
}: {
  plugin: Plugin;
  source: Source;
  mine: SpaceSummary[];
  views?: Map<string, Holds>;
  onClose: () => void;
  onAdded: (plugin: Plugin) => void;
}) {
  const client = useClient();
  const toast = useToast();
  const add = useAction();
  const [chosen, setChosen] = useState<Set<string>>(() => new Set(plugin.collections));
  // Once the collections are known, the ones that fit are chosen to start.
  const fitting = useMemo(
    () => (views ? mine.filter((space) => fits(plugin, views.get(space.name))).map((space) => space.name) : []),
    [views, mine, plugin],
  );
  // Only what it needs: one fitting collection is chosen for the owner; of
  // several, the owner chooses.
  useEffect(() => {
    if (views && plugin.collections.length === 0 && fitting.length === 1) setChosen(new Set(fitting));
  }, [views, fitting, plugin.collections.length]);
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
            disabled={add.busy || chosen.size === 0}
            onClick={async () => {
              const added = await add.run(
                () =>
                  client.installPlugin({
                    ...("builtIn" in source ? { builtIn: source.builtIn } : { path: source.path }),
                    collections: [...chosen],
                  }),
                describe,
              );
              if (added) {
                toast(s.added(inLanguage(added).name));
                onAdded(inLanguage(added));
              }
            }}
          >
            {s.addToHome}
          </Button>
        </>
      }
    >
      <Heading plugin={plugin} />
      <Collections plugin={plugin} mine={mine} views={views} chosen={chosen} onChange={setChosen} />
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
          <Button onClick={onClose}>{strings.common.cancel}</Button>
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
        </>
      }
    >
      <Heading plugin={plugin} />
      <Collections plugin={plugin} mine={mine} views={views} chosen={chosen} onChange={setChosen} />
      <NoInternet />
      {save.error && <ErrorText>{save.error}</ErrorText>}
    </Dialog>
  );
}
