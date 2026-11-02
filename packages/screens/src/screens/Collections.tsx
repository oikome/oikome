// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Collections: the list, and what is in the selected one, with who can see it.

import { useEffect, useRef, useState } from "react";
import {
  Avatar,
  AvatarStack,
  Button,
  Card,
  ConfirmDialog,
  Dialog,
  EmptyState,
  ErrorText,
  Group,
  Menu,
  Row,
  Select,
  SkeletonRows,
  TextField,
  Tile,
  collectionTones,
  useToast,
} from "@oikome/ui";
import type { TileTone } from "@oikome/ui";
import type { Contact, Share, SpaceSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { RowsTable, ScreenHeader } from "../parts";
import { Link, navigate } from "../router";
import { strings } from "../strings";
import { ShareDialog } from "./ShareDialog";

const PAGE_ROWS = 100;
const s = strings.collections;
/** The node's rule for a name: ASCII letters, digits, _ and -, not led by a digit, 1 to 64. */
const NAME = /^[A-Za-z_-][A-Za-z0-9_-]{0,63}$/;

/** Contact names each collection is shared with now. */
function sharedNames(shares: Share[] | undefined): Map<string, string[]> {
  const names = new Map<string, string[]>();
  for (const share of shares ?? []) {
    if (share.status !== "active") continue;
    const list = names.get(share.space) ?? [];
    const name = share.contactName ?? strings.common.unknown;
    if (!list.includes(name)) list.push(name);
    names.set(share.space, list);
  }
  return names;
}

export function Collections({
  selected,
  startCreating = false,
}: {
  selected?: string;
  /** Opened to make one (?new=1), from a step that needs a collection first. */
  startCreating?: boolean;
}) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const contacts = useLoad(() => client.listContacts(), []);
  const [creating, setCreating] = useState(startCreating);
  useEffect(() => {
    if (startCreating) setCreating(true);
  }, [startCreating]);
  const [name, setName] = useState("");
  const create = useAction();
  const toast = useToast();
  const shared = sharedNames(shares.data);
  const list = spaces.data ?? [];
  const mine = list.filter((space) => space.kind !== "remote");
  const fromOthers = list.filter((space) => space.kind === "remote");
  const current = list.find((space) => space.name === selected);
  const toneOf = collectionTones(list.map((space) => space.name));
  const item = (space: SpaceSummary) => {
    const names = shared.get(space.name) ?? [];
    return (
      <Row
        key={space.name}
        leading={<Tile icon="collection" tone={toneOf(space.name)} />}
        title={space.name}
        meta={
          space.kind === "remote"
            ? s.fromElsewhere
            : names.length > 0
              ? s.sharedWith(names)
              : s.notShared
        }
        selected={space.name === selected}
        trailing={
          space.kind === "remote" ? undefined : (
            <AvatarStack names={names} label={s.whoSentence(names)} />
          )
        }
        href={`/collections/${encodeURIComponent(space.name)}`}
        onClick={() => navigate(`/collections/${encodeURIComponent(space.name)}`)}
      />
    );
  };

  const reload = () => {
    spaces.reload();
    shares.reload();
  };

  // Side by side there is always a collection open, as in the design: the
  // first one, in place of an empty "choose one" panel. Below 960px the list
  // stands alone until one is chosen.
  const first = mine[0] ?? fromOthers[0];
  useEffect(() => {
    if (!selected && first && window.matchMedia("(min-width: 961px)").matches) {
      navigate(`/collections/${encodeURIComponent(first.name)}`, { replace: true });
    }
  }, [selected, first]);

  const empty = spaces.data !== undefined && list.length === 0;
  const stopCreating = () => {
    setCreating(false);
    setName("");
    create.clear();
    // Drop ?new=1, so a reload does not open the form again.
    if (startCreating) navigate(window.location.pathname, { replace: true });
  };

  return (
    <>
      <ScreenHeader title={s.title}>
        {/* An empty page offers the same action once, in its empty state. */}
        {!empty && (
          <Button icon="plus" onClick={() => setCreating(true)}>
            {s.newCollection}
          </Button>
        )}
      </ScreenHeader>

      {/* Making a collection is a short task in a dialog, like sharing one. */}
      {creating && (
        <Dialog
          title={s.newCollection}
          closeLabel={strings.common.close}
          onClose={stopCreating}
          footer={
            <>
              <Button onClick={stopCreating}>{strings.common.cancel}</Button>
              <Button type="submit" form="new-collection" variant="primary" disabled={create.busy}>
                {s.create}
              </Button>
            </>
          }
        >
          <form
            id="new-collection"
            onSubmit={async (e) => {
              e.preventDefault();
              const wanted = name.trim();
              if (!wanted) {
                create.fail(strings.common.nameRequired);
                return;
              }
              if (!NAME.test(wanted)) {
                create.fail(s.nameInvalid);
                return;
              }
              const space = await create.run(
                () => client.createSpace({ name: wanted }),
                (err) => (err.code === "conflict" ? s.nameTaken(wanted) : undefined),
              );
              if (space) {
                toast(strings.toast.created(space.name));
                stopCreating();
                reload();
                navigate(`/collections/${encodeURIComponent(space.name)}`);
              }
            }}
          >
            <TextField
              label={s.nameLabel}
              name="collection-name"
              hint={s.nameHint}
              value={name}
              onChange={(value) => {
                setName(value);
                create.clear();
              }}
              error={create.error}
              autoFocus
            />
          </form>
        </Dialog>
      )}

      {spaces.error && <ErrorText>{spaces.error}</ErrorText>}
      {!spaces.data && !spaces.error && (
        <div className="screen-split">
          <div className="screen-split__list">
            <Group title={s.mine} variant="list">
              <SkeletonRows />
            </Group>
          </div>
        </div>
      )}
      {empty && (
        <Card>
          <EmptyState
            icon="collection"
            title={s.emptyTitle}
            body={s.emptyBody}
            action={
              <Button variant="primary" icon="plus" onClick={() => setCreating(true)}>
                {s.newCollection}
              </Button>
            }
          />
        </Card>
      )}

      {list.length > 0 && (
        <div className="screen-split">
          <div className="screen-split__list">
            {mine.length > 0 && (
              <Group title={s.mine} variant="list">
                {mine.map(item)}
              </Group>
            )}
            {fromOthers.length > 0 && (
              <Group title={s.sharedWithYou} variant="list">
                {fromOthers.map(item)}
              </Group>
            )}
          </div>
          {current ? (
            <Detail
              key={current.name}
              space={current}
              tone={toneOf(current.name)}
              toneOf={toneOf}
              mine={mine}
              contacts={contacts.data ?? []}
              shares={shares.data ?? []}
              onShared={reload}
              names={shared.get(current.name) ?? []}
              onRemoved={() => {
                toast(strings.toast.removed(current.name));
                reload();
                // The removed collection's page is gone: Back skips it.
                navigate("/collections", { replace: true });
              }}
            />
          ) : (
            <div className="screen-split__placeholder">
              <Card>
                <p className="screen-muted">{s.pick}</p>
              </Card>
            </div>
          )}
        </div>
      )}
    </>
  );
}

