// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Sharing: what you share and with whom, your contacts, and a new share in a
// panel beside them.

import { useState } from "react";
import {
  Avatar,
  Button,
  ChoiceGroup,
  CopyButton,
  CopyField,
  ErrorText,
  Group,
  GroupPanel,
  IconButton,
  InsetWell,
  Menu,
  Row,
  SegmentedControl,
  Select,
  TextField,
  Tile,
  toneFor,
} from "@oikome/ui";
import type { Contact, NewShare, NodeInfo, Share, SpaceSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { dateFromSeconds, fullId, shortId } from "../format";
import { navigate } from "../router";
import { strings } from "../strings";

const s = strings.sharing;
type Ends = "7" | "30" | "365";
type Access = "view" | "copy";

export function Sharing({ info, preselect }: { info: NodeInfo; preselect?: string }) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const contacts = useLoad(() => client.listContacts(), []);
  const [composing, setComposing] = useState(Boolean(preselect));
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

  return (
    <>
      <header className="screen-header">
        <h1 className="screen-title">{s.title}</h1>
        <Button icon="plus" aria-expanded={composing} onClick={() => setComposing(true)}>
          {s.newShare}
        </Button>
      </header>

      <div className={composing ? "screen-aside-layout screen-aside-layout--open" : "screen-aside-layout"}>
        <div>
          {[spaces.error, shares.error, contacts.error, stop.error]
            .filter(Boolean)
            .slice(0, 1)
            .map((e) => (
              <ErrorText key="error">{e}</ErrorText>
            ))}

          <Group title={s.sharedByYou}>
            {all.length === 0 && <Row title={s.nothingShared} muted />}
            {active.map((share) => (
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
            ))}
            {ended.map((share) => (
              <Row
                key={share.id}
                muted
                leading={<Tile icon="collection" tone="stone" />}
                title={share.space}
                meta={`${share.contactName ?? strings.common.unknown} · ${
                  share.status === "expired"
                    ? s.expiredOn(dateFromSeconds(share.expiresAt))
                    : s.stopped
                }`}
                trailing={<span className="screen-muted">{s.ended}</span>}
              />
            ))}
          </Group>

          <Contacts contacts={contacts.data ?? []} shares={active} me={info.did} onChange={reload} />

          <Group title={s.yourId} footnote={s.yourIdBody}>
            <Row
              title={<span className="oik-mono">{shortId(info.did)}</span>}
              trailing={
                <CopyButton
                  value={shortId(info.did)}
                  copyLabel={strings.common.copy}
                  copiedLabel={strings.common.copied}
                />
              }
            />
          </Group>
        </div>

        {composing && spaces.data && contacts.data && (
          <NewSharePanel
            spaces={local}
            contacts={contacts.data}
            preselect={preselect}
            onClose={() => {
              setComposing(false);
              reload();
              // Opened by Share… (?share=): a reload should not open it again.
              if (preselect) navigate("/sharing", { replace: true });
            }}
          />
        )}
      </div>
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
  const [access, setAccess] = useState<Access>("view");
  const [ends, setEnds] = useState<Ends>("30");
  const [made, setMade] = useState<NewShare>();
  const share = useAction();
  const copyable = spaces.find((x) => x.name === space)?.kind === "analytical";
  const contactName = contacts.find((c) => c.did === did)?.name ?? "";

  const head = (title: string) => (
    <div className="screen-aside__head">
      <h2 className="screen-aside__title">{title}</h2>
      <IconButton icon="close" label={strings.common.close} onClick={onClose} />
    </div>
  );

  if (made) {
    const name = contacts.find((c) => c.did === made.did)?.name ?? "";
    return (
      <aside className="screen-aside" aria-label={s.inviteTitle(name)}>
        {head(s.inviteTitle(name))}
        <InsetWell>
          <CopyField value={made.token} copyLabel={strings.common.copy} copiedLabel={strings.common.copied} />
        </InsetWell>
        <p className="screen-muted">{s.inviteBody(name)}</p>
        <div className="screen-aside__actions">
          <Button variant="primary" onClick={onClose}>
            {strings.common.done}
          </Button>
        </div>
      </aside>
    );
  }

  return (
    <aside className="screen-aside" aria-label={s.newShare}>
      {head(s.newShare)}
      {spaces.length === 0 ? (
        <p className="screen-muted">{s.needCollection}</p>
      ) : contacts.length === 0 ? (
        <p className="screen-muted">{s.needContact}</p>
      ) : (
        <form
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
            if (result) setMade(result);
          }}
        >
          <Select
            label={s.collectionLabel}
            value={space}
            onChange={(value) => {
              setSpace(value);
              if (spaces.find((x) => x.name === value)?.kind !== "analytical") setAccess("view");
            }}
            leading={<Tile icon="collection" tone={toneFor(space)} small />}
            options={spaces.map((x) => ({ value: x.name, label: x.name }))}
          />
          <Select
            label={s.withLabel}
            value={did}
            onChange={setDid}
            leading={<Avatar name={contactName || "?"} small />}
            options={contacts.map((c) => ({ value: c.did, label: c.name }))}
          />
          <ChoiceGroup<Access>
            label={s.howLabel}
            value={copyable ? access : "view"}
            onChange={setAccess}
            choices={[
              { value: "view", label: s.viewLive, description: s.viewLiveBody },
              {
                value: "copy",
                label: s.keepCopy,
                description: copyable ? s.keepCopyBody : s.keepCopyUnavailable,
                disabled: !copyable,
              },
            ]}
          />
          <SegmentedControl<Ends>
            label={s.endsLabel}
            value={ends}
            onChange={setEnds}
            fill
            options={[
              { value: "7", label: s.ends7 },
              { value: "30", label: s.ends30 },
              { value: "365", label: s.endsYear },
            ]}
          />
          {share.error && <ErrorText>{share.error}</ErrorText>}
          <div className="screen-aside__actions">
            <Button onClick={onClose}>{strings.common.cancel}</Button>
            <Button
              type="submit"
              variant="primary"
              disabled={share.busy || !space || !did}
              aria-label={`${s.share} ${space} with ${contactName}`}
            >
              {s.share}
            </Button>
          </div>
        </form>
      )}
    </aside>
  );
}

