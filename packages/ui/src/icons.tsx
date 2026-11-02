// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The few glyphs the apps use, drawn on a 24px grid with 1.75px strokes.

import type { ReactElement } from "react";

export type IconName =
  | "activity"
  | "back"
  | "check"
  | "chevron"
  | "close"
  | "collection"
  | "computer"
  | "copy"
  | "download"
  | "error"
  | "key"
  | "mark"
  | "more"
  | "person-plus"
  | "plus"
  | "sliders"
  | "table"
  | "terminal"
  | "updown"
  | "warning";

const PATHS: Record<IconName, ReactElement> = {
  activity: <path d="M3 12h4l3-7 4 14 3-7h4" />,
  back: <path d="M15 5l-7 7 7 7" />,
  check: <path d="M5 12.5l4.5 4.5L19 7.5" />,
  chevron: <path d="M9 5l7 7-7 7" />,
  close: <path d="M6 6l12 12M18 6L6 18" />,
  collection: (
    <>
      <rect x="3.5" y="5" width="17" height="5" rx="1.5" />
      <rect x="3.5" y="12" width="17" height="7" rx="1.5" />
      <path d="M8 15.5h8" />
    </>
  ),
  computer: (
    <>
      <rect x="4" y="5" width="16" height="11" rx="1.75" />
      <path d="M2.5 19h19" />
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
  key: (
    <>
      <circle cx="8" cy="15" r="4" />
      <path d="M11 12l8-8m-3 3l2.5 2.5M14 9l2 2" />
    </>
  ),
  mark: <circle cx="12" cy="12" r="7" />,
  more: (
    <>
      <circle cx="6" cy="12" r="0.9" />
      <circle cx="12" cy="12" r="0.9" />
      <circle cx="18" cy="12" r="0.9" />
    </>
  ),
  "person-plus": (
    <>
      <circle cx="9" cy="8" r="3.5" />
      <path d="M3 20c0-3.3 2.7-6 6-6s6 2.7 6 6M18 8v6M15 11h6" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  sliders: (
    <>
      <path d="M4 7h9M17 7h3M4 17h3M11 17h9" />
      <circle cx="15" cy="7" r="2" />
      <circle cx="9" cy="17" r="2" />
    </>
  ),
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
  updown: <path d="M8 10l4-4 4 4M8 14l4 4 4-4" />,
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
