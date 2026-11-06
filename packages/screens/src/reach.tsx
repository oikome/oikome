// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Asking before this computer is reached from outside its network (3.12),
// as operating systems ask before an app accepts connections: Deny or
// Allow, with "Don't ask again". Without it the answer is for this one
// share or lending and ends with it; with it, it becomes the rule, which
// Settings › Network changes back. Asked only while the rule is to ask.

import { useCallback, useRef, useState } from "react";
import type { ReactNode } from "react";
import { Button, Checkbox, Dialog } from "@oikome/ui";
import type { NodeClient, ReachChoice } from "@oikome/client";
import { useClient } from "./data";
import { strings } from "./strings";

const r = strings.reach;

/** What needs this computer reached from outside. */
export type ReachFor =
  | { kind: "share"; collection: string; name: string }
  | { kind: "lend"; names: string[] };

/**
 * `ask` resolves with the owner's answer, `undefined` when there was
 * nothing to ask (the rule is to allow or deny), or `null` when the owner
 * went back without answering (nothing should happen).
 */
export function useReachPrompt(): {
  ask: (what: ReachFor) => Promise<ReachChoice | undefined | null>;
  prompt: ReactNode;
} {
  const client = useClient();
  const [asking, setAsking] = useState<ReachFor>();
  const answer = useRef<(choice: ReachChoice | null) => void>(undefined);

  const ask = useCallback(
    async (what: ReachFor) => {
      const network = await client.network().catch(() => undefined);
      if (!network?.available || network.policy !== "ask") return undefined;
      return new Promise<ReachChoice | null>((resolve) => {
        answer.current = (choice) => {
          setAsking(undefined);
          resolve(choice);
        };
        setAsking(what);
      });
    },
    [client],
  );

  const prompt = asking ? (
    <ReachDialog what={asking} onAnswer={(choice) => answer.current?.(choice)} />
  ) : null;
  return { ask, prompt };
}

function ReachDialog({ what, onAnswer }: { what: ReachFor; onAnswer: (choice: ReachChoice | null) => void }) {
  const [remember, setRemember] = useState(false);
  const names = what.kind === "lend" ? r.names(what.names) : "";
  return (
    <Dialog
      title={r.title}
      closeLabel={strings.common.cancel}
      closeButton={false}
      onClose={() => onAnswer(null)}
      footer={
        <>
          <Button autoFocus onClick={() => onAnswer({ allow: false, remember })}>
            {r.deny}
          </Button>
          <Button variant="primary" onClick={() => onAnswer({ allow: true, remember })}>
            {r.allow}
          </Button>
        </>
      }
    >
      <p className="oik-dialog__text">
        {what.kind === "share" ? r.whyShare(what.collection, what.name) : r.whyLend(names)}
      </p>
      <p className="oik-dialog__text">{r.risk}</p>
      <p className="oik-dialog__text">{r.stays}</p>
      <p className="oik-dialog__text">
        {what.kind === "share" ? r.whileShare(what.collection, what.name) : r.whileLend(names)}
      </p>
      <Checkbox label={r.dontAsk} checked={remember} onChange={setRemember} />
    </Dialog>
  );
}

/**
 * Say what the answer did, and when it opened the port, whether the router
 * opened it (it was asked while the share or lending was made).
 */
export async function toastReach(
  client: NodeClient,
  toast: (message: string) => void,
  what: ReachFor,
  choice: ReachChoice | undefined,
): Promise<void> {
  if (!choice) return;
  const names = what.kind === "lend" ? r.names(what.names) : "";
  if (!choice.allow) {
    toast(
      choice.remember
        ? r.neverAllowed
        : what.kind === "share"
          ? r.deniedShare(what.name, what.collection)
          : r.deniedLend(names),
    );
    return;
  }
  const network = await client.network().catch(() => undefined);
  if (network?.reachable === "router_refused") {
    toast(r.routerRefused);
    return;
  }
  toast(
    choice.remember
      ? r.alwaysAllowed
      : what.kind === "share"
        ? r.allowedShare(what.name, what.collection)
        : r.allowedLend(names),
  );
}
