// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Sharing: what you share and with whom, and your contacts. New Share opens
// the same share dialog as a collection's Share button.

import { useEffect, useState } from "react";
import {
  Avatar,
  Button,
  ConfirmDialog,
  CopyButton,
  ErrorText,
  Group,
  GroupPanel,
  Menu,
  Row,
  TextField,
  Tile,
  collectionTones,
} from "@oikome/ui";
import type { Contact, NodeInfo, Share } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { dateFromSeconds, fullId, shortId } from "../format";
import { ScreenHeader } from "../parts";
import { navigate } from "../router";
import { strings } from "../strings";
import { ShareDialog } from "./ShareDialog";

const s = strings.sharing;

export function Sharing({
  info,
  startAdding = false,
}: {
  info: NodeInfo;
  /** Opened to add a contact (?add=1), from a share that needs one first. */
  startAdding?: boolean;
}) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const contacts = useLoad(() => client.listContacts(), []);
  const [composing, setComposing] = useState(false);
  const stop = useAction();

  const local = (spaces.data ?? []).filter((space) => space.kind !== "remote");
  const all = shares.data ?? [];
  const active = all.filter((share) => share.status === "active");
  const ended = all.filter((share) => share.status !== "active");
  const reload = () => {
    spaces.reload();
    shares.reload();
    contacts.reload();
  };
  // The same colors as on Collections: built from the same list of names.
  const toneOf = collectionTones((spaces.data ?? []).map((space) => space.name));

  return (
    <div className="screen-column">
      <ScreenHeader title={s.title}>
        <Button icon="plus" onClick={() => setComposing(true)}>
          {s.newShare}
        </Button>
      </ScreenHeader>

      {[spaces.error, shares.error, contacts.error, stop.error]
        .filter(Boolean)
        .slice(0, 1)
        .map((e) => (
          <ErrorText key="error">{e}</ErrorText>
        ))}

      <Group title={s.sharedByYou}>
        {active.length === 0 && <Row title={s.nothingShared} muted />}
        {active.map((share) => {
          const name = share.contactName ?? strings.common.unknown;
          return (
            <Row
              key={share.id}
              leading={<Tile icon="collection" tone={toneOf(share.space)} />}
              title={share.space}
              meta={s.shareSentence(name, share.access === "copy", dateFromSeconds(share.expiresAt))}
              trailing={
                <Button
                  variant="quiet"
                  disabled={stop.busy}
                  aria-label={`${s.stop}: ${share.space}, ${name}`}
                  onClick={async () => {
                    const done = await stop.run(() => client.revokeShare({ id: share.id }));
                    if (done) reload();
                  }}
                >
                  {s.stop}
                </Button>
              }
            />
          );
        })}
      </Group>

      {ended.length > 0 && (
        <Group title={s.ended}>
          {ended.map((share) => {
            const name = share.contactName ?? strings.common.unknown;
            return (
              <Row
                key={share.id}
                muted
                leading={<Tile icon="collection" tone="stone" />}
                title={share.space}
                meta={
                  share.status === "expired"
                    ? s.expiredFor(name, dateFromSeconds(share.expiresAt))
                    : s.stoppedFor(name)
                }
              />
            );
          })}
        </Group>
      )}

      <Contacts
        contacts={contacts.data ?? []}
        shares={active}
        me={info.did}
        startAdding={startAdding}
        onChange={reload}
      />

      <Group title={s.yourId} footnote={s.yourIdBody}>
        <Row
          title={
            <span className="oik-mono" translate="no">
              {shortId(info.did)}
            </span>
          }
          trailing={
            <CopyButton
              value={shortId(info.did)}
              copyLabel={strings.common.copy}
              copiedLabel={strings.common.copied}
            />
          }
        />
      </Group>

      {composing && spaces.data && contacts.data && shares.data && (
        <ShareDialog
          spaces={local}
          contacts={contacts.data}
          shares={shares.data}
          toneOf={toneOf}
          onClose={() => setComposing(false)}
          onChange={reload}
        />
      )}
    </div>
  );
}

