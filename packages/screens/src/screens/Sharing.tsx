// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Sharing: what you share and with whom, and your contacts. New Share opens
// the same share dialog as a collection's Share button. An invite someone
// sent is added here (Add an Invite) and waits at the top, as the
// prototype's notice, until it is accepted or declined. Compute lends this
// computer to contacts, and lists the computers lent to you.

import { useEffect, useId, useState } from "react";
import {
  Avatar,
  Button,
  Checkbox,
  ChoiceList,
  ConfirmDialog,
  CopyButton,
  CopyField,
  Dialog,
  ErrorText,
  Group,
  GroupPanel,
  Icon,
  InsetWell,
  Menu,
  Row,
  Segmented,
  SkeletonRows,
  Status,
  Switch,
  TextArea,
  TextField,
  Tile,
  collectionTones,
  useToast,
} from "@oikome/ui";
import type { Borrowed, Contact, Invite, LendInvite, Lending, NodeInfo, Share, SpaceSummary } from "@oikome/client";
import type { StatusKind } from "@oikome/ui";
import type { Describe } from "../data";
import { asSentence, useAction, useClient, useLoad } from "../data";
import { toastReach, useReachPrompt } from "../reach";
import type { ReachFor } from "../reach";
import { ago, dateFromSeconds, fullId, shortId } from "../format";
import { ScreenHeader } from "../parts";
import { navigate } from "../router";
import { strings } from "../strings";
import { ShareDialog } from "./ShareDialog";

const s = strings.sharing;

