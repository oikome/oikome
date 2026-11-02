// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Sharing: what you share and with whom, your contacts, and a new share.

import { useState } from "react";
import {
  Avatar,
  Button,
  Card,
  Checkbox,
  CopyField,
  ErrorText,
  Group,
  InsetWell,
  Row,
  SegmentedControl,
  Select,
  TextField,
  Tile,
  toneFor,
} from "@oikome/ui";
import type { Contact, NewShare, NodeInfo, Share, SpaceSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { dateFromSeconds } from "../format";
import { strings } from "../strings";

const s = strings.sharing;
type Ends = "7" | "30" | "365";

export function Sharing({ info, preselect }: { info: NodeInfo; preselect?: string }) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const contacts = useLoad(() => client.listContacts(), []);
  const [composing, setComposing] = useState(Boolean(preselect));
  const stop = useAction();

  const local = (spaces.data ?? []).filter((space) => space.kind !== "remote");
  const active = (shares.data ?? []).filter((share) => share.status === "active");
  const ended = (shares.data ?? []).filter((share) => share.status !== "active");
  const reload = () => {
    spaces.reload();
    shares.reload();
    contacts.reload();
  };

  return (
    <>
      <header className="screen-header">
        <h1 className="screen-title">{s.title}</h1>
        {!composing && (
          <Button icon="plus" onClick={() => setComposing(true)}>
            {s.newShare}
          </Button>
        )}
      </header>

      {composing && spaces.data && contacts.data && (
        <NewSharePanel
          spaces={local}
          contacts={contacts.data}
          preselect={preselect}
          onClose={() => {
            setComposing(false);
            reload();
          }}
        />
      )}

      {[spaces.error, shares.error, contacts.error, stop.error]
        .filter(Boolean)
        .slice(0, 1)
        .map((e) => (
          <ErrorText key="error">{e}</ErrorText>
        ))}

      <Group title={s.sharedByYou}>
        {active.length === 0 ? (
          <Row title={s.nothingShared} muted />
        ) : (
          active.map((share) => (
            <Row
              key={share.id}
              leading={<Tile icon="collection" tone={toneFor(share.space)} />}
              title={share.space}
              meta={`${shareWho(share)} · ${s.until(dateFromSeconds(share.expiresAt))}`}
              trailing={
                <Button
                  variant="quiet"
                  disabled={stop.busy}
                  aria-label={`${s.stop}: ${share.space}, ${share.contactName ?? ""}`}
                  onClick={async () => {
                    const done = await stop.run(() => client.revokeShare({ id: share.id }));
                    if (done) reload();
                  }}
                >
                  {s.stop}
                </Button>
              }
            />
          ))
        )}
      </Group>

      {ended.length > 0 && (
        <Group title={s.ended}>
          {ended.map((share) => (
            <Row
              key={share.id}
              leading={<Tile icon="collection" tone="stone" />}
              title={share.space}
              meta={`${share.contactName ?? strings.common.unknown} · ${
                share.status === "expired"
                  ? s.expiredOn(dateFromSeconds(share.expiresAt))
                  : s.stopped
              }`}
            />
          ))}
        </Group>
      )}

      <Contacts contacts={contacts.data ?? []} shares={active} onChange={reload} />

      <Card>
        <h2 className="oik-field__label">{s.yourId}</h2>
        <p className="screen-muted">{s.yourIdBody}</p>
        <CopyField value={info.did} copyLabel={strings.common.copy} copiedLabel={strings.common.copied} />
      </Card>
    </>
  );
}

function shareWho(share: Share): string {
  const name = share.contactName ?? strings.common.unknown;
  return share.access === "copy" ? s.canCopy(name) : s.canView(name);
}

function NewSharePanel({
  spaces,
  contacts,
  preselect,
  onClose,
}: {
  spaces: SpaceSummary[];
  contacts: Contact[];
  preselect?: string;
  onClose: () => void;
}) {
  const client = useClient();
  const [space, setSpace] = useState(
    spaces.find((x) => x.name === preselect)?.name ?? spaces[0]?.name ?? "",
  );
  const [did, setDid] = useState(contacts[0]?.did ?? "");
  const [keepCopy, setKeepCopy] = useState(false);
  const [ends, setEnds] = useState<Ends>("30");
  const [made, setMade] = useState<NewShare>();
  const share = useAction();
  const analytical = spaces.find((x) => x.name === space)?.kind === "analytical";
  const contactName = contacts.find((c) => c.did === did)?.name ?? "";

  if (made) {
    const name = contacts.find((c) => c.did === made.did)?.name ?? "";
    return (
      <Card className="screen-panel">
        <h2 className="screen-subtitle">{s.inviteTitle(name)}</h2>
        <InsetWell>
          <CopyField value={made.token} copyLabel={strings.common.copy} copiedLabel={strings.common.copied} />
        </InsetWell>
        <p className="screen-muted screen-result">{s.inviteBody(name)}</p>
        <div className="screen-actions">
          <Button variant="primary" onClick={onClose}>
            {strings.common.done}
          </Button>
        </div>
      </Card>
    );
  }

  return (
    <Card className="screen-panel">
      <h2 className="screen-subtitle">{s.newShare}</h2>
      {spaces.length === 0 ? (
        <p className="screen-muted">{s.needCollection}</p>
      ) : contacts.length === 0 ? (
        <p className="screen-muted">{s.needContact}</p>
      ) : (
        <form
          className="screen-form"
          onSubmit={async (e) => {
            e.preventDefault();
            const result = await share.run(() =>
              client.shareSpace({
                space,
                did,
                allowCopy: analytical && keepCopy,
                expiresInDays: Number(ends),
              }),
            );
            if (result) setMade(result);
          }}
        >
          <Select
            label={s.collectionLabel}
            value={space}
            onChange={setSpace}
            options={spaces.map((x) => ({ value: x.name, label: x.name }))}
          />
          <Select
            label={s.withLabel}
            value={did}
            onChange={setDid}
            options={contacts.map((c) => ({ value: c.did, label: c.name }))}
          />
          <div className="oik-field">
            <Checkbox
              label={s.keepCopy}
              checked={analytical && keepCopy}
              disabled={!analytical}
              onChange={setKeepCopy}
            />
            {!analytical && <span className="oik-field__hint">{s.keepCopyOnlyAnalytical}</span>}
          </div>
          <SegmentedControl<Ends>
            label={s.endsLabel}
            value={ends}
            onChange={setEnds}
            options={[
              { value: "7", label: s.ends7 },
              { value: "30", label: s.ends30 },
              { value: "365", label: s.endsYear },
            ]}
          />
          {share.error && <ErrorText>{share.error}</ErrorText>}
          <div className="screen-actions">
            <Button
              type="submit"
              variant="primary"
              disabled={share.busy || !space || !did}
              aria-label={`${s.share} ${space} with ${contactName}`}
            >
              {s.share}
            </Button>
            <Button variant="text" onClick={onClose}>
              {strings.common.cancel}
            </Button>
          </div>
        </form>
      )}
      {(spaces.length === 0 || contacts.length === 0) && (
        <div className="screen-actions">
          <Button variant="text" onClick={onClose}>
            {strings.common.cancel}
          </Button>
        </div>
      )}
    </Card>
  );
}

