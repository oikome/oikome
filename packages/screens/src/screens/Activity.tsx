// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Activity: who asked for your data, and whether they got it, and the
// owner's own first steps (setting up, the examples, the recovery key).
// Each line opens what it means and where to act on it, as the prototype's.

import { useEffect, useState } from "react";
import { Avatar, Button, Card, Dialog, EmptyState, ErrorText, Segmented, Status } from "@oikome/ui";
import type { AccessItem } from "@oikome/client";
import { useAction, useClient } from "../data";
import { ago, listNames } from "../format";
import { ScreenHeader } from "../parts";
import { navigate } from "../router";
import { strings } from "../strings";

const s = strings.activity;
const PAGE = 50;

/** Who it was, by name: you, a person, an assistant, a plugin, or a computer with no name here. */
export function who(item: AccessItem): string {
  switch (item.who) {
    case "you":
      return strings.common.you;
    case "plugin":
      return item.name ?? s.removedPlugin;
    case "assistant":
      return item.name ?? s.removedAssistant;
    case "person":
      return item.name ?? s.unknown;
    case "unknown":
      return s.unknown;
  }
}

function lends(item: AccessItem): boolean {
  return item.method === "do_exchange:oikome.compute" || item.method === "do_action:oikome.compute_methods";
}

/** What happened, without who: "read lisbon-photos". */
function deed(item: AccessItem): string {
  return s.withoutWho(sentence(item), who(item));
}

/** What happened, with the collections it touched set apart. */
function Deed({ item }: { item: AccessItem }) {
  const line = deed(item);
  const what = listNames(item.spaces);
  const at = what ? line.indexOf(what) : -1;
  if (at < 0) return <>{line}</>;
  return (
    <>
      {line.slice(0, at)}
      <strong translate="no">{what}</strong>
      {line.slice(at + what.length)}
    </>
  );
}

/** How it reached the data center. */
function through(item: AccessItem): string {
  if (item.who === "you") return s.viaThisComputer;
  if (item.who === "plugin") return s.viaPlugin;
  if (item.who === "assistant") return s.viaAssistant;
  // Compute lines carry their grant's id too: lending first.
  if (lends(item)) return s.viaLending;
  if (item.shareId) return s.viaShare;
  return s.viaNetwork;
}

/** One line of the log, in words: "Sam read lisbon-photos". */
export function sentence(item: AccessItem): string {
  const name = who(item);
  // Using this computer, lent to them: no collection is read.
  if (lends(item)) {
    if (item.outcome === "denied") return s.turnedAwayCompute(name);
    if (item.outcome === "failed") return s.taskFailed(name);
    return item.method === "do_exchange:oikome.compute" ? s.ranTask(name) : s.checkedLending(name);
  }
  const what = item.spaces.length > 0 ? listNames(item.spaces) : s.yourData;
  // Turned away at the door: not a contact, from outside the network (3.12).
  if (item.method === "connect") return s.turnedAwayOutside(name);
  if (item.method === "owner_setup") return s.setUp(name);
  if (item.method === "owner_examples") return s.addedExamples(name, what);
  if (item.method === "owner_key") return s.savedKey(name);
  if (item.method === "owner_computed_on") return s.workedOut(name, what, item.detail ?? "");
  // A plugin on Home, reading as itself.
  if (item.method === "plugin_read") {
    return item.outcome === "allowed" ? s.readForCard(name, what) : s.pluginRefused(name, what);
  }
  // An AI assistant, searching and reading as itself.
  if (item.outcome === "allowed" && item.method === "assistant_search") {
    return s.searched(name, what, item.detail ?? "");
  }
  if (item.outcome === "allowed" && item.method === "assistant_read") return s.readRows(name, what, item.count);
  if (item.outcome === "denied") return s.turnedAway(name, what);
  if (item.outcome === "failed") return s.failed(name, what);
  if (item.method === "do_action:oikome.snapshot") return s.copied(name, what);
  if (item.method === "do_get") return s.read(name, what);
  return s.browsed(name, what);
}

