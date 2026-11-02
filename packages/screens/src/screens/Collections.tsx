// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Collections: the list, and what is in the selected one, with who can see it.

import { useEffect, useState } from "react";
import {
  AvatarStack,
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
import { navigate } from "../router";
import { strings } from "../strings";

const PAGE_ROWS = 100;
const s = strings.collections;

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
              const space = await create.run(() => client.createSpace({ name: name.trim() }));
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
              onChange={setName}
              autoFocus
            />
            {create.error && <ErrorText>{create.error}</ErrorText>}
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
          <div>
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
                navigate("/collections");
              }}
            />
          ) : (
            <Card>
              <p className="screen-muted">{s.pick}</p>
            </Card>
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
  const tables = useLoad(() => client.listTables({ space: space.name }), [space.name]);
  const [table, setTable] = useState<string>();
  const [offset, setOffset] = useState(0);
  const [confirming, setConfirming] = useState(false);
  const remove = useAction();
  const local = space.kind !== "remote";

  useEffect(() => {
    const first = tables.data?.[0]?.name;
    if (first && !table) setTable(first);
  }, [tables.data, table]);

  const rows = useLoad(
    async () =>
      table ? client.readRows({ space: space.name, table, limit: PAGE_ROWS, offset }) : undefined,
    [space.name, table, offset],
  );
  const count = rows.data?.table.numRows ?? 0;

  return (
    <div className="screen-stack">
      <Card>
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
            <span className="oik-field__label">{s.whoCanSee}</span>
            <div className="screen-who__row">
              <AvatarStack
                names={[strings.common.you, ...names]}
                label={[strings.common.you, ...names].join(", ")}
              />
              <span className="screen-who__names">
                {[strings.common.you, ...names].join(", ")}
              </span>
              <Button variant="text" onClick={() => navigate("/sharing")}>
                {s.manage}
              </Button>
            </div>
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
      </Card>

      {tables.error && <ErrorText>{tables.error}</ErrorText>}
      {tables.data && tables.data.length === 0 && (
        <Card>
          <EmptyState
            title={s.nothingYetTitle(space.name)}
            body={s.nothingYetBody}
            action={
              <Button onClick={() => navigate("/settings/advanced")}>{s.openAdvanced}</Button>
            }
          />
        </Card>
      )}
      {tables.data && tables.data.length > 0 && table && (
        <section>
          {tables.data.length > 1 && (
            <Select
              label={s.tableLabel}
              value={table}
              onChange={(value) => {
                setTable(value);
                setOffset(0);
              }}
              options={tables.data.map((t) => ({ value: t.name, label: t.name }))}
            />
          )}
          {rows.error && <ErrorText>{rows.error}</ErrorText>}
          {rows.data && <RowsTable table={rows.data.table} label={table} />}
          <div className="screen-pager">
            <span className="screen-muted">
              {count === 0 ? s.noRows : s.rows(offset + 1, offset + count)}
            </span>
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
        </section>
      )}
    </div>
  );
}
