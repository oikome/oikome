// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// A few routes on the History API; the server answers every route with the
// app (paths without an extension get index.html).

import { useEffect, useState } from "react";
import type { MouseEvent, ReactNode } from "react";

const CHANGE = "oikome:navigate";

export function navigate(to: string) {
  if (to !== window.location.pathname + window.location.search) {
    window.history.pushState(null, "", to);
  }
  window.dispatchEvent(new Event(CHANGE));
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
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
    e.preventDefault();
    navigate(to);
  };
  return (
    <a href={to} className={className} onClick={onClick} aria-current={current ? "page" : undefined}>
      {children}
    </a>
  );
}