/** What a line means, and where to act on it. */
function detail(item: AccessItem): { body: string; go?: { to: string; label: string } } {
  const name = who(item);
  const what = listNames(item.spaces);
  switch (item.method) {
    case "owner_setup":
      return { body: s.detailSetUp, go: { to: "/settings", label: s.goSettings } };
    case "owner_examples":
      return { body: s.detailExamples(what), go: { to: "/collections", label: s.goCollections } };
    case "owner_key":
      return { body: s.detailKey, go: { to: "/settings", label: s.goSettings } };
    case "owner_computed_on":
      return { body: s.detailWorkedOut(what, item.detail ?? ""), go: { to: "/sharing", label: s.goSharing } };
    case "plugin_read":
      return {
        body: item.outcome === "allowed" ? s.detailPlugin(name, what) : s.detailPluginRefused(name, what),
        go: { to: "/plugins", label: s.goPlugins },
      };
    case "assistant_search":
    case "assistant_read":
      return {
        body:
          item.outcome !== "allowed"
            ? s.detailAssistantRefused(name, what)
            : item.method === "assistant_search"
              ? s.detailSearched(name, what, item.detail ?? "")
              : s.detailAssistantRead(name, what, item.count),
        go: { to: "/assistants", label: s.goAssistants },
      };
    case "connect":
      return { body: s.detailOutside, go: { to: "/settings", label: s.goSettings } };
    case "do_exchange:oikome.compute":
    case "do_action:oikome.compute_methods":
      return {
        body: item.outcome === "allowed" ? s.detailCompute(name) : s.detailComputeRefused(name),
        go: { to: "/sharing", label: s.goSharing },
      };
  }
  if (item.outcome !== "allowed") {
    return item.who === "person"
      ? { body: s.detailRefused(name, what), go: { to: "/sharing", label: s.goSharing } }
      : { body: s.detailUnknown(what, item.count) };
  }
  if (item.method === "do_action:oikome.snapshot") {
    return { body: s.detailCopy(name, what), go: { to: "/sharing", label: s.goSharing } };
  }
  return { body: s.detailRead(name, what), go: { to: "/sharing", label: s.goSharing } };
}

type Filter = "all" | "people" | "assistants" | "plugins" | "you";

/** The filter a line shows under; a computer with no name here only under Everyone. */
function kindOf(item: AccessItem): Filter | undefined {
  return ({ you: "you", person: "people", assistant: "assistants", plugin: "plugins", unknown: undefined } as const)[
    item.who
  ];
}