function Detail({
  space,
  tone,
  toneOf,
  mine,
  contacts,
  shares,
  names,
  onShared,
  onRemoved,
}: {
  space: SpaceSummary;
  tone: TileTone;
  toneOf: (name: string) => TileTone;
  /** This data center's own collections, for the share dialog. */
  mine: SpaceSummary[];
  contacts: Contact[];
  shares: Share[];
  names: string[];
  onShared: () => void;
  onRemoved: () => void;
}) {
  const client = useClient();
  const local = space.kind !== "remote";
  // A shared collection is read from its owner's computer, which can be off
  // or may have stopped sharing; say so instead of the node's message.
  const unreadable = local ? undefined : () => s.cantRead;
  const tables = useLoad(() => client.listTables({ space: space.name }), [space.name], unreadable);
  const [table, setTable] = useState<string>();
  const [offset, setOffset] = useState(0);
  const [confirming, setConfirming] = useState(false);
  const [sharing, setSharing] = useState(false);
  const remove = useAction();

  useEffect(() => {
    const first = tables.data?.[0]?.name;
    if (first && !table) setTable(first);
  }, [tables.data, table]);

  const rows = useLoad(
    async () =>
      table ? client.readRows({ space: space.name, table, limit: PAGE_ROWS, offset }) : undefined,
    [space.name, table, offset],
    unreadable,
  );
  const count = rows.data?.table.numRows ?? 0;

  const panel = useRef<HTMLElement>(null);

  // Below 960px the collection sits under the list: bring it into view.
  useEffect(() => {
    if (window.matchMedia("(max-width: 960px)").matches) {
      panel.current?.scrollIntoView({ block: "start" });
    }
  }, []);

  return (
    <section ref={panel} className="screen-detail" aria-label={space.name}>
      <div className="screen-detail__head">
        <Tile icon="collection" tone={tone} large />
        <div className="screen-detail__name">
          <h2>{space.name}</h2>
          <p className="screen-muted">{local ? s.storedHere : s.fromElsewhere}</p>
        </div>
        {local && (
          <Button variant="primary" onClick={() => setSharing(true)}>
            {s.share}
          </Button>
        )}
        <Menu
          label={strings.common.moreActions(space.name)}
          items={[{ label: s.remove, destructive: true, onSelect: () => setConfirming(true) }]}
        />
      </div>

      {local && (
        <div className="screen-who">
          <span className="screen-who__label">{s.whoCanSee}</span>
          <span className="screen-chip">
            <Avatar name={strings.common.you} you small />
            {strings.common.you}
          </span>
          {names.map((name) => (
            <span key={name} className="screen-chip">
              <Avatar name={name} small />
              {name}
            </span>
          ))}
          <span className="screen-who__manage">
            <Link to="/sharing">{s.manage}</Link>
          </span>
        </div>
      )}

      {sharing && (
        <ShareDialog
          collection={space.name}
          spaces={mine}
          contacts={contacts}
          shares={shares}
          toneOf={toneOf}
          onClose={() => setSharing(false)}
          onChange={onShared}
        />
      )}

      {confirming && (
        <ConfirmDialog
          title={s.removeTitle(space.name)}
          body={s.removeBody}
          confirmLabel={s.removeYes}
          cancelLabel={strings.common.cancel}
          busy={remove.busy}
          error={remove.error}
          onConfirm={async () => {
            const done = await remove.run(() => client.dropSpace({ space: space.name }));
            if (done) onRemoved();
          }}
          onCancel={() => {
            setConfirming(false);
            remove.clear();
          }}
        />
      )}

      {tables.error && <ErrorText>{tables.error}</ErrorText>}
      {tables.data && tables.data.length === 0 && (
        <EmptyState
          icon="table"
          title={s.nothingYetTitle}
          body={s.nothingYetBody}
          action={<Button onClick={() => navigate("/settings/advanced")}>{s.openAdvanced}</Button>}
        />
      )}
      {tables.data && tables.data.length > 0 && table && (
        <div className="screen-detail__content">
          {tables.data.length > 1 && (
            <div className="screen-toolbar">
              <Select
                inline
                label={s.tableLabel}
                value={table}
                onChange={(value) => {
                  setTable(value);
                  setOffset(0);
                }}
                options={tables.data.map((t) => ({ value: t.name, label: t.name }))}
              />
            </div>
          )}
          {rows.error && <ErrorText>{rows.error}</ErrorText>}
          {rows.data && <RowsTable table={rows.data.table} label={table} />}
          {rows.data && count === 0 && <p className="screen-muted screen-result">{s.noRows}</p>}
          {/* Paging only when there is more than one page. */}
          {(offset > 0 || count === PAGE_ROWS) && (
            <div className="screen-pager">
              <span className="screen-muted">{s.rows(offset + 1, offset + count)}</span>
              <span>
                <Button
                  variant="quiet"
                  disabled={offset === 0}
                  onClick={() => setOffset(Math.max(0, offset - PAGE_ROWS))}
                >
                  {s.previous}
                </Button>
                <Button
                  variant="quiet"
                  disabled={count < PAGE_ROWS}
                  onClick={() => setOffset(offset + PAGE_ROWS)}
                >
                  {s.next}
                </Button>
              </span>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