function Contacts({
  contacts,
  shares,
  onChange,
}: {
  contacts: Contact[];
  shares: Share[];
  onChange: () => void;
}) {
  const client = useClient();
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [did, setDid] = useState("");
  const [editing, setEditing] = useState<string>();
  const [newName, setNewName] = useState("");
  const [removing, setRemoving] = useState<string>();
  const action = useAction();

  return (
    <>
      <Group title={s.contacts} footnote={s.contactsFootnote}>
        {contacts.map((contact) => {
          const count = shares.filter((share) => share.did === contact.did).length;
          if (editing === contact.did) {
            return (
              <li key={contact.did} className="oik-row">
                <form
                  className="screen-inline-form"
                  onSubmit={async (e) => {
                    e.preventDefault();
                    const done = await action.run(() =>
                      client.renameContact({ did: contact.did, name: newName }),
                    );
                    if (done) {
                      setEditing(undefined);
                      onChange();
                    }
                  }}
                >
                  <TextField label={s.contactName} value={newName} onChange={setNewName} autoFocus />
                  <Button type="submit" disabled={action.busy}>
                    {strings.common.save}
                  </Button>
                  <Button variant="text" onClick={() => setEditing(undefined)}>
                    {strings.common.cancel}
                  </Button>
                </form>
              </li>
            );
          }
          return (
            <Row
              key={contact.did}
              leading={<Avatar name={contact.name} />}
              title={contact.name}
              meta={
                removing === contact.did
                  ? s.removeConfirm(contact.name)
                  : count > 0
                    ? s.sharesCount(count)
                    : s.noShares
              }
              trailing={
                removing === contact.did ? (
                  <>
                    <Button
                      variant="quiet"
                      disabled={action.busy}
                      onClick={async () => {
                        const done = await action.run(() =>
                          client.removeContact({ did: contact.did }),
                        );
                        if (done) {
                          setRemoving(undefined);
                          onChange();
                        }
                      }}
                    >
                      {s.remove}
                    </Button>
                    <Button variant="text" onClick={() => setRemoving(undefined)}>
                      {strings.common.cancel}
                    </Button>
                  </>
                ) : (
                  <>
                    <Button
                      variant="text"
                      onClick={() => {
                        setEditing(contact.did);
                        setNewName(contact.name);
                      }}
                    >
                      {s.rename}
                    </Button>
                    <Button variant="quiet" onClick={() => setRemoving(contact.did)}>
                      {s.remove}
                    </Button>
                  </>
                )
              }
            />
          );
        })}
        {!adding && (
          <Row
            add
            leading={<Tile icon="plus" tone="mist" />}
            title={s.addContact}
            onClick={() => setAdding(true)}
          />
        )}
      </Group>
      {action.error && <ErrorText>{action.error}</ErrorText>}
      {adding && (
        <Card className="screen-panel">
          <h2 className="screen-subtitle">{s.addContact}</h2>
          <form
            className="screen-form"
            onSubmit={async (e) => {
              e.preventDefault();
              const added = await action.run(() =>
                client.addContact({ did: did.trim(), name: name.trim() }),
              );
              if (added) {
                setAdding(false);
                setName("");
                setDid("");
                onChange();
              }
            }}
          >
            <TextField label={s.contactName} value={name} onChange={setName} autoFocus />
            <TextField label={s.contactId} hint={s.contactIdHint} value={did} onChange={setDid} mono />
            <div className="screen-actions">
              <Button type="submit" variant="primary" disabled={action.busy || !name.trim() || !did.trim()}>
                {s.add}
              </Button>
              <Button variant="text" onClick={() => setAdding(false)}>
                {strings.common.cancel}
              </Button>
            </div>
          </form>
        </Card>
      )}
    </>
  );
}