function Contacts({
  contacts,
  shares,
  me,
  onChange,
}: {
  contacts: Contact[];
  shares: Share[];
  /** This data center's own ID, which is never a contact. */
  me: string;
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
                <Button type="submit" variant="primary" disabled={action.busy || !newName.trim()}>
                  {strings.common.save}
                </Button>
                <Button onClick={() => setEditing(undefined)}>{strings.common.cancel}</Button>
              </form>
              {action.error && <ErrorText>{action.error}</ErrorText>}
            </GroupPanel>
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
                    variant="destructive"
                    disabled={action.busy}
                    onClick={async () => {
                      const done = await action.run(() => client.removeContact({ did: contact.did }));
                      if (done) {
                        setRemoving(undefined);
                        onChange();
                      }
                    }}
                  >
                    {s.remove}
                  </Button>
                  <Button variant="quiet" onClick={() => setRemoving(undefined)}>
                    {strings.common.cancel}
                  </Button>
                </>
              ) : (
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
                    { label: s.remove, destructive: true, onSelect: () => setRemoving(contact.did) },
                  ]}
                />
              )
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
              if (id === me) {
                action.fail(s.idIsYours);
                return;
              }
              if (known) {
                action.fail(s.idTaken(known.name));
                return;
              }
              const added = await action.run(
                () => client.addContact({ did: id, name: name.trim() }),
                (err) => (err.code === "invalid" ? s.idInvalid : undefined),
              );
              if (added) {
                setAdding(false);
                setName("");
                setDid("");
                onChange();
              }
            }}
          >
            <h3 className="screen-panel-title">{s.addContact}</h3>
            <TextField label={s.contactName} value={name} onChange={setName} autoFocus />
            <TextField label={s.contactId} hint={s.contactIdHint} value={did} onChange={setDid} mono />
            {action.error && <ErrorText>{action.error}</ErrorText>}
            <div className="screen-actions">
              <Button type="submit" variant="primary" disabled={action.busy || !name.trim() || !did.trim()}>
                {s.add}
              </Button>
              <Button
                onClick={() => {
                  setAdding(false);
                  action.clear();
                }}
              >
                {strings.common.cancel}
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
  );
}
