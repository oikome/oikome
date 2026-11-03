// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Settings, as grouped rows: the recovery key, the data center itself,
// where collections are kept, language.

import { useState } from "react";
import {
  Button,
  Checkbox,
  CopyButton,
  Dialog,
  ErrorText,
  Group,
  GroupPanel,
  Menu,
  Row,
  Segmented,
  Status,
  TextField,
  Tile,
  useToast,
} from "@oikome/ui";
import type { IconName, TileTone } from "@oikome/ui";
import type { NodeInfo, StorageSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { RecoveryKeyPanel, ScreenHeader } from "../parts";
import { bytes, dateFromMs, homeRelative, middle, shortId } from "../format";
import { addPlanned, useStoragePicker } from "../storage";
import { applyTheme, savedTheme } from "../theme";
import type { Theme } from "../theme";
import { navigate } from "../router";
import { strings } from "../strings";

const s = strings.settings;

const STORAGE_LOOK: Record<StorageSummary["type"], [IconName, TileTone]> = {
  disk: ["computer", "stone"],
  webdav: ["nas", "sage"],
  folder: ["folder", "oat"],
  drive: ["drive", "periwinkle"],
};

/** Free space and a bar, where the place says. */
function Room({ place }: { place: StorageSummary }) {
  if (place.freeBytes == null || place.totalBytes == null || place.totalBytes === 0) return null;
  const used = (place.totalBytes - place.freeBytes) / place.totalBytes;
  return (
    <span className="screen-room-mini">
      <span className="screen-room-mini__figure">{strings.home.freeShort(bytes(place.freeBytes))}</span>
      <span className="screen-bar">
        <i style={{ width: `${Math.max(2, used * 100)}%` }} />
      </span>
    </span>
  );
}

/** Where collections can be kept, with how many are on each, and which one new ones go to. */
function Storage() {
  const client = useClient();
  const toast = useToast();
  const storages = useLoad(() => client.listStorages(), []);
  const [adding, setAdding] = useState(false);
  const change = useAction();
  return (
    <Group
      title={s.storageTitle}
      action={
        <Button variant="quiet" icon="plus" onClick={() => setAdding(true)}>
          {strings.storage.add}
        </Button>
      }
    >
      {storages.error && <Row title={storages.error} muted />}
      {(storages.data ?? []).map((x) => {
        const [icon, tone] = STORAGE_LOOK[x.type];
        const label = x.name === null ? s.thisComputer : x.label;
        const meta = [
          s.storageCount(x.spaces),
          x.name === null ? null : s.storageKind[x.type],
          x.encrypted ? s.encrypted : null,
        ]
          .filter(Boolean)
          .join(" · ");
        return (
          <Row
            key={x.name ?? ""}
            leading={<Tile icon={icon} tone={tone} />}
            title={
              <>
                {label}
                {x.isDefault && <span className="oik-tag screen-tag-inline">{strings.storage.isDefault}</span>}
              </>
            }
            meta={
              <span className="screen-ellipsis" translate="no" title={x.place}>
                {homeRelative(x.place)} · {meta}
              </span>
            }
            trailing={
              <>
                <Room place={x} />
                {!x.isDefault && (
                  <Menu
                    label={strings.common.moreActions(label)}
                    items={[
                      {
                        label: strings.storage.useForNew,
                        onSelect: async () => {
                          const updated = await change.run(() =>
                            client.setDefaultStorage({ storage: x.name }),
                          );
                          if (updated) {
                            toast(strings.toast.defaultStorage(label));
                            storages.reload();
                          }
                        },
                      },
                    ]}
                  />
                )}
              </>
            }
          />
        );
      })}
      {adding && (
        <AddStorage
          onClose={() => setAdding(false)}
          onAdded={(label) => {
            toast(strings.toast.storageAdded(label));
            setAdding(false);
            storages.reload();
          }}
        />
      )}
    </Group>
  );
}

/** How the app looks here: its style and its language. */
function Appearance() {
  const [theme, setTheme] = useState<Theme>(savedTheme);
  return (
    <Group title={s.appearanceTitle}>
      <Row
        title={s.styleTitle}
        trailing={
          <Segmented
            label={s.styleTitle}
            value={theme}
            onChange={(value) => {
              const next = value as Theme;
              applyTheme(next);
              setTheme(next);
            }}
            options={[
              { value: "ink", label: s.ink },
              { value: "bold", label: s.bold },
            ]}
          />
        }
      />
      <Row
        title={s.languageTitle}
        trailing={<span className="oik-row__value">{s.language}</span>}
      />
    </Group>
  );
}

/** Add a place: a cloud drive or drive found here, a NAS, or a folder. */
function AddStorage({ onClose, onAdded }: { onClose: () => void; onAdded: (label: string) => void }) {
  const client = useClient();
  const storage = useStoragePicker({ withThisComputer: false });
  const [makeDefault, setMakeDefault] = useState(false);
  const add = useAction();
  return (
    <Dialog
      title={strings.storage.add}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button type="submit" form="add-storage" variant="primary" disabled={add.busy}>
            {add.busy ? strings.storage.connecting : strings.storage.add}
          </Button>
        </>
      }
    >
      <form
        id="add-storage"
        onSubmit={async (e) => {
          e.preventDefault();
          const plan = storage.plan();
          if ("error" in plan) {
            add.fail(plan.error);
            return;
          }
          const added = await add.run(
            () => addPlanned(client, plan, makeDefault),
            (err) => (err.code === "invalid" ? strings.storage.unreachable : undefined),
          );
          if (added) onAdded(added.label);
        }}
      >
        {storage.picker}
        <Checkbox label={strings.storage.useForNew} checked={makeDefault} onChange={setMakeDefault} />
        {add.error && <ErrorText>{add.error}</ErrorText>}
      </form>
    </Dialog>
  );
}

