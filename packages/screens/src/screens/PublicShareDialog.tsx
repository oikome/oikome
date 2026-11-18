// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { useState } from "react";
import { Button, Checkbox, CopyField, Dialog, ErrorText, InsetWell, Select, useToast } from "@oikome/ui";
import type { PublicShare } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { dateFromSeconds } from "../format";
import { toastReach, useReachPrompt } from "../reach";
import { strings } from "../strings";

const s = strings.publicShare;

export function PublicShareDialog({
  collection,
  onClose,
  onBack,
}: {
  collection: string;
  onClose: () => void;
  onBack?: () => void;
}) {
  const client = useClient();
  const toast = useToast();
  const action = useAction();
  const stop = useAction();
  const shares = useLoad(() => client.listPublicShares(), []);
  const reach = useReachPrompt();
  const [access, setAccess] = useState<"view" | "copy">("view");
  // How long the link works; the request, row and time limits keep the
  // node's defaults and are not asked about.
  const [days, setDays] = useState("30");
  const [privacy, setPrivacy] = useState(true);
  // Location is only a question for a collection that holds photos.
  const photos = useLoad(
    async () =>
      (await client.listFiles({ space: collection, deep: true, limit: 1000 })).files.some((file) => file.kind === "image"),
    [collection],
  );
  const [made, setMade] = useState<PublicShare>();
  const current = (shares.data ?? []).filter((share) => share.space === collection);
  const link = (share: PublicShare) => `${window.location.origin}/public/${share.id}`;

  const create = async () => {
    const what = { kind: "share" as const, collection, name: s.anyone };
    const choice = await reach.ask(what);
    if (choice === null) return;
    const share = await action.run(() =>
      client.createPublicShare({
        space: collection,
        allowCopy: access === "copy",
        removeLocation: privacy,
        expiresAtMs: days === "" ? undefined : Date.now() + Number(days) * 86_400_000,
        reach: choice,
      }),
    );
    if (!share) return;
    setMade(share);
    shares.reload();
    void toastReach(client, toast, what, choice);
  };

  if (made) {
    return (
      <Dialog
        title={s.ready}
        closeLabel={strings.common.close}
        onClose={onClose}
        footer={<Button variant="primary" onClick={onClose}>{strings.common.done}</Button>}
      >
        <InsetWell>
          <CopyField value={link(made)} copyLabel={strings.common.copy} copiedLabel={strings.common.copied} />
        </InsetWell>
        <p className="screen-help">{made.allowCopy ? s.copyReady : s.queryReady}</p>
        <p className="screen-help">{s.liveNote}</p>
      </Dialog>
    );
  }

  return (
    <>
      <Dialog
        title={s.title(collection)}
        closeLabel={strings.common.close}
        onClose={onClose}
        footer={
          <>
            {onBack ? <Button onClick={onBack}>{strings.sharing.backToContact}</Button> : <Button onClick={onClose}>{strings.common.cancel}</Button>}
            <Button variant="primary" disabled={action.busy} onClick={() => void create()}>
              {action.busy ? s.publishing : s.publish}
            </Button>
          </>
        }
      >
        <p className="oik-dialog__text">{s.body}</p>
        <Select
          label={s.access}
          value={access}
          onChange={(value) => setAccess(value as "view" | "copy")}
          options={[
            { value: "view", label: s.queryOnly },
            { value: "copy", label: s.allowCopy },
          ]}
        />
        <p className="screen-help">{access === "copy" ? s.copyHelp : s.queryHelp}</p>
        <Select
          label={s.until}
          value={days}
          onChange={setDays}
          options={[
            { value: "7", label: s.untilWeek },
            { value: "30", label: s.untilMonth },
            { value: "365", label: s.untilYear },
            { value: "", label: s.untilStopped },
          ]}
        />
        {photos.data && <Checkbox label={s.privacy} checked={privacy} onChange={setPrivacy} />}
        {action.error && <ErrorText>{action.error}</ErrorText>}
        {current.length > 0 && (
          <section className="screen-share-access" aria-label={s.existing}>
            <h3 className="screen-share-access__title">{s.existing}</h3>
            <ul className="screen-share-access__list">
              {current.map((share) => (
                <li key={share.id} className="screen-share-person">
                  <span className="screen-share-person__text">
                    <span className="screen-share-person__name">
                      {share.active ? (share.allowCopy ? s.allowCopy : s.queryOnly) : s.ended}
                    </span>
                    <span className="screen-share-person__meta">
                      {share.expiresAtMs != null ? s.endsOn(dateFromSeconds(share.expiresAtMs / 1000)) : s.noEnd}
                    </span>
                  </span>
                  {share.active && (
                    <Button
                      variant="quiet"
                      disabled={stop.busy}
                      onClick={async () => {
                        if (await stop.run(() => client.revokePublicShare({ id: share.id }))) {
                          toast(s.revoked);
                          shares.reload();
                        }
                      }}
                    >
                      {s.revoke}
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          </section>
        )}
        {stop.error && <ErrorText>{stop.error}</ErrorText>}
      </Dialog>
      {reach.prompt}
    </>
  );
}
