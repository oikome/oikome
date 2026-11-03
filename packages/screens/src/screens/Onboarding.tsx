// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// First run: name the data center, save the recovery key, done.

import { useState } from "react";
import { Button, Card, ErrorText, Icon, StepDots, TextField } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { NodeInfo } from "@oikome/client";
import { useAction, useClient } from "../data";
import { addPlanned, useStoragePicker } from "../storage";
import { RecoveryKeyPanel } from "../parts";
import { strings } from "../strings";

const STEPS = 3;

/** The large line glyph that opens each step. */
function Hero({ icon }: { icon: IconName }) {
  return (
    <span className="screen-glyph" aria-hidden="true">
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
  // A name is filled in: a lazy Continue is a fine answer.
  const [name, setName] = useState(info.name || strings.defaultName);
  const [restoreHelp, setRestoreHelp] = useState(false);
  const save = useAction();
  const s = strings.onboarding;
  // Where collections are kept: this computer unless the owner picks another place.
  const storage = useStoragePicker({ withThisComputer: true });

  return (
    <main className="screen-onboarding">
      <Card className="screen-onboarding__card">
        <StepDots step={step} total={STEPS} label={s.stepOf(step, STEPS)} />
        {step === 1 && (
          <form
            className="screen-form"
            onSubmit={async (e) => {
              e.preventDefault();
              const plan = storage.plan();
              if ("error" in plan) {
                save.fail(plan.error);
                return;
              }
              const updated = await save.run(
                async () => {
                  const renamed = await client.renameNode({
                    name: name.trim() || strings.defaultName,
                  });
                  // The place picked keeps new collections, the examples first.
                  await addPlanned(client, plan, true);
                  // Something to look at on the first day; once, and only on
                  // a data center with nothing in it yet.
                  await client.addExamples().catch(() => undefined);
                  return renamed;
                },
                (err) => (err.code === "invalid" && plan.add ? strings.storage.unreachable : undefined),
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
              selectOnFocus
              autoFocus
            />
            {storage.picker}
            <p className="screen-help">{strings.storage.later}</p>
            {save.error && <ErrorText>{save.error}</ErrorText>}
            <div className="screen-actions">
              <Button type="submit" variant="primary" large disabled={save.busy}>
                {save.busy ? strings.storage.connecting : s.continue}
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