export function Sharing({
  info,
  startAdding = false,
  startSharing = false,
  startInviting = false,
  startLending = false,
}: {
  info: NodeInfo;
  /** Opened to add a contact (?add=1), from a share that needs one first. */
  startAdding?: boolean;
  /** Opened to share something (?share=1), from the ⌘K palette. */
  startSharing?: boolean;
  /** Opened to add an invite someone sent (?invite=1), from the palette. */
  startInviting?: boolean;
  /** Opened to lend this computer (?lend=1), from the palette. */
  startLending?: boolean;
}) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const contacts = useLoad(() => client.listContacts(), []);
  const invites = useLoad(() => client.listInvites(), []);
  const lending = useLoad(() => client.lending(), []);
  const borrowed = useLoad(() => client.listBorrowed(), []);
  const [composing, setComposing] = useState(startSharing);
  // Add Contact, open on one of its two ways: an invite they sent, or their ID.
  const [addingContact, setAddingContact] = useState<"code" | "id" | undefined>(
    startInviting ? "code" : startAdding ? "id" : undefined,
  );
  const [lendingOpen, setLendingOpen] = useState(startLending);
  const [borrowing, setBorrowing] = useState<Borrowed>();
  useEffect(() => {
    if (startSharing || startInviting || startAdding || startLending) {
      setComposing(startSharing);
      setAddingContact(startInviting ? "code" : startAdding ? "id" : undefined);
      setLendingOpen(startLending);
      // Drop the ?share=1 kind of query, so a reload does not open the dialog again.
      navigate("/sharing", { replace: true });
    }
  }, [startSharing, startInviting, startAdding, startLending]);
  const stop = useAction();
  const toast = useToast();

  const local = (spaces.data ?? []).filter((space) => space.kind !== "remote");
  const all = shares.data ?? [];
  const active = all.filter((share) => share.status === "active");
  const ended = all.filter((share) => share.status !== "active");
  const reload = () => {
    spaces.reload();
    shares.reload();
    contacts.reload();
    invites.reload();
    lending.reload();
    borrowed.reload();
  };
  // The same colors as on Collections: built from the same list of names.
  const toneOf = collectionTones((spaces.data ?? []).map((space) => space.name));

  return (
    <>
      <ScreenHeader title={s.title} lede={s.lede}>
        <Button icon="person-plus" onClick={() => setAddingContact("id")}>
          {s.addContact}
        </Button>
        <Button variant="primary" icon="plus" onClick={() => setComposing(true)}>
          {s.newShare}
        </Button>
      </ScreenHeader>

      {(invites.data ?? []).length > 0 && (
        <div className="screen-invites" role="region" aria-label={s.invitesLabel}>
          {invites.data!.map((invite) => (
            <InviteNotice key={invite.id} invite={invite} onDone={reload} />
          ))}
        </div>
      )}

      {[spaces.error, shares.error, contacts.error, stop.error]
        .filter(Boolean)
        .slice(0, 1)
        .map((e) => (
          <ErrorText key="error">{e}</ErrorText>
        ))}

      {/* Cards in rows, as in the prototype: each row's two share their edges. */}
      <div className="screen-dash screen-dash--split screen-dash--rows">
        <section className="oik-group" aria-labelledby="shared-by-you">
          <header className="oik-group__head">
            <h2 id="shared-by-you" className="oik-group__title">
              {s.sharedByYou}
            </h2>
            {shares.data && (
              <span className="screen-group-count">{s.counts(active.length, ended.length)}</span>
            )}
          </header>
          {!shares.data && !shares.error ? (
            <ul className="oik-group__body">
              <SkeletonRows count={2} />
            </ul>
          ) : active.length + ended.length === 0 ? (
            <p className="screen-panel__empty">{s.nothingShared}</p>
          ) : (
            <div className="screen-table-in-card">
              <table className="oik-table screen-shares" aria-label={s.sharedByYou}>
                <thead>
                  <tr>
                    <th scope="col">{s.colCollection}</th>
                    <th scope="col">{s.colWith}</th>
                    <th scope="col">{s.colAccess}</th>
                    <th scope="col">{s.colUntil}</th>
                    <th scope="col">
                      <span className="oik-visually-hidden">{s.stop}</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {[...active, ...ended].map((share) => {
                    const name = share.contactName ?? strings.common.unknown;
                    const on = share.status === "active";
                    return (
                      <tr key={share.id} className={on ? undefined : "screen-row-ended"}>
                        <td className="screen-cell-mono" translate="no">
                          {share.space}
                        </td>
                        <td>
                          <span className="screen-cell-person">
                            <Avatar name={name} small />
                            {name}
                          </span>
                          {on && !share.reachOutside && (
                            <span className="screen-cell-sub">{strings.reach.onlyOnNetwork(name)}</span>
                          )}
                        </td>
                        <td>
                          {on ? (
                            <Status kind={share.access === "copy" ? "run" : "ok"}>
                              {share.access === "copy" ? s.accessCopyShort : s.accessViewShort}
                            </Status>
                          ) : (
                            <Status kind="off">
                              {share.status === "expired" ? s.expired : s.endedShort}
                            </Status>
                          )}
                        </td>
                        <td className="screen-cell-mono">{dateFromSeconds(share.expiresAt)}</td>
                        <td className="screen-cell-action">
                          {on && (
                            <Button
                              variant="quiet"
                              disabled={stop.busy}
                              aria-label={`${s.stop}: ${share.space}, ${name}`}
                              onClick={async () => {
                                const done = await stop.run(() => client.revokeShare({ id: share.id }));
                                if (done) {
                                  toast(strings.toast.stopped(share.space, name));
                                  reload();
                                }
                              }}
                            >
                              {s.stopShort}
                            </Button>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </section>

        <Contacts
          contacts={contacts.data ?? []}
          fromThem={(spaces.data ?? []).filter((space) => space.kind === "remote")}
          lenders={(borrowed.data ?? []).map((b) => b.fromDid)}
          shares={active}
          onChange={reload}
        />

        <Compute
          lending={lending.data}
          borrowed={borrowed.data}
          onLend={() => setLendingOpen(true)}
          onOpen={setBorrowing}
          onChange={reload}
        />

      </div>

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
      {lendingOpen && lending.data && contacts.data && (
        <LendDialog
          lending={lending.data}
          contacts={contacts.data}
          onClose={() => setLendingOpen(false)}
          onSaved={reload}
        />
      )}
      {borrowing && (
        <BorrowDialog
          borrowed={borrowing}
          onClose={() => setBorrowing(undefined)}
          onChange={reload}
          onShare={() => {
            setBorrowing(undefined);
            setComposing(true);
          }}
        />
      )}
      {addingContact && (
        <AddContact
          start={addingContact}
          me={info.did}
          contacts={contacts.data ?? []}
          onClose={() => setAddingContact(undefined)}
          onAdded={() => {
            setAddingContact(undefined);
            reload();
          }}
        />
      )}
    </>
  );
}

/** An invite waiting here: who sent it and what for; accept or decline. */
function InviteNotice({ invite, onDone }: { invite: Invite; onDone: () => void }) {
  const client = useClient();
  const toast = useToast();
  const act = useAction();
  const titleId = useId();
  const from = invite.fromName;
  const computer = invite.kind === "compute";
  const meta = invite.ended
    ? s.inviteEndedBody
    : (computer
        ? [
            invite.concurrent != null ? s.tasksAtOnce(invite.concurrent) : null,
            invite.perHour != null ? s.perHour(invite.perHour) : null,
            s.inviteUntil(dateFromSeconds(invite.expiresAt)),
          ]
        : [
            strings.collections.viewLive,
            s.inviteUntil(dateFromSeconds(invite.expiresAt)),
            invite.renews ? s.inviteRenews(invite.name) : invite.name !== invite.space ? s.inviteAs(invite.name) : null,
          ]
      )
        .filter(Boolean)
        .join(" · ");
  const what = computer ? s.borrowedTitle(from) : invite.space;
  // Words for a failed accept: the node's own when they are about the
  // invite, else what to check.
  const describe: Describe = (e) =>
    e.code === "not_found"
      ? s.inviteGone
      : e.code === "invalid" && /invite/.test(e.message)
        ? asSentence(e)
        : s.acceptFailed(from);
  return (
    <section className="oik-card screen-callout screen-callout--quiet" aria-labelledby={titleId}>
      <Tile icon={computer ? "cpu" : "person-plus"} tone="lavender" />
      <div className="screen-callout__text">
        <h2 id={titleId} className="screen-callout__title">
          {invite.ended
            ? computer
              ? s.lendEnded(from)
              : s.inviteEnded(from, invite.space)
            : computer
              ? s.lendsYou(from)
              : s.invitedYou(from, invite.space)}
        </h2>
        <p className="screen-callout__meta">{meta}</p>
        {act.error && <ErrorText>{act.error}</ErrorText>}
      </div>
      <Button
        variant="quiet"
        disabled={act.busy}
        aria-label={invite.ended ? undefined : s.declineFor(what)}
        onClick={async () => {
          const left = await act.run(() => client.declineInvite({ id: invite.id }));
          if (left) {
            if (!invite.ended) toast(s.declined(from));
            onDone();
          }
        }}
      >
        {invite.ended ? s.dismiss : s.decline}
      </Button>
      {!invite.ended && (
        <Button
          disabled={act.busy}
          aria-label={s.acceptFor(what)}
          onClick={async () => {
            const added = await act.run(() => client.acceptInvite({ id: invite.id }), describe);
            if (added) {
              toast(added.kind === "compute" ? s.acceptedCompute(added.name) : s.accepted(added.name));
              onDone();
            }
          }}
        >
          {s.accept}
        </Button>
      )}
    </section>
  );
}

const BORROW_STATUS: Record<Borrowed["status"], StatusKind> = {
  available: "ok",
  unavailable: "off",
  unreachable: "idle",
  ended: "off",
};

/** Lend this computer (a switch, and Change for whom and how much); computers lent to you. */
function Compute({
  lending,
  borrowed,
  onLend,
  onOpen,
  onChange,
}: {
  lending?: Lending;
  borrowed?: Borrowed[];
  onLend: () => void;
  /** Open a computer lent to you: what went there, and whether to use it. */
  onOpen: (borrowed: Borrowed) => void;
  onChange: () => void;
}) {
  const client = useClient();
  const toast = useToast();
  const act = useAction();
  const reach = useReachPrompt();
  const names = (lending?.people ?? []).map((p) => p.name ?? strings.common.unknown);
  const meta = !lending
    ? undefined
    : names.length === 0
      ? s.notLent
      : [lending.on ? null : s.paused, s.lentTo(names), s.tasksAtOnce(lending.concurrent), s.perHour(lending.perHour)]
          .filter(Boolean)
          .join(" · ");
  return (
    <>
    {reach.prompt}
    <Group title={s.compute} footnote={s.computeFootnote}>
      <Row
        leading={<Tile icon="cpu" tone="sage" />}
        title={s.lendRow}
        meta={meta}
        trailing={
          <span className="screen-compute__acts">
            <Button variant="quiet" onClick={onLend}>
              {s.change}
            </Button>
            <Switch
              label={s.lendSwitch}
              checked={lending?.on ?? false}
              disabled={!lending || act.busy}
              onChange={async (on) => {
                if (!lending) return;
                // Nobody to lend to yet: choose first.
                if (on && lending.people.length === 0) {
                  onLend();
                  return;
                }
                // Lent again: the people lent to reach it from outside only if allowed.
                const what: ReachFor = { kind: "lend", names };
                const choice = on ? await reach.ask(what) : undefined;
                if (choice === null) return;
                const saved = await act.run(() =>
                  client.setLending({
                    on,
                    people: lending.people.map((p) => p.did),
                    concurrent: lending.concurrent,
                    perHour: lending.perHour,
                    reach: choice,
                  }),
                );
                if (saved) {
                  if (choice) void toastReach(client, toast, what, choice);
                  else toast(on ? s.lendOn : s.lendOff);
                  onChange();
                }
              }}
            />
          </span>
        }
      />
      {(borrowed ?? []).map((b) => (
        <Row
          key={b.id}
          leading={<Tile icon="cpu" tone="lavender" />}
          title={s.borrowedTitle(b.fromName)}
          meta={[
            b.inUse ? s.inUse : null,
            s.lendsYouMeta(
              [
                b.concurrent != null ? s.tasksAtOnce(b.concurrent) : null,
                b.perHour != null ? s.perHour(b.perHour) : null,
                s.inviteUntil(dateFromSeconds(b.expiresAt)),
              ].filter((part): part is string => part !== null),
            ),
          ]
            .filter(Boolean)
            .join(" · ")}
          trailing={
            <span className="screen-compute__acts">
              <Status kind={BORROW_STATUS[b.status]}>{s.borrowStatus[b.status]}</Status>
              <Button variant="quiet" onClick={() => onOpen(b)}>
                {s.borrowDetails}
              </Button>
              <Menu
                label={strings.common.moreActions(s.borrowedTitle(b.fromName))}
                items={[
                  {
                    label: s.stopUsing,
                    destructive: true,
                    onSelect: async () => {
                      const left = await act.run(() => client.stopBorrowing({ id: b.id }));
                      if (left) {
                        toast(s.stoppedUsing(b.fromName));
                        onChange();
                      }
                    },
                  },
                ]}
              />
            </span>
          }
        />
      ))}
      {act.error && (
        <GroupPanel>
          <ErrorText>{act.error}</ErrorText>
        </GroupPanel>
      )}
    </Group>
    </>
  );
}

/** The start of today, where the viewer is. */
function startOfToday(): number {
  const day = new Date();
  day.setHours(0, 0, 0, 0);
  return day.getTime();
}

/**
 * A computer lent to you: what went there today and lately, which
 * collections may, and whether to use it. Only big summaries of collections
 * shared with its owner go, and only what they can already see.
 */
function BorrowDialog({
  borrowed,
  onClose,
  onChange,
  onShare,
}: {
  borrowed: Borrowed;
  onClose: () => void;
  onChange: () => void;
  onShare: () => void;
}) {
  const client = useClient();
  const toast = useToast();
  const act = useAction();
  const name = borrowed.fromName;
  const [since] = useState(startOfToday);
  const jobs = useLoad(() => client.borrowedJobs({ id: borrowed.id, sinceMs: since }), [borrowed.id, since]);
  const [inUse, setInUse] = useState(borrowed.inUse);
  const collections = jobs.data?.collections ?? [];
  return (
    <Dialog
      title={s.borrowedTitle(name)}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          {!inUse && <span className="screen-borrow__off">{s.bigJobsStay}</span>}
          <Button variant="primary" onClick={onClose}>
            {strings.common.done}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{s.borrowBody(name)}</p>
      <div className="screen-borrow__today">
        <span className="oik-label">{s.usedToday}</span>
        <span>{jobs.data ? s.tasks(jobs.data.since) : "—"}</span>
      </div>
      {jobs.data && jobs.data.recent.length > 0 && (
        <Group>
          {jobs.data.recent.map((job) => (
            <Row
              key={`${job.space}-${job.atMs}`}
              leading={<Tile icon="cpu" tone="lavender" />}
              title={s.jobTitle(job.space)}
              meta={[s.jobFrom(job.space), job.count > 1 ? s.tasks(job.count) : null].filter(Boolean).join(" · ")}
              trailing={<span className="screen-borrow__when">{ago(job.atMs)}</span>}
            />
          ))}
        </Group>
      )}
      {jobs.data && (
        <p className="screen-borrow__which">
          {collections.length > 0 ? (
            s.canWorkOut(collections)
          ) : (
            <>
              {s.nothingSharedWith(name)}{" "}
              <Button variant="text" onClick={onShare}>
                {s.shareOne}
              </Button>
            </>
          )}
        </p>
      )}
      <div className="screen-borrow__use">
        <span>{s.useSwitch(name)}</span>
        <Switch
          label={s.useSwitch(name)}
          checked={inUse}
          disabled={act.busy}
          onChange={async (on) => {
            const listed = await act.run(() => client.useBorrowed({ id: borrowed.id, inUse: on }));
            if (!listed) return;
            setInUse(on);
            toast(on ? s.usingNow(name) : s.notUsing);
            onChange();
          }}
        />
      </div>
      {(act.error || jobs.error) && <ErrorText>{act.error ?? jobs.error}</ErrorText>}
    </Dialog>
  );
}

const AT_ONCE = ["1", "2", "4"];
const EACH_HOUR = ["10", "30", "100"];

/** Whom to lend this computer to, and how much each may use; then the codes to send. */
function LendDialog({
  lending,
  contacts,
  onClose,
  onSaved,
}: {
  lending: Lending;
  contacts: Contact[];
  onClose: () => void;
  onSaved: () => void;
}) {
  const client = useClient();
  const toast = useToast();
  const save = useAction();
  const reach = useReachPrompt();
  const [people, setPeople] = useState(() => new Set(lending.people.map((p) => p.did)));
  const [atOnce, setAtOnce] = useState(String(lending.concurrent));
  const [eachHour, setEachHour] = useState(String(lending.perHour));
  const [codes, setCodes] = useState<LendInvite[]>();
  // Values set elsewhere (the command line) are offered beside the usual ones.
  const atOnceChoices = [...new Set([...AT_ONCE, atOnce])].sort((a, b) => Number(a) - Number(b));
  const eachHourChoices = [...new Set([...EACH_HOUR, eachHour])].sort((a, b) => Number(a) - Number(b));

  if (codes) {
    return (
      <Dialog
        title={s.lendCodesTitle(codes.map((c) => c.name))}
        closeLabel={strings.common.close}
        onClose={onClose}
        footer={
          <Button variant="primary" onClick={onClose}>
            {strings.common.done}
          </Button>
        }
      >
        {codes.map((code) => (
          <div key={code.did} className="screen-lend-code">
            {codes.length > 1 && <h3 className="oik-label">{code.name}</h3>}
            <InsetWell>
              <CopyField value={code.invite} copyLabel={strings.common.copy} copiedLabel={strings.common.copied} />
            </InsetWell>
          </div>
        ))}
        <p className="screen-help">{s.lendCodesBody}</p>
      </Dialog>
    );
  }

  return (
    <>
    {reach.prompt}
    <Dialog
      title={s.lendTitle}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button
            variant="primary"
            disabled={save.busy || contacts.length === 0}
            onClick={async () => {
              // Asked when someone new is lent to, or lending starts again:
              // they reach this computer from outside only if allowed.
              const added = [...people].filter((d) => !(lending.on && lending.people.some((p) => p.did === d)));
              const what: ReachFor = {
                kind: "lend",
                names: added.map((d) => contacts.find((c) => c.did === d)?.name ?? strings.common.unknown),
              };
              const choice = added.length > 0 ? await reach.ask(what) : undefined;
              if (choice === null) return;
              const saved = await save.run(() =>
                client.setLending({
                  on: people.size > 0,
                  people: [...people],
                  concurrent: Number(atOnce),
                  perHour: Number(eachHour),
                  reach: choice,
                }),
              );
              if (!saved) return;
              onSaved();
              if (choice) void toastReach(client, toast, what, choice);
              if (saved.invites.length > 0) {
                setCodes(saved.invites);
              } else {
                if (!choice) toast(s.lendSaved);
                onClose();
              }
            }}
          >
            {strings.common.save}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">{s.lendBody}</p>
      <fieldset className="screen-lend-field">
        <legend className="oik-field__label">{s.lendTo}</legend>
        {contacts.length === 0 ? (
          <p className="screen-help">
            {s.lendNeedsContact}{" "}
            <Button
              variant="text"
              onClick={() => {
                onClose();
                navigate("/sharing?add=1");
              }}
            >
              {s.addContact}
            </Button>
          </p>
        ) : (
          <ul className="screen-lend-people">
            {contacts.map((contact) => (
              <li key={contact.did}>
                <Checkbox
                  label={
                    <span className="screen-cell-person">
                      <Avatar name={contact.name} small />
                      {contact.name}
                    </span>
                  }
                  checked={people.has(contact.did)}
                  onChange={(on) => {
                    const next = new Set(people);
                    if (on) next.add(contact.did);
                    else next.delete(contact.did);
                    setPeople(next);
                  }}
                />
              </li>
            ))}
          </ul>
        )}
      </fieldset>
      <div className="screen-lend-field">
        <span className="oik-field__label">{s.lendAtOnce}</span>
        <Segmented
          label={s.lendAtOnce}
          value={atOnce}
          onChange={setAtOnce}
          options={atOnceChoices.map((v) => ({ value: v, label: s.tasks(Number(v)) }))}
        />
      </div>
      <div className="screen-lend-field">
        <span className="oik-field__label">{s.lendEachHour}</span>
        <Segmented
          label={s.lendEachHour}
          value={eachHour}
          onChange={setEachHour}
          options={eachHourChoices.map((v) => ({ value: v, label: s.perHour(Number(v)) }))}
        />
      </div>
      <p className="screen-help">{s.lendHint(Math.round(lending.taskMs / 1000))}</p>
      {save.error && <ErrorText>{save.error}</ErrorText>}
    </Dialog>
    </>
  );
}

/** Add a contact, the one way in: an invite they sent (which names them
 * and what they share), or their ID; with this data center's own ID to
 * copy, for them to add you. */
function AddContact({
  start,
  me,
  contacts,
  onClose,
  onAdded,
}: {
  start: "code" | "id";
  /** This data center's own ID, which is never a contact. */
  me: string;
  contacts: Contact[];
  onClose: () => void;
  onAdded: () => void;
}) {
  const client = useClient();
  const toast = useToast();
  const [how, setHow] = useState(start);
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [did, setDid] = useState("");
  // A problem with one field, shown under that field.
  const [problem, setProblem] = useState<{ field: "code" | "name" | "id"; message: string }>();
  const action = useAction();
  // The node's words name what is wrong with a code; other failures get
  // the general wording.
  const describeCode: Describe = (e) => (e.code === "invalid" || e.code === "conflict" ? asSentence(e) : undefined);
  const edited = () => {
    setProblem(undefined);
    action.clear();
  };
  // The dialog opens on its first control; the field to fill is focused
  // after it, and again when the way changes (the dialog's effect runs first).
  useEffect(() => {
    document
      .querySelector<HTMLElement>(how === "code" ? '#add-contact [name="invite-code"]' : '#add-contact [name="contact-name"]')
      ?.focus();
  }, [how]);

  const submit = async () => {
    if (how === "code") {
      if (!code.trim()) {
        setProblem({ field: "code", message: s.inviteCodeRequired });
        return;
      }
      if (await action.run(() => client.addInvite({ invite: code }), describeCode)) onAdded();
      return;
    }
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
      toast(strings.toast.contactAdded(name.trim()));
      onAdded();
    } else if (invalid) {
      action.clear();
      setProblem({ field: "id", message: s.idInvalid });
    }
  };

  return (
    <Dialog
      title={s.addContact}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.cancel}</Button>
          <Button type="submit" form="add-contact" variant="primary" disabled={action.busy}>
            {how === "code" ? s.addInviteYes : s.add}
          </Button>
        </>
      }
    >
      <form
        id="add-contact"
        className="screen-form"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <ChoiceList
          label={s.addHow}
          name="add-contact-how"
          value={how}
          onChange={(value) => {
            setHow(value === "code" ? "code" : "id");
            edited();
          }}
          choices={[
            { value: "id", title: s.byId, meta: s.byIdHelp, leading: <Icon name="person-plus" /> },
            { value: "code", title: s.byInvite, meta: s.byInviteHelp, leading: <Icon name="key" /> },
          ]}
        />
        {how === "code" ? (
          <TextArea
            label={s.inviteCode}
            name="invite-code"
            hint={s.inviteCodeHint}
            value={code}
            onChange={(value) => {
              setCode(value);
              edited();
            }}
            error={problem?.field === "code" ? problem.message : (action.error ?? undefined)}
            mono
            autoFocus
          />
        ) : (
          <>
            <TextField
              label={s.contactName}
              name="contact-name"
              value={name}
              onChange={(value) => {
                setName(value);
                edited();
              }}
              error={problem?.field === "name" ? problem.message : undefined}
              autoFocus
            />
            <TextField
              label={s.contactId}
              name="contact-id"
              value={did}
              onChange={(value) => {
                setDid(value);
                edited();
              }}
              error={problem?.field === "id" ? problem.message : undefined}
              mono
            />
            {action.error && <ErrorText>{action.error}</ErrorText>}
          </>
        )}
      </form>
      <InsetWell>
        <div className="screen-your-id">
          <span className="screen-your-id__text">
            <span className="oik-label">{s.yourId}</span>
            <span className="oik-mono" translate="no">
              {shortId(me)}
            </span>
            <span className="screen-help">{s.yourIdBody}</span>
          </span>
          <CopyButton value={shortId(me)} copyLabel={strings.common.copy} copiedLabel={strings.common.copied} />
        </div>
      </InsetWell>
    </Dialog>
  );
}

function Contacts({
  contacts,
  fromThem,
  lenders,
  shares,
  onChange,
}: {
  contacts: Contact[];
  /** Collections shared with you, each from a data center. */
  fromThem: SpaceSummary[];
  /** Data centers that lend you their computer. */
  lenders: string[];
  shares: Share[];
  onChange: () => void;
}) {
  const client = useClient();
  const [editing, setEditing] = useState<string>();
  const [newName, setNewName] = useState("");
  const [removing, setRemoving] = useState<Contact>();
  const action = useAction();
  const remove = useAction();
  const toast = useToast();

  return (
    <>
      <Group
        title={s.contacts}
        footnote={s.contactsFootnote}
      >
        {contacts.length === 0 && <Row title={s.noContacts} muted />}
        {contacts.map((contact) => {
          const count = shares.filter((share) => share.did === contact.did).length;
          const theirs = fromThem.filter((space) => space.fromDid === contact.did).map((space) => space.name);
          const lends = lenders.includes(contact.did);
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
                      toast(strings.toast.contactRenamed(newName.trim()));
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
              meta={[
                count > 0 ? s.sharesCount(count) : theirs.length === 0 && !lends ? s.noShares : null,
                theirs.length > 0 ? s.sharesWithYou(theirs) : null,
                lends ? s.lendsYouShort : null,
              ]
                .filter(Boolean)
                .join(" · ")}
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
              toast(strings.toast.removed(removing.name));
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
