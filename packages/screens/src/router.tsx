// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A few routes on the History API; the server answers every route with the
// app (paths without an extension get index.html).

import { useEffect, useState } from "react";
import type { MouseEvent, ReactNode } from "react";

const CHANGE = "oikome:navigate";

function here(): string {
  return window.location.pathname + window.location.search;
}

/**
 * Go to `to`. A new step starts at the top of the page and remembers where
 * it came from, so Back links can return there. `replace` swaps the current
 * step instead: for redirects, and for pages that should not be returned to
 * (a removed collection, a panel that was closed). `keepScroll` stays where
 * the page is: for an item opened over it.
 */
export function navigate(
  to: string,
  { replace = false, keepScroll = false }: { replace?: boolean; keepScroll?: boolean } = {},
) {
  if (to !== here()) {
    if (replace) {
      window.history.replaceState(window.history.state, "", to);
    } else {
      window.history.pushState({ from: here() }, "", to);
      if (!keepScroll) window.scrollTo(0, 0);
    }
  }
  window.dispatchEvent(new Event(CHANGE));
}

/** The page this step was opened from inside the app, if any. */
export function previousPage(): string | undefined {
  const from: unknown = (window.history.state as { from?: unknown } | null)?.from;
  return typeof from === "string" ? from : undefined;
}

/** The current path and query, updated on navigation. */
export function useLocation(): { path: string; query: URLSearchParams } {
  const read = () => ({
    path: window.location.pathname,
    query: new URLSearchParams(window.location.search),
  });
  const [location, setLocation] = useState(read);
  useEffect(() => {
    const update = () => setLocation(read());
    window.addEventListener("popstate", update);
    window.addEventListener(CHANGE, update);
    return () => {
      window.removeEventListener("popstate", update);
      window.removeEventListener(CHANGE, update);
    };
  }, []);
  return location;
}

/** True for a plain click: modified clicks keep the browser's own behavior. */
export function plainClick(e: MouseEvent<HTMLAnchorElement>): boolean {
  return !(e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0);
}

export function Link({
  to,
  children,
  className = "oik-link",
  current,
}: {
  to: string;
  children: ReactNode;
  className?: string;
  current?: boolean;
}) {
  const onClick = (e: MouseEvent<HTMLAnchorElement>) => {
    if (!plainClick(e)) return;
    e.preventDefault();
    navigate(to);
  };
  return (
    <a href={to} className={className} onClick={onClick} aria-current={current ? "page" : undefined}>
      {children}
    </a>
  );
}