function Contacts({
  contacts,
  shares,
  me,
  startAdding,
  onChange,
}: {
  contacts: Contact[];
  shares: Share[];
  /** This data center's own ID, which is never a contact. */
  me: string;
  startAdding: boolean;
  onChange: () => void;
}) {
  const client = useClient();
  const [adding, setAdding] = useState(startAdding);
  useEffect(() => {
    if (startAdding) setAdding(true);
  }, [startAdding]);
  const [name, setName] = useState("");
  const [did, setDid] = useState("");
  // A problem with one field, shown under that field.
  const [problem, setProblem] = useState<{ field: "name" | "id"; message: string }>();
  const [editing, setEditing] = useState<string>();
  const [newName, setNewName] = useState("");
  const [removing, setRemoving] = useState<Contact>();
  const action = useAction();
  const remove = useAction();

  const stopAdding = () => {
    setAdding(false);
    setName("");
    setDid("");
    setProblem(undefined);
    action.clear();
    // Drop ?add=1, so a reload does not open the form again.
    if (startAdding) navigate("/sharing", { replace: true });
  };

  return (
    <>
      <Group title={s.contacts} footnote={s.contactsFootnote}>
        {contacts.map((contact) => {
          const count = shares.filter((share) => share.did === contact.did).length;
          if (editing === contact.did) {
            return (
              <GroupPanel key={contact.did}>
                <form
                  className="screen-inline-form"
                  onSubmit={async (e) => {
                    e.preventDefault();
                    if (!newName.trim()) {
                      action.fail(strings.common.nameRequired);
                      return;
                    }
                    const done = await action.run(() =>
                      client.renameContact({ did: contact.did, name: newName.trim() }),
                    );
                    if (done) {
                      setEditing(undefined);
                      onChange();
                    }
                  }}
                >
                  <TextField
                    label={s.contactName}
                    name="contact-name"
                    value={newName}
                    onChange={(value) => {
                      setNewName(value);
                      action.clear();
                    }}
                    error={action.error}
                    autoFocus
                  />
                  <Button
                    onClick={() => {
                      setEditing(undefined);
                      action.clear();
                    }}
                  >
                    {strings.common.cancel}
                  </Button>
                  <Button type="submit" disabled={action.busy}>
                    {strings.common.save}
                  </Button>
                </form>
              </GroupPanel>
            );
          }
          return (
            <Row
              key={contact.did}
              leading={<Avatar name={contact.name} />}
              title={contact.name}
              meta={count > 0 ? s.sharesCount(count) : s.noShares}
              trailing={
                <Menu
                  label={strings.common.moreActions(contact.name)}
                  items={[
                    {
                      label: s.rename,
                      onSelect: () => {
                        setEditing(contact.did);
                        setNewName(contact.name);
                      },
                    },
                    { label: s.remove, destructive: true, onSelect: () => setRemoving(contact) },
                  ]}
                />
              }
            />
          );
        })}
        {adding ? (
          <GroupPanel>
            <form
              className="screen-form"
              onSubmit={async (e) => {
                e.preventDefault();
                const id = fullId(did);
                const known = contacts.find((c) => c.did === id);
                const field = !name.trim()
                  ? { field: "name" as const, message: strings.common.nameRequired }
                  : !did.trim()
                    ? { field: "id" as const, message: s.idRequired }
                    : id === me
                      ? { field: "id" as const, message: s.idIsYours }
                      : known
                        ? { field: "id" as const, message: s.idTaken(known.name) }
                        : undefined;
                setProblem(field);
                if (field) return;
                let invalid = false;
                const added = await action.run(
                  () => client.addContact({ did: id, name: name.trim() }),
                  (err) => {
                    invalid = err.code === "invalid";
                    return undefined;
                  },
                );
                if (added) {
                  stopAdding();
                  onChange();
                } else if (invalid) {
                  action.clear();
                  setProblem({ field: "id", message: s.idInvalid });
                }
              }}
            >
              <h3 className="screen-panel-title">{s.addContact}</h3>
              <TextField
                label={s.contactName}
                name="contact-name"
                value={name}
                onChange={(value) => {
                  setName(value);
                  setProblem(undefined);
                }}
                error={problem?.field === "name" ? problem.message : undefined}
                autoFocus
              />
              <TextField
                label={s.contactId}
                name="contact-id"
                hint={s.contactIdHint}
                value={did}
                onChange={(value) => {
                  setDid(value);
                  setProblem(undefined);
                }}
                error={problem?.field === "id" ? problem.message : undefined}
                mono
              />
              {action.error && <ErrorText>{action.error}</ErrorText>}
              <div className="screen-actions">
                <Button onClick={stopAdding}>{strings.common.cancel}</Button>
                <Button type="submit" variant="primary" disabled={action.busy}>
                  {s.add}
                </Button>
              </div>
            </form>
          </GroupPanel>
        ) : (
          <Row
            add
            leading={<Tile icon="plus" tone="mist" />}
            title={s.addContact}
            onClick={() => setAdding(true)}
          />
        )}
      </Group>
      {removing && (
        <ConfirmDialog
          title={s.removeTitle(removing.name)}
          body={s.removeBody}
          confirmLabel={s.remove}
          cancelLabel={strings.common.cancel}
          busy={remove.busy}
          error={remove.error}
          onConfirm={async () => {
            const done = await remove.run(() => client.removeContact({ did: removing.did }));
            if (done) {
              setRemoving(undefined);
              onChange();
            }
          }}
          onCancel={() => {
            setRemoving(undefined);
            remove.clear();
          }}
        />
      )}
    </>
  );
}
