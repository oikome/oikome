// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Collections: the list, and what is in the selected one, with who can see it.

import { useEffect, useRef, useState } from "react";
import {
  Avatar,
  Button,
  Card,
  EmptyState,
  ErrorText,
  Group,
  Menu,
  Row,
  Select,
  TextField,
  Tile,
  toneFor,
} from "@oikome/ui";
import type { Share, SpaceSummary } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { RowsTable } from "../parts";
import { Link, navigate } from "../router";
import { strings } from "../strings";

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

export function Collections({ selected }: { selected?: string }) {
  const client = useClient();
  const spaces = useLoad(() => client.listSpaces(), []);
  const shares = useLoad(() => client.listShares(), []);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const create = useAction();
  const shared = sharedNames(shares.data);
  const list = spaces.data ?? [];
  const mine = list.filter((space) => space.kind !== "remote");
  const fromOthers = list.filter((space) => space.kind === "remote");
  const current = list.find((space) => space.name === selected);
  const item = (space: SpaceSummary) => {
    const names = shared.get(space.name) ?? [];
    return (
      <Row
        key={space.name}
        leading={<Tile icon="collection" tone={toneFor(space.name)} />}
        title={space.name}
        meta={
          space.kind === "remote"
            ? s.fromElsewhere
            : names.length > 0
              ? s.sharedWith(names)
              : s.notShared
        }
        selected={space.name === selected}
        href={`/collections/${encodeURIComponent(space.name)}`}
        onClick={() => navigate(`/collections/${encodeURIComponent(space.name)}`)}
      />
    );
  };

  const reload = () => {
    spaces.reload();
    shares.reload();
  };

  return (
    <>
      <header className="screen-header">
        <h1 className="screen-title">{s.title}</h1>
        {!creating && (
          <Button icon="plus" onClick={() => setCreating(true)}>
            {s.newCollection}
          </Button>
        )}
      </header>

      {creating && (
        <Card className="screen-panel">
          <form
            className="screen-form"
            onSubmit={async (e) => {
              e.preventDefault();
              const wanted = name.trim();
              if (!NAME.test(wanted)) {
                create.fail(s.nameInvalid);
                return;
              }
              const space = await create.run(
                () => client.createSpace({ name: wanted }),
                (err) => (err.code === "conflict" ? s.nameTaken(wanted) : undefined),
              );
              if (space) {
                setCreating(false);
                setName("");
                reload();
                navigate(`/collections/${encodeURIComponent(space.name)}`);
              }
            }}
          >
            <TextField
              label={s.nameLabel}
              hint={s.nameHint}
              value={name}
              onChange={(value) => {
                setName(value);
                create.clear();
              }}
              error={create.error}
              autoFocus
            />
            <div className="screen-actions">
              <Button type="submit" variant="primary" disabled={create.busy || !name.trim()}>
                {s.create}
              </Button>
              <Button
                variant="text"
                onClick={() => {
                  setCreating(false);
                  create.clear();
                }}
              >
                {strings.common.cancel}
              </Button>
            </div>
          </form>
        </Card>
      )}

      {spaces.error && <ErrorText>{spaces.error}</ErrorText>}
      {spaces.data && list.length === 0 && !creating && (
        <Card>
          <EmptyState
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
              names={shared.get(current.name) ?? []}
              onRemoved={() => {
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
  names,
  onRemoved,
}: {
  space: SpaceSummary;
  names: string[];
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
  const who = [strings.common.you, ...names];
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
        <Tile icon="collection" tone={toneFor(space.name)} large />
        <div className="screen-detail__name">
          <h2>{space.name}</h2>
          <p className="screen-muted">{local ? s.storedHere : s.fromElsewhere}</p>
        </div>
        {local && (
          <Button
            variant="primary"
            onClick={() => navigate(`/sharing?share=${encodeURIComponent(space.name)}`)}
          >
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
          {who.map((name) => (
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

      {confirming && (
        <div className="screen-confirm">
          <p className="screen-confirm__text">{s.removeConfirm(space.name)}</p>
          {remove.error && <ErrorText>{remove.error}</ErrorText>}
          <div className="screen-confirm__actions">
            <Button
              variant="destructive"
              disabled={remove.busy}
              onClick={async () => {
                const done = await remove.run(() => client.dropSpace({ space: space.name }));
                if (done) onRemoved();
              }}
            >
              {s.removeYes}
            </Button>
            <Button variant="quiet" onClick={() => setConfirming(false)}>
              {strings.common.cancel}
            </Button>
          </div>
        </div>
      )}

      {tables.error && <ErrorText>{tables.error}</ErrorText>}
      {tables.data && tables.data.length === 0 && (
        <EmptyState
          title={s.nothingYetTitle(space.name)}
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
          <div className="screen-pager">
            <span className="screen-muted">
              {count === 0 ? s.noRows : s.rows(offset + 1, offset + count)}
            </span>
            {(offset > 0 || count === PAGE_ROWS) && (
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
            )}
          </div>
        </div>
      )}
    </section>
  );
}
