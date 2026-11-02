// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The node the screens talk to, and a small hook for loading from it.

import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import { OikomeError } from "@oikome/client";
import type { NodeClient } from "@oikome/client";
import { strings } from "./strings";

const ClientContext = createContext<NodeClient | null>(null);

export function ClientProvider({ client, children }: { client: NodeClient; children: ReactNode }) {
  return <ClientContext.Provider value={client}>{children}</ClientContext.Provider>;
}

export function useClient(): NodeClient {
  const client = useContext(ClientContext);
  if (!client) throw new Error("useClient needs a ClientProvider");
  return client;
}

/** What to tell a person about a failed call. */
export function errorMessage(e: unknown): string {
  if (e instanceof OikomeError) {
    if (e.code === "network") return strings.errors.network;
    if (e.code === "unauthenticated") return strings.errors.session;
    return e.message;
  }
  return strings.errors.generic;
}

export interface Loaded<T> {
  data: T | undefined;
  error: string | undefined;
  loading: boolean;
  reload: () => void;
}

/** Load `load()` now and whenever `deps` change; `reload` loads again. */
export function useLoad<T>(load: () => Promise<T>, deps: readonly unknown[]): Loaded<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);
  const latest = useRef(load);
  latest.current = load;
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
        if (current) setError(errorMessage(e));
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
  run: <T>(action: () => Promise<T>) => Promise<T | undefined>;
  clear: () => void;
} {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const run = useCallback(async <T,>(action: () => Promise<T>) => {
    setBusy(true);
    setError(undefined);
    try {
      return await action();
    } catch (e) {
      setError(errorMessage(e));
      return undefined;
    } finally {
      setBusy(false);
    }
  }, []);
  return { busy, error, run, clear: () => setError(undefined) };
}
