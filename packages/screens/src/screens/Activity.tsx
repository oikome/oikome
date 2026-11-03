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

/** One line of the log, in words: "Sam read lisbon-photos". */
export function sentence(item: AccessItem): string {
  const who = item.contactName ?? (item.did ? s.notAContact : strings.common.unknown);
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
                  <th scope="col">{s.colWhat}</th>
                  <th scope="col">{s.colResult}</th>
                </tr>
              </thead>
              <tbody>
                {items.length === 0 && load.busy ? (
                  <tr>
                    <td colSpan={3}>
                      <span className="oik-skel oik-skel--line" aria-hidden="true" />
                    </td>
                  </tr>
                ) : items.length === 0 ? (
                  <tr>
                    <td colSpan={3} className="screen-log-table__empty">
                      {s.empty}
                    </td>
                  </tr>
                ) : (
                  items.map((item) => (
                    <tr key={item.id}>
                      <td className="oik-num screen-log-table__when">
                        {item.count > 1 ? `${ago(item.atMs)} · ${s.times(item.count)}` : ago(item.atMs)}
                      </td>
                      <td>
                        <span className="screen-log-table__who">
                          <Avatar name={item.contactName ?? "?"} small />
                          {sentence(item)}
                        </span>
                      </td>
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
