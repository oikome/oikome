// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The few glyphs the apps use, drawn on a 24px grid with 1.75px strokes.

import type { ReactElement } from "react";

export type IconName =
  | "activity"
  | "check"
  | "chevron"
  | "collection"
  | "copy"
  | "download"
  | "error"
  | "gear"
  | "key"
  | "people"
  | "plus"
  | "table"
  | "terminal"
  | "warning";

const PATHS: Record<IconName, ReactElement> = {
  activity: <path d="M3 12h4l3-7 4 14 3-7h4" />,
  check: <path d="M5 12.5l4.5 4.5L19 7.5" />,
  chevron: <path d="M9 5l7 7-7 7" />,
  collection: (
    <>
      <rect x="3.5" y="5" width="17" height="5" rx="1.5" />
      <rect x="3.5" y="12" width="17" height="7" rx="1.5" />
      <path d="M8 15.5h8" />
    </>
  ),
  copy: (
    <>
      <rect x="8.5" y="8.5" width="11" height="11" rx="2" />
      <path d="M15.5 8.5V6a1.5 1.5 0 0 0-1.5-1.5H6A1.5 1.5 0 0 0 4.5 6v8A1.5 1.5 0 0 0 6 15.5h2.5" />
    </>
  ),
  download: <path d="M12 4v11m-5-5l5 5 5-5M5 19.5h14" />,
  error: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5v5.5m0 3v.01" />
    </>
  ),
  gear: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3.5v2.5m0 12v2.5M3.5 12H6m12 0h2.5M6 6l1.8 1.8m8.4 8.4L18 18M6 18l1.8-1.8m8.4-8.4L18 6" />
    </>
  ),
  key: (
    <>
      <circle cx="8" cy="15" r="4" />
      <path d="M11 12l8-8m-3 3l2.5 2.5M14 9l2 2" />
    </>
  ),
  people: (
    <>
      <circle cx="9" cy="9" r="3.25" />
      <path d="M3.5 19c.6-3 2.8-4.75 5.5-4.75S13.9 16 14.5 19" />
      <circle cx="16.5" cy="9.5" r="2.5" />
      <path d="M16 14.3c2.3.2 3.9 1.7 4.5 4.2" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  table: (
    <>
      <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
      <path d="M3.5 9.5h17M3.5 14.5h17M9.5 9.5v10" />
    </>
  ),
  terminal: (
    <>
      <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
      <path d="M7.5 9.5l3 2.5-3 2.5M12.5 15h4" />
    </>
  ),
  warning: (
    <>
      <path d="M12 4l9 15.5H3L12 4z" />
      <path d="M12 10v4.5m0 2.5v.01" />
    </>
  ),
};

/** A glyph; decorative unless `label` is given. */
export function Icon({ name, label }: { name: IconName; label?: string }) {
  return (
    <svg
      className="oik-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      {PATHS[name]}
    </svg>
  );
}
