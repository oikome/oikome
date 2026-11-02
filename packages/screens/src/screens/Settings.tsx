// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Settings, as grouped rows: the recovery key, the data center itself,
// where collections are kept, language.

import { useState } from "react";
import {
  Button,
  CopyButton,
  ErrorText,
  Group,
  GroupPanel,
  Row,
  TextField,
  Tile,
  useToast,
} from "@oikome/ui";
import type { IconName, TileTone } from "@oikome/ui";
import type { NodeInfo, StorageSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { RecoveryKeyPanel, ScreenHeader } from "../parts";
import { shortId } from "../format";
import { navigate } from "../router";
import { strings } from "../strings";

const s = strings.settings;

const STORAGE_LOOK: Record<StorageSummary["type"], [IconName, TileTone]> = {
  disk: ["computer", "stone"],
  webdav: ["nas", "sage"],
  folder: ["folder", "oat"],
  drive: ["drive", "periwinkle"],
};

/** Where collections can be kept, with how many are on each. */
function Storage() {
  const client = useClient();
  const storages = useLoad(() => client.listStorages(), []);
  return (
    <Group title={s.storageTitle} footnote={s.storageFootnote}>
      {storages.error && <Row title={storages.error} muted />}
      {(storages.data ?? []).map((x) => {
        const [icon, tone] = STORAGE_LOOK[x.type];
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
            title={x.name ?? s.thisComputer}
            meta={
              <>
                {meta}
                <br />
                <span className="oik-mono" translate="no">
                  {x.place}
                </span>
              </>
            }
          />
        );
      })}
    </Group>
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
    <div className="screen-column">
      <ScreenHeader title={s.title} />

      <Group title={s.recoveryTitle}>
        <Row
          alignStart
          leading={<Tile icon="key" tone="apricot" />}
          title={
            info.recoveryKeySaved ? (
              <span className="screen-status-ok">{s.backedUp}</span>
            ) : (
              <span className="screen-status-warn">{s.notBackedUp}</span>
            )
          }
          meta={s.recoveryBody}
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

      <Group title={s.centerTitle} footnote={s.idFootnote}>
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
            title={s.nameLabel}
            trailing={
              <>
                <span className="oik-row__value">{info.name || strings.defaultName}</span>
                <Button
                onClick={() => {
                  setName(info.name);
                  setEditing(true);
                }}
              >
                  {s.rename}
                </Button>
              </>
            }
          />
        )}
        <Row
          title={strings.sharing.yourId}
          meta={<span className="oik-mono" translate="no">{shortId(info.did)}</span>}
          trailing={
            <CopyButton
              value={shortId(info.did)}
              copyLabel={strings.common.copy}
              copiedLabel={strings.common.copied}
            />
          }
        />
        <Row title={s.dataFolder} meta={<span className="oik-mono" translate="no">{info.dataDir}</span>} />
        <Row title={s.version} trailing={<span className="oik-row__value">{info.version}</span>} />
      </Group>

      <Storage />

      <Group title={s.preferencesTitle}>
        <Row
          title={s.languageTitle}
          meta={s.languageMore}
          trailing={<span className="oik-row__value">{s.language}</span>}
        />
      </Group>

    </div>
  );
}
