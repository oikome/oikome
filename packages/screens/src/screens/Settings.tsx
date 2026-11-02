// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Settings: the recovery key, the data center itself, language, advanced.

import { useState } from "react";
import { Button, Card, CopyField, ErrorText, Group, Icon, Row, TextField, Tile } from "@oikome/ui";
import type { NodeInfo } from "@oikome/client";
import { useAction, useClient } from "../data";
import { RecoveryKeyPanel } from "../parts";
import { navigate } from "../router";
import { strings } from "../strings";

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
  const [name, setName] = useState(info.name);
  const rename = useAction();
  const s = strings.settings;

  return (
    <>
      <header className="screen-header">
        <h1 className="screen-title">{s.title}</h1>
      </header>
      <div className="screen-stack">
        <Card>
          <div className="screen-detail__head">
            <Tile icon="key" tone="apricot" />
            <div className="screen-detail__name">
              <h2 className="screen-subtitle">{s.recoveryTitle}</h2>
              {info.recoveryKeySaved ? (
                <span className="screen-status-ok">{s.backedUp}</span>
              ) : (
                <span className="screen-status-warn">
                  <Icon name="warning" />
                  {s.notBackedUp}
                </span>
              )}
            </div>
          </div>
          <p className="screen-lede">{s.recoveryBody}</p>
          {showKey ? (
            <RecoveryKeyPanel
              doneLabel={strings.common.done}
              onDone={(updated) => {
                onInfo(updated);
                setShowKey(false);
              }}
            />
          ) : (
            <Button
              variant={info.recoveryKeySaved ? "secondary" : "primary"}
              onClick={() => setShowKey(true)}
            >
              {info.recoveryKeySaved ? s.showAgain : strings.backup.backUpNow}
            </Button>
          )}
        </Card>

        <Card>
          <h2 className="screen-subtitle">{s.centerTitle}</h2>
          <form
            className="screen-form"
            onSubmit={async (e) => {
              e.preventDefault();
              const updated = await rename.run(() => client.renameNode({ name }));
              if (updated) onInfo(updated);
            }}
          >
            <TextField
              label={strings.onboarding.nameLabel}
              value={name}
              onChange={setName}
              placeholder={strings.defaultName}
            />
            {rename.error && <ErrorText>{rename.error}</ErrorText>}
            <Button type="submit" disabled={rename.busy || name.trim() === info.name}>
              {strings.common.save}
            </Button>
          </form>
          <div className="screen-result">
            <h3 className="oik-field__label">{strings.sharing.yourId}</h3>
            <p className="screen-muted">{strings.sharing.yourIdBody}</p>
            <CopyField
              value={info.did}
              copyLabel={strings.common.copy}
              copiedLabel={strings.common.copied}
            />
          </div>
          <p className="screen-muted screen-result">
            {s.version} {info.version} · {s.dataFolder}{" "}
            <span className="screen-path">{info.dataDir}</span>
          </p>
        </Card>

        <Card>
          <h2 className="screen-subtitle">{s.languageTitle}</h2>
          <p className="screen-muted">
            {s.language}. {s.languageMore}
          </p>
        </Card>

        <Group title={s.advancedTitle}>
          <Row
            leading={<Tile icon="terminal" tone="stone" />}
            title={s.sqlTitle}
            meta={s.sqlBody}
            chevron
            onClick={() => navigate("/settings/advanced")}
          />
        </Group>
      </div>
    </>
  );
}
