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
  ConfirmDialog,
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
  collectionTones,
} from "@oikome/ui";
import type { TileTone } from "@oikome/ui";
import type { Contact, NewShare, NodeInfo, Share, SpaceSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { dateFromSeconds, fullId, shortId } from "../format";
import { ScreenHeader } from "../parts";
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
  // The same colors as on Collections: built from the same list of names.
  const toneOf = collectionTones((spaces.data ?? []).map((space) => space.name));

  return (
    <div className={composing ? undefined : "screen-column"}>
      <ScreenHeader title={s.title}>
        <Button icon="plus" aria-expanded={composing} onClick={() => setComposing(true)}>
          {s.newShare}
        </Button>
      </ScreenHeader>

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
                leading={<Tile icon="collection" tone={toneOf(share.space)} />}
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
            toneOf={toneOf}
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
    </div>
  );
}

function shareWho(share: Share): string {
  const name = share.contactName ?? strings.common.unknown;
  return share.access === "copy" ? s.canCopy(name) : s.canView(name);
}

function NewSharePanel({
  spaces,
  toneOf,
  contacts,
  preselect,
  onClose,
}: {
  spaces: SpaceSummary[];
  toneOf: (name: string) => TileTone;
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
      <IconButton icon="close" label={strings.common.close} filled onClick={onClose} />
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
            leading={<Tile icon="collection" tone={toneOf(space)} small />}
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
