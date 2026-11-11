// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Settings, as grouped rows: the recovery key, the data center itself,
// where collections are kept, language.

import { useEffect, useState } from "react";
import {
  Button,
  ConfirmDialog,
  CopyButton,
  Dialog,
  ErrorText,
  Group,
  GroupPanel,
  Menu,
  Row,
  Segmented,
  Select,
  Status,
  Switch,
  TextField,
  Tile,
  useToast,
} from "@oikome/ui";
import type { IconName, TileTone } from "@oikome/ui";
import type { Network, NodeInfo, ReachPolicy, StorageSummary } from "@oikome/client";
import type { StatusKind } from "@oikome/ui";
import { useAction, useClient, useLoad } from "../data";
import { LanguageSelect, RecoveryKeyPanel, ScreenHeader } from "../parts";
import { bytes, dateFromMs, homeRelative, listNames, middle, shortId } from "../format";
import { AddStorage, googleMessage } from "../storage";
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
  s3: ["bucket", "apricot"],
  gdrive: ["drive", "mist"],
  dropbox: ["drive", "periwinkle"],
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

/**
 * Where collections can be kept, with how many are on each, and which one
 * new ones go to. `google` says how connecting Google Drive went on return.
 */
function Storage({ google }: { google: string | null }) {
  const client = useClient();
  const toast = useToast();
  const storages = useLoad(() => client.listStorages(), []);
  // Said once: the flag leaves the address, so a reload doesn't say it again.
  const [googleNote] = useState(() => googleMessage(google));
  useEffect(() => {
    if (!google) return;
    if (google === "added") toast(strings.toast.storageAdded(strings.storage.gdrive));
    navigate("/settings", { replace: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const spaces = useLoad(() => client.listSpaces(), []);
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<StorageSummary>();
  const change = useAction();
  const remove = useAction();
  const stopRemoving = () => {
    setRemoving(undefined);
    remove.clear();
  };
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
      {googleNote && <Row title={googleNote} muted />}
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
                {/* Local is always there; places added later can be removed. */}
                {(!x.isDefault || x.name !== null) && (
                  <Menu
                    label={strings.common.moreActions(label)}
                    items={[
                      ...(x.isDefault
                        ? []
                        : [
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
                          ]),
                      ...(x.name === null
                        ? []
                        : [{ label: strings.storage.remove, destructive: true, onSelect: () => setRemoving(x) }]),
                    ]}
                  />
                )}
              </>
            }
          />
        );
      })}
      {change.error && <Row title={change.error} muted />}
      {removing && removing.spaces > 0 && (
        <Dialog
          title={strings.storage.stillHoldsTitle(removing.label)}
          closeLabel={strings.common.close}
          onClose={stopRemoving}
          footer={
            <Button variant="primary" onClick={stopRemoving}>
              {strings.common.done}
            </Button>
          }
        >
          <p className="oik-dialog__text">
            {strings.storage.stillHoldsBody(
              (spaces.data ?? []).filter((space) => space.storage === removing.name).map((space) => space.name),
              removing.spaces,
            )}
          </p>
        </Dialog>
      )}
      {removing && removing.spaces === 0 && removing.name !== null && (
        <ConfirmDialog
          title={strings.storage.removeTitle(removing.label)}
          body={
            removing.type === "dropbox"
              ? strings.storage.removeBodyDropbox
              : removing.type === "gdrive"
                ? strings.storage.removeBodyGdrive
                : removing.encrypted
                ? strings.storage.removeBodyEncrypted
                : strings.storage.removeBody
          }
          confirmLabel={strings.storage.removeYes}
          cancelLabel={strings.common.cancel}
          busy={remove.busy}
          error={remove.error}
          onConfirm={async () => {
            const name = removing.name;
            if (name === null) return;
            const done = await remove.run(() => client.removeStorage({ storage: name }));
            if (done) {
              toast(strings.toast.storageRemoved(removing.label));
              stopRemoving();
              storages.reload();
            }
          }}
          onCancel={stopRemoving}
        />
      )}
      {adding && (
        <AddStorage
          returnTo="settings"
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
      <GroupPanel>
        <LanguageSelect label={s.languageTitle} />
      </GroupPanel>
    </Group>
  );
}

const REACH_LOOK: Record<Network["reachable"], StatusKind> = {
  direct: "ok",
  through_contact: "ok",
  local_only: "off",
  opening: "run",
  router_refused: "bad",
  shared_address: "bad",
};

/**
 * Whether people outside this computer's network reach it, for what, and
 * what Oikome does when something needs that (3.12).
 */
function NetworkSettings() {
  const client = useClient();
  const toast = useToast();
  const network = useLoad(() => client.network(), []);
  const save = useAction();
  const [changed, setChanged] = useState<Network>();
  const n = changed ?? network.data;
  if (!n) return network.error ? <Group title={s.networkTitle}><Row title={network.error} muted /></Group> : null;
  const reachable =
    n.reachable === "through_contact" ? s.reachable.through_contact(n.through ?? "") : s.reachable[n.reachable];
  const help =
    n.reachable === "router_refused" || n.reachable === "shared_address" ? s.reachHelp[n.reachable] : undefined;
  const openFor =
    n.openFor.length === 0
      ? s.openForNothing
      : listNames(
          n.openFor.map((o) => (o.kind === "share" ? s.openForShare(o.space ?? "", o.name) : s.openForLend(o.name))),
        );
  const change = async (request: { policy?: ReachPolicy; helpContacts?: boolean }) => {
    const updated = await save.run(() => client.setNetwork(request));
    if (updated) {
      setChanged(updated);
      toast(s.networkSaved);
    }
  };
  return (
    <Group title={s.networkTitle} footnote={s.networkFootnote}>
      <Row
        overline={s.reachableLabel}
        title={
          <span className="screen-row-stack">
            <Status kind={REACH_LOOK[n.reachable]}>{reachable}</Status>
            {help && <span className="screen-row-note">{help}</span>}
          </span>
        }
      />
      <Row overline={s.openForLabel} title={<span className="screen-row-prose">{openFor}</span>} />
      <GroupPanel>
        <Select
          label={s.whenNeeded}
          value={n.policy}
          onChange={(value) => void change({ policy: value as ReachPolicy })}
          options={[
            { value: "ask", label: s.policy.ask },
            { value: "always", label: s.policy.always },
            { value: "never", label: s.policy.never },
          ]}
        />
      </GroupPanel>
      <Row
        title={
          <span className="screen-row-stack">
            {s.helpContacts}
            <span className="screen-row-note">{s.helpContactsBody}</span>
          </span>
        }
        trailing={
          <Switch
            label={s.helpContacts}
            checked={n.helpContacts}
            disabled={save.busy}
            onChange={(on) => void change({ helpContacts: on })}
          />
        }
      />
      {save.error && <Row title={save.error} muted />}
    </Group>
  );
}

export function Settings({
  info,
  onInfo,
  backup,
  google,
}: {
  info: NodeInfo;
  onInfo: (info: NodeInfo) => void;
  backup: boolean;
  /** How connecting Google Drive went, on coming back from Google's page. */
  google: string | null;
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
          <Storage google={google} />
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
          <NetworkSettings />
        </div>
      </div>
    </>
  );
}