export function Settings({
  info,
  onInfo,
  backup,
}: {
  info: NodeInfo;
  onInfo: (info: NodeInfo) => void;
  backup: boolean;
}) {
  const client = useClient();
  const [showKey, setShowKey] = useState(backup);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(info.name);
  const rename = useAction();
  const toast = useToast();
  // Opened from the banner (?backup=1): closing drops the flag, so a reload
  // or Forward does not open the key again.
  const close = () => {
    setShowKey(false);
    if (backup) navigate("/settings", { replace: true });
  };

  return (
    <>
      <ScreenHeader title={s.title} lede={s.lede} />
      <div className="screen-dash screen-dash--start screen-dash--split">
        <div>
          <Storage />
          <Group
            title={s.recoveryTitle}
            action={
              info.recoveryKeySaved ? (
                <Status kind="ok">
                  {info.recoveryKeySavedAtMs ? s.backedUpOn(dateFromMs(info.recoveryKeySavedAtMs)) : s.backedUp}
                </Status>
              ) : (
                <Status kind="bad">{s.notBackedUp}</Status>
              )
            }
          >
            <Row
              leading={<Tile icon="key" tone="apricot" />}
              title={<span className="screen-row-prose">{s.recoveryBody}</span>}
              trailing={
                !showKey && (
                  <Button
                    variant={info.recoveryKeySaved ? "secondary" : "primary"}
                    onClick={() => setShowKey(true)}
                  >
                    {info.recoveryKeySaved ? s.showAgain : strings.backup.backUpNow}
                  </Button>
                )
              }
            />
            {showKey && (
              <GroupPanel>
                <RecoveryKeyPanel
                  doneLabel={strings.common.done}
                  onDone={(updated) => {
                    toast(strings.toast.keySaved);
                    onInfo(updated);
                    close();
                  }}
                  onLater={close}
                  laterLabel={info.recoveryKeySaved ? strings.common.close : undefined}
                />
              </GroupPanel>
            )}
          </Group>
        </div>
        <div>
          <Appearance />
          <Group title={s.centerTitle}>
            {editing ? (
              <GroupPanel>
                <form
                  className="screen-inline-form"
                  onSubmit={async (e) => {
                    e.preventDefault();
                    const updated = await rename.run(() =>
                      client.renameNode({ name: name.trim() || strings.defaultName }),
                    );
                    if (updated) {
                      toast(strings.toast.renamed(updated.name));
                      onInfo(updated);
                      setEditing(false);
                    }
                  }}
                >
                  <TextField
                    label={s.nameLabel}
                    name="node-name"
                    value={name}
                    onChange={setName}
                    placeholder={strings.defaultName}
                    autoFocus
                  />
                  <Button
                    onClick={() => {
                      setEditing(false);
                      setName(info.name);
                      rename.clear();
                    }}
                  >
                    {strings.common.cancel}
                  </Button>
                  <Button type="submit" disabled={rename.busy}>
                    {strings.common.save}
                  </Button>
                </form>
                {rename.error && <ErrorText>{rename.error}</ErrorText>}
              </GroupPanel>
            ) : (
              <Row
                overline={s.nameLabel}
                title={info.name || strings.defaultName}
                trailing={
                  <Button
                    variant="quiet"
                    onClick={() => {
                      setName(info.name);
                      setEditing(true);
                    }}
                  >
                    {s.rename}
                  </Button>
                }
              />
            )}
            <Row
              overline={strings.sharing.yourId}
              title={
                <span className="oik-mono" translate="no" title={shortId(info.did)}>
                  {middle(shortId(info.did))}
                </span>
              }
              trailing={
                <CopyButton
                  value={shortId(info.did)}
                  copyLabel={strings.common.copyId}
                  copiedLabel={strings.common.copied}
                  iconOnly
                />
              }
            />
            <Row
              overline={s.dataFolder}
              title={
                <span className="oik-mono screen-ellipsis" translate="no" title={info.dataDir}>
                  {homeRelative(info.dataDir)}
                </span>
              }
            />
            <Row overline={s.version} title={<span className="oik-mono">{info.version}</span>} />
          </Group>
        </div>
      </div>
    </>
  );
}
