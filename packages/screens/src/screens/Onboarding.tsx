// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// First run: name the data center and pick where its data goes (other
// places too, for those who want several at once), save the recovery key,
// done.

import { useEffect, useState } from "react";
import { Button, Card, ErrorText, Icon, TextField } from "@oikome/ui";
import type { IconName } from "@oikome/ui";
import type { NodeInfo } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { homeRelative } from "../format";
import { AddStorage, addPlanned, googleMessage, unreachableFor, useStoragePicker } from "../storage";
import { LanguageSelect, RecoveryKeyPanel } from "../parts";
import { language } from "../i18n/languages";
import { navigate } from "../router";
import { STORAGE_ICONS } from "./Collections";
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
  google,
}: {
  info: NodeInfo;
  onInfo: (info: NodeInfo) => void;
  onFinished: () => void;
  /** How connecting Google Drive went (`?google=`), on coming back from Google's page. */
  google: string | null;
}) {
  const client = useClient();
  // Back from Google's page without the drive: the first step again, to try
  // again or pick another place.
  const [step, setStep] = useState(info.name && !googleMessage(google) ? 2 : 1);
  const s = strings.onboarding;
  // A name is filled in, the owner's own when the computer knows it: a lazy
  // Continue is a fine answer.
  // Only a first name that reads as one ("Rosh"), not an account name ("z.jin").
  const first = info.ownerName.trim().split(/\s+/)[0] ?? "";
  const suggested = /^\p{Lu}[\p{L}'’-]*$/u.test(first) ? s.namedAfter(first) : strings.defaultName;
  const [name, setName] = useState(info.name || suggested);
  const [restoreHelp, setRestoreHelp] = useState(false);
  const save = useAction();
  // Where collections are kept: this computer unless the owner picks another place.
  const storage = useStoragePicker({ withThisComputer: true });
  // Places added besides the one new collections go to.
  const [addingMore, setAddingMore] = useState(false);
  const places = useLoad(() => client.listStorages(), []);
  const others = (places.data ?? []).filter((x) => x.name !== null);
  useEffect(() => {
    if (!google) return;
    const message = googleMessage(google);
    if (message) save.fail(message);
    // Back from connecting the drive new collections go to (the data
    // center was named first): the examples now. A drive added besides it
    // comes back before that.
    if (google === "added" && info.name) {
      void client.addExamples({ language: language() }).catch(() => undefined);
    }
    navigate("/onboarding", { replace: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <main className="screen-onboarding">
      <Card className="screen-onboarding__card">
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
                  // Google Drive is connected on Google's page, which comes
                  // back here; the examples are added then.
                  await addPlanned(client, plan, true, "onboarding");
                  // Something to look at on the first day; once, and only on
                  // a data center with nothing in it yet.
                  await client.addExamples({ language: language() }).catch(() => undefined);
                  return renamed;
                },
                (err) => (err.code === "invalid" && "add" in plan && plan.add ? unreachableFor(plan) : undefined),
              );
              if (updated) {
                onInfo(updated);
                setStep(2);
              }
            }}
          >
            <div className="screen-onboarding__top">
              <Hero icon="mark" />
              <LanguageSelect label={s.language} compact />
            </div>
            <h1 className="screen-title">{s.welcomeTitle}</h1>
            <p className="screen-lede">{s.welcomeLede}</p>
            <TextField
              label={s.nameLabel}
              name="node-name"
              value={name}
              onChange={setName}
              placeholder={suggested}
              selectOnFocus
              autoFocus
            />
            {storage.picker}
            {/* The examples' files are on this computer, so they stay on Local. */}
            {storage.choice !== "here" && <p className="screen-help">{s.examplesStayHere}</p>}
            {/* Other places, for those who want several from the start: beside
                the places, quieter than they are; the ones added listed above. */}
            {others.length > 0 && (
              <div className="screen-added">
                <span className="oik-field__label">{s.alsoAddedLabel}</span>
                {others.map((x) => (
                  <div key={x.name} className="screen-added__row">
                    <Icon name={STORAGE_ICONS[x.type]} />
                    <span className="screen-added__name" translate="no">
                      {x.label}
                    </span>
                    <span className="screen-path" translate="no">
                      {homeRelative(x.place)}
                    </span>
                  </div>
                ))}
              </div>
            )}
            <button type="button" className="screen-add-place" onClick={() => setAddingMore(true)}>
              <span className="screen-add-place__icon">
                <Icon name="plus" />
              </span>
              <span className="screen-add-place__text">
                <span className="screen-add-place__title">{s.addMore}</span>
                <span className="screen-add-place__meta">{s.addMoreSub}</span>
              </span>
            </button>
            {save.error && <ErrorText>{save.error}</ErrorText>}
            <div className="screen-actions">
              <Button type="submit" variant="primary" large disabled={save.busy}>
                {save.busy ? strings.storage.connecting : storage.toGoogle ? strings.storage.toGoogle : s.continue}
              </Button>
            </div>
            <p className="screen-onboarding__step">{s.stepNext(1, STEPS, s.nextKey)}</p>
            <button type="button" className="screen-onboarding__restore" onClick={() => setRestoreHelp((v) => !v)}>
              {s.restoreLink}
            </button>
            {restoreHelp && <p className="screen-muted screen-result">{s.restoreHelp}</p>}
          </form>
        )}
        {addingMore && (
          <AddStorage
            returnTo="onboarding"
            onClose={() => setAddingMore(false)}
            onAdded={() => {
              setAddingMore(false);
              places.reload();
            }}
          />
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
            <p className="screen-onboarding__step">{s.stepNext(2, STEPS, s.nextSet)}</p>
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
            <p className="screen-onboarding__step">{s.stepOf(3, STEPS)}</p>
          </>
        )}
      </Card>
    </main>
  );
}
