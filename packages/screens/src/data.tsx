// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The node the screens talk to, and a small hook for loading from it.

import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { OikomeError } from "@oikome/client";
import type { NodeClient } from "@oikome/client";
import { strings } from "./strings";
import { sentence } from "./i18n/en";

const ClientContext = createContext<NodeClient | null>(null);

export function ClientProvider({ client, children }: { client: NodeClient; children: ReactNode }) {
  return <ClientContext.Provider value={client}>{children}</ClientContext.Provider>;
}

export function useClient(): NodeClient {
  const client = useContext(ClientContext);
  if (!client) throw new Error("useClient needs a ClientProvider");
  return client;
}

/**
 * Words for one failed call in its context ("a collection with that name
 * already exists"); undefined falls back to the general wording.
 */
export type Describe = (e: OikomeError) => string | undefined;

/**
 * What to tell a person about a failed call. The node's own messages name
 * engine concepts (spaces, tokens), so outside the SQL workbench people see
 * the context's words or a plain sentence for the kind of failure.
 */
export function errorMessage(e: unknown, describe?: Describe): string {
  if (!(e instanceof OikomeError)) return strings.errors.generic;
  if (e.code === "network") return strings.errors.network;
  if (e.code === "unauthenticated") return strings.errors.session;
  const own = describe?.(e);
  if (own) return own;
  switch (e.code) {
    case "permission_denied":
      return strings.errors.refused;
    case "not_found":
      return strings.errors.gone;
    case "conflict":
      return strings.errors.conflict;
    case "invalid":
      return strings.errors.invalid;
    default:
      return strings.errors.generic;
  }
}

/** The node's own message: for the SQL workbench, whose people know SQL. */
export const rawMessage: Describe = (e) => e.message;

/**
 * For calls whose failures are written for people (invites, plugins): the
 * app's words for the node's reason, in the language in use, else the
 * node's English message as a sentence ("this invite has ended" reads
 * "This invite has ended.").
 */
export const asSentence: Describe = (e) => {
  const words = e.reason ? (strings.reasons as Record<string, unknown>)[e.reason] : undefined;
  if (typeof words === "string") return words;
  if (typeof words === "function") return (words as (params: Record<string, string>) => string)(e.params);
  return sentence(e.message) || undefined;
};

export interface Loaded<T> {
  data: T | undefined;
  error: string | undefined;
  loading: boolean;
  reload: () => void;
}

/** Load `load()` now and whenever `deps` change; `reload` loads again. */
export function useLoad<T>(
  load: () => Promise<T>,
  deps: readonly unknown[],
  describe?: Describe,
): Loaded<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);
  const latest = useRef(load);
  latest.current = load;
  const words = useRef(describe);
  words.current = describe;
  useEffect(() => {
    let current = true;
    setLoading(true);
    latest
      .current()
      .then((value) => {
        if (current) {
          setData(value);
          setError(undefined);
        }
      })
      .catch((e: unknown) => {
        if (current) setError(errorMessage(e, words.current));
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick]);
  const reload = useCallback(() => setTick((t) => t + 1), []);
  return { data, error, loading, reload };
}

/** Run an action, keeping its error message and whether it is running. */
export function useAction(): {
  busy: boolean;
  error: string | undefined;
  /** Runs `action`; a failure is put in `describe`'s words when it has some. */
  run: <T>(action: () => Promise<T>, describe?: Describe) => Promise<T | undefined>;
  /** Shows a problem found before calling the node (a name in the wrong shape). */
  fail: (message: string) => void;
  clear: () => void;
} {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const run = useCallback(async <T,>(action: () => Promise<T>, describe?: Describe) => {
    setBusy(true);
    setError(undefined);
    try {
      return await action();
    } catch (e) {
      setError(errorMessage(e, describe));
      return undefined;
    } finally {
      setBusy(false);
    }
  }, []);
  return { busy, error, run, fail: setError, clear: () => setError(undefined) };
}