export function Activity() {
  const client = useClient();
  const [filter, setFilter] = useState<Filter>("all");
  const [open, setOpen] = useState<AccessItem>();
  const [items, setItems] = useState<AccessItem[]>([]);
  const [more, setMore] = useState(false);
  const load = useAction();

  useEffect(() => {
    let current = true;
    load
      .run(() => client.listAccess({ limit: PAGE }))
      .then((page) => {
        if (current && page) {
          setItems(page);
          setMore(page.length === PAGE);
        }
      });
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);
  // Newest first by when it happened: the first steps of a data center set
  // up before Activity kept them are written later, with their own times.
  const shown = items
    .filter((item) => filter === "all" || kindOf(item) === filter)
    .sort((a, b) => b.atMs - a.atMs);

  return (
    <div className="screen-column screen-column--wide">
      <ScreenHeader title={s.title} lede={s.lede}>
        {/* Filtering an empty log is pointless. */}
        {items.length > 0 && (
          <Segmented
            label={s.filterLabel}
            value={filter}
            onChange={(value) => setFilter(value as Filter)}
            options={[
              { value: "all", label: s.everyone },
              { value: "people", label: s.people },
              { value: "assistants", label: s.assistants },
              { value: "plugins", label: s.plugins },
              { value: "you", label: s.you },
            ]}
          />
        )}
      </ScreenHeader>
      {load.error && <ErrorText>{load.error}</ErrorText>}
      {/* A log with nothing in it at all gets the page's empty state; an empty
          result of a filter stays a row in the list. */}
      {items.length === 0 && !load.busy ? (
        <Card>
          <EmptyState icon="activity" title={s.emptyTitle} body={s.emptyBody} />
        </Card>
      ) : (
        <>
          <div className="oik-table-wrap">
            <table className="oik-table screen-log-table" aria-label={s.recent}>
              <thead>
                <tr>
                  <th scope="col">{s.colWhen}</th>
                  <th scope="col">{s.colWho}</th>
                  <th scope="col">{s.colWhat}</th>
                  <th scope="col">{s.colThrough}</th>
                  <th scope="col">{s.colResult}</th>
                </tr>
              </thead>
              <tbody>
                {items.length === 0 && load.busy ? (
                  <tr>
                    <td colSpan={5}>
                      <span className="oik-skel oik-skel--line" aria-hidden="true" />
                    </td>
                  </tr>
                ) : shown.length === 0 ? (
                  <tr>
                    <td colSpan={5} className="screen-log-table__empty">
                      {s.empty}
                    </td>
                  </tr>
                ) : (
                  shown.map((item) => (
                    <tr
                      key={item.id}
                      className={item.outcome === "allowed" ? "screen-log-table__row" : "screen-log-table__row screen-log-table__refused"}
                      onClick={() => setOpen(item)}
                    >
                      <td className="oik-num screen-log-table__when">{ago(item.atMs)}</td>
                      <td>
                        <span className="screen-log-table__who">
                          <Avatar name={who(item)} small />
                          {who(item)}
                        </span>
                      </td>
                      <td className="screen-log-table__what">
                        {/* The line opens what it means; a button, for the keyboard too. */}
                        <button type="button" className="screen-log-table__open" onClick={() => setOpen(item)}>
                          <Deed item={item} />
                        </button>
                        {/* An assistant's reads say how many in their words. */}
                        {item.count > 1 && item.method !== "assistant_read" && (
                          <span className="screen-log-table__times" title={s.times(item.count)}>
                            {" "}×{item.count}
                          </span>
                        )}
                      </td>
                      <td className="screen-log-table__via">{through(item)}</td>
                      <td>
                        <Status kind={item.outcome === "allowed" ? "ok" : "bad"}>
                          {item.outcome === "allowed" ? s.allowed : s.refused}
                        </Status>
                      </td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>
          <p className="screen-footnote">{s.footnote}</p>
        </>
      )}
      {open && (
        <LineDetail
          item={open}
          onClose={() => setOpen(undefined)}
        />
      )}
      {more && (
        <Button
          disabled={load.busy}
          onClick={async () => {
            const last = items[items.length - 1];
            const page = await load.run(() =>
              client.listAccess({ limit: PAGE, beforeId: last?.id }),
            );
            if (page) {
              setItems([...items, ...page]);
              setMore(page.length === PAGE);
            }
          }}
        >
          {s.more}
        </Button>
      )}
    </div>
  );
}

/** One line of Activity, opened: who and when, what it means, and where to act on it. */
function LineDetail({ item, onClose }: { item: AccessItem; onClose: () => void }) {
  const { body, go } = detail(item);
  const title = item.outcome === "allowed" ? `${who(item)} · ${ago(item.atMs)}` : s.turnedAwayTitle;
  return (
    <Dialog
      title={title}
      closeLabel={strings.common.close}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>{strings.common.close}</Button>
          {go && (
            <Button
              variant="primary"
              onClick={() => {
                onClose();
                navigate(go.to);
              }}
            >
              {go.label}
            </Button>
          )}
        </>
      }
    >
      <p className="oik-dialog__text">{body}</p>
    </Dialog>
  );
}
