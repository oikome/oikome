// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// First run: name the data center, save the recovery key, done.

import { useState } from "react";
import { Button, Card, ErrorText, Icon, StepDots, TextField } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { NodeInfo } from "@oikome/client";
import { useAction, useClient } from "../data";
import { RecoveryKeyPanel } from "../parts";
import { strings } from "../strings";

const STEPS = 3;

/** A path under a home folder as ~/…, the way people know it. */
function homeRelative(path: string): string {
  return path.replace(/^\/(?:Users|home)\/[^/]+(?=\/|$)/, "~");
}

/** The large line glyph that opens each step. */
function Hero({ icon }: { icon: IconName }) {
  return (
    <span className="screen-hero" aria-hidden="true">
      <Icon name={icon} />
    </span>
  );
}

export function Onboarding({
  info,
  onInfo,
  onFinished,
}: {
  info: NodeInfo;
  onInfo: (info: NodeInfo) => void;
  onFinished: () => void;
}) {
  const client = useClient();
  const [step, setStep] = useState(info.name ? 2 : 1);
  const [name, setName] = useState(info.name);
  const [restoreHelp, setRestoreHelp] = useState(false);
  const save = useAction();
  const s = strings.onboarding;

  return (
    <main className="screen-onboarding">
      <Card className="screen-onboarding__card">
        <StepDots step={step} total={STEPS} label={s.stepOf(step, STEPS)} />
        {step === 1 && (
          <form
            className="screen-form"
            onSubmit={async (e) => {
              e.preventDefault();
              const updated = await save.run(() =>
                client.renameNode({ name: name.trim() || strings.defaultName }),
              );
              if (updated) {
                onInfo(updated);
                setStep(2);
              }
            }}
          >
            <Hero icon="mark" />
            <h1 className="screen-title">{s.welcomeTitle}</h1>
            <p className="screen-lede">{s.welcomeLede}</p>
            <TextField
              label={s.nameLabel}
              name="node-name"
              value={name}
              onChange={setName}
              placeholder={strings.defaultName}
              autoFocus
            />
            <div className="screen-location">
              <Icon name="computer" />
              <span className="screen-location__text">
                <span className="screen-location__title">{s.keptHere}</span>
                <span className="screen-path" title={info.dataDir} translate="no">
                  {homeRelative(info.dataDir)}
                </span>
              </span>
            </div>
            {save.error && <ErrorText>{save.error}</ErrorText>}
            <div className="screen-actions">
              <Button type="submit" variant="primary" large disabled={save.busy}>
                {s.continue}
              </Button>
              <Button variant="text" onClick={() => setRestoreHelp((v) => !v)}>
                {s.restoreLink}
              </Button>
            </div>
            {restoreHelp && <p className="screen-muted screen-result">{s.restoreHelp}</p>}
          </form>
        )}
        {step === 2 && (
          <>
            <Hero icon="key" />
            <h1 className="screen-title">{strings.recovery.title}</h1>
            <p className="screen-lede">{strings.recovery.lede}</p>
            <RecoveryKeyPanel
              doneLabel={s.continue}
              onDone={(updated) => {
                onInfo(updated);
                setStep(3);
              }}
              onLater={() => setStep(3)}
            />
          </>
        )}
        {step === 3 && (
          <>
            <Hero icon="check" />
            <h1 className="screen-title">{s.doneTitle}</h1>
            <p className="screen-lede">{s.doneLede}</p>
            <div className="screen-actions">
              <Button variant="primary" large onClick={onFinished}>
                {s.goToCollections}
              </Button>
            </div>
          </>
        )}
      </Card>
    </main>
  );
}
