// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The share dialog: choose who and how, see who already has access and stop
// it there; then the invite code, once, in the same dialog. Opened from a
// collection (its name in the title) or from Sharing (a Collection choice
// added at the top).

import { useState } from "react";
import {
  Avatar,
  Button,
  CopyField,
  Dialog,
  ErrorText,
  InsetWell,
  Select,
  Tile,
} from "@oikome/ui";
import type { TileTone } from "@oikome/ui";
import type { Contact, NewShare, Share, SpaceSummary } from "@oikome/client";
import { useAction, useClient } from "../data";
import { dateFromSeconds } from "../format";
import { strings } from "../strings";

const s = strings.sharing;
type Ends = "7" | "30" | "365";
type Access = "view" | "copy";

export function ShareDialog({
  collection,
  spaces,
  contacts,
  shares,
  toneOf,
  onClose,
  onChange,
}: {
  /** The collection it was opened from; without one, a choice is offered. */
  collection?: string;
  /** Local collections, the only ones that can be shared. */
  spaces: SpaceSummary[];
  contacts: Contact[];
  shares: Share[];
  toneOf: (name: string) => TileTone;
  onClose: () => void;
  /** A share was made or stopped: lists behind the dialog reload. */
  onChange: () => void;
}) {
  const client = useClient();
  const [space, setSpace] = useState(collection ?? spaces[0]?.name ?? "");
  const [did, setDid] = useState(contacts[0]?.did ?? "");
  const [access, setAccess] = useState<Access>("view");
  const [ends, setEnds] = useState<Ends>("30");
  const [made, setMade] = useState<NewShare>();
  const share = useAction();
  const stop = useAction();

  const copyable = spaces.find((x) => x.name === space)?.kind === "analytical";
  const chosen = contacts.find((c) => c.did === did);
  const withAccess = shares.filter((x) => x.space === space && x.status === "active");
  const close = strings.common.close;

  if (made) {
    const name = contacts.find((c) => c.did === made.did)?.name ?? "";
    return (
      <Dialog
        title={s.inviteTitle(name)}
        closeLabel={close}
        onClose={onClose}
        footer={
          <Button variant="primary" onClick={onClose}>
            {strings.common.done}
          </Button>
        }
      >
        <InsetWell>
          <CopyField value={made.token} copyLabel={strings.common.copy} copiedLabel={strings.common.copied} />
        </InsetWell>
        <p className="screen-help">{s.inviteBody(name)}</p>
      </Dialog>
    );
  }

  const title = collection ? s.shareTitle(collection) : s.newShare;
  if (spaces.length === 0 || contacts.length === 0) {
    return (
      <Dialog title={title} closeLabel={close} onClose={onClose} footer={<Button onClick={onClose}>{close}</Button>}>
        <p className="oik-dialog__text">{spaces.length === 0 ? s.needCollection : s.needContact}</p>
      </Dialog>
    );
  }

  return (
    <Dialog
      title={title}
      closeLabel={close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button
            type="submit"
            form="share-form"
            variant="primary"
            disabled={share.busy}
            aria-label={`${s.share} ${space} with ${chosen?.name ?? ""}`}
          >
            {s.share}
          </Button>
        </>
      }
    >
      <form
        id="share-form"
        onSubmit={async (e) => {
          e.preventDefault();
          const result = await share.run(() =>
            client.shareSpace({
              space,
              did,
              allowCopy: copyable && access === "copy",
              expiresInDays: Number(ends),
            }),
          );
          if (result) {
            setMade(result);
            onChange();
          }
        }}
      >
        {!collection && (
          <Select
            label={s.collectionLabel}
            value={space}
            onChange={(value) => {
              setSpace(value);
              if (spaces.find((x) => x.name === value)?.kind !== "analytical") setAccess("view");
            }}
            leading={<Tile icon="collection" tone={toneOf(space)} small />}
            options={spaces.map((x) => ({ value: x.name, label: x.name }))}
          />
        )}
        <div className="screen-share-pair">
          <Select
            label={s.withLabel}
            value={did}
            onChange={setDid}
            leading={<Avatar name={chosen?.name ?? "?"} small />}
            options={contacts.map((c) => ({ value: c.did, label: c.name }))}
          />
          <Select
            label={s.accessLabel}
            value={copyable ? access : "view"}
            onChange={(value) => setAccess(value as Access)}
            options={[
              { value: "view", label: s.accessView },
              {
                value: "copy",
                label: copyable ? s.accessCopy : s.accessCopyUnavailable,
                disabled: !copyable,
              },
            ]}
          />
        </div>
        <p className="screen-help screen-share-help">
          {copyable && access === "copy" ? s.copyHelp : s.viewHelp}
        </p>
        <div className="screen-share-ends">
          <Select
            label={s.endsLabel}
            value={ends}
            onChange={(value) => setEnds(value as Ends)}
            options={[
              { value: "7", label: s.ends7 },
              { value: "30", label: s.ends30 },
              { value: "365", label: s.endsYear },
            ]}
          />
        </div>
        {share.error && <ErrorText>{share.error}</ErrorText>}
      </form>

      <section className="screen-share-access" aria-label={s.peopleWithAccess}>
        <h3 className="screen-share-access__title">{s.peopleWithAccess}</h3>
        <ul className="screen-share-access__list">
          <li className="screen-share-person">
            <Avatar name={strings.common.you} you />
            <span className="screen-share-person__text">
              <span className="screen-share-person__name">{strings.common.you}</span>
            </span>
            <span className="screen-share-person__role">{s.owner}</span>
          </li>
          {withAccess.map((x) => {
            const name = x.contactName ?? strings.common.unknown;
            return (
              <li key={x.id} className="screen-share-person">
                <Avatar name={name} />
                <span className="screen-share-person__text">
                  <span className="screen-share-person__name">{name}</span>
                  <span className="screen-share-person__meta">
                    {s.accessUntil(x.access === "copy", dateFromSeconds(x.expiresAt))}
                  </span>
                </span>
                <Button
                  variant="quiet"
                  disabled={stop.busy}
                  aria-label={`${s.stop}: ${x.space}, ${name}`}
                  onClick={async () => {
                    const done = await stop.run(() => client.revokeShare({ id: x.id }));
                    if (done) onChange();
                  }}
                >
                  {s.stop}
                </Button>
              </li>
            );
          })}
        </ul>
        {stop.error && <ErrorText>{stop.error}</ErrorText>}
      </section>
    </Dialog>
  );
}
