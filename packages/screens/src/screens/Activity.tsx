// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Activity: who asked for your data, and whether they got it.

import { useEffect, useState } from "react";
import { Avatar, Button, Card, EmptyState, ErrorText, Select, Status } from "@oikome/ui";
import type { AccessItem } from "@oikome/client";
import { useAction, useClient, useLoad } from "../data";
import { ago } from "../format";
import { ScreenHeader } from "../parts";
import { strings } from "../strings";

const s = strings.activity;
const PAGE = 50;

/** Who it was, in the words the owner uses. */
export function who(item: AccessItem): string {
  return item.contactName ?? (item.did ? s.notAContact : strings.common.unknown);
}

/** What happened, without who: "read lisbon-photos". */
function deed(item: AccessItem): string {
  const line = sentence(item);
  const name = who(item);
  return line.startsWith(`${name} `) ? line.slice(name.length + 1) : line;
}

/** What happened, with the collections it touched set apart. */
function Deed({ item }: { item: AccessItem }) {
  const line = deed(item);
  const what = item.spaces.join(", ");
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
  if (item.shareId) return s.viaShare;
  return item.did ? s.viaContact : s.viaNetwork;
}

/** One line of the log, in words: "Sam read lisbon-photos". */
export function sentence(item: AccessItem): string {
  const who = item.contactName ?? (item.did ? s.notAContact : strings.common.unknown);
  // Using this computer, lent to them: no collection is read.
  if (item.method === "do_exchange:oikome.compute" || item.method === "do_action:oikome.compute_methods") {
    if (item.outcome === "denied") return s.turnedAwayCompute(who);
    if (item.outcome === "failed") return s.taskFailed(who);
    return item.method === "do_exchange:oikome.compute" ? s.ranTask(who) : s.checkedLending(who);
  }
  const what = item.spaces.length > 0 ? item.spaces.join(", ") : s.yourData;
  if (item.outcome === "denied") return s.turnedAway(who, what);
  if (item.outcome === "failed") return s.failed(who, what);
  if (item.method === "do_action:oikome.snapshot") return s.copied(who, what);
  if (item.method === "do_get") return s.read(who, what);
  return s.browsed(who, what);
}

export function Activity() {
  const client = useClient();
  const contacts = useLoad(() => client.listContacts(), []);
  const [did, setDid] = useState("");
  const [items, setItems] = useState<AccessItem[]>([]);
  const [more, setMore] = useState(false);
  const load = useAction();

  useEffect(() => {
    let current = true;
    load
      .run(() => client.listAccess({ limit: PAGE, did: did || undefined }))
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
  }, [client, did]);

  return (
    <div className="screen-column screen-column--wide">
      <ScreenHeader title={s.title} lede={s.lede}>
        {/* Filtering an empty log is pointless, unless a filter emptied it. */}
        {(contacts.data?.length ?? 0) > 0 && (items.length > 0 || did !== "") && (
          <Select
            inline
            label={s.filterLabel}
            value={did}
            onChange={setDid}
            options={[
              { value: "", label: s.everyone },
              ...(contacts.data ?? []).map((c) => ({ value: c.did, label: c.name })),
            ]}
          />
        )}
      </ScreenHeader>
      {load.error && <ErrorText>{load.error}</ErrorText>}
      {/* A log with nothing in it at all gets the page's empty state; an empty
          result of a filter stays a row in the list. */}
      {items.length === 0 && !load.busy && did === "" ? (
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
                ) : items.length === 0 ? (
                  <tr>
                    <td colSpan={5} className="screen-log-table__empty">
                      {s.empty}
                    </td>
                  </tr>
                ) : (
                  items.map((item) => (
                    <tr
                      key={item.id}
                      className={item.outcome === "allowed" ? undefined : "screen-log-table__refused"}
                    >
                      <td className="oik-num screen-log-table__when">{ago(item.atMs)}</td>
                      <td>
                        <span className="screen-log-table__who">
                          <Avatar name={who(item)} small />
                          {who(item)}
                        </span>
                      </td>
                      <td>
                        <Deed item={item} />
                        {item.count > 1 && (
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
      {more && (
        <Button
          disabled={load.busy}
          onClick={async () => {
            const last = items[items.length - 1];
            const page = await load.run(() =>
              client.listAccess({ limit: PAGE, did: did || undefined, beforeId: last?.id }),
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
