// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The few glyphs the apps use, drawn on a 24px grid with 1.75px strokes.

import type { ReactElement } from "react";

export type IconName =
  | "activity"
  | "ai"
  | "back"
  | "check"
  | "chevron"
  | "close"
  | "collection"
  | "computer"
  | "copy"
  | "cpu"
  | "download"
  | "drag"
  | "drive"
  | "error"
  | "folder"
  | "home"
  | "key"
  | "lock"
  | "mark"
  | "minus"
  | "more"
  | "nas"
  | "note"
  | "person-plus"
  | "photo"
  | "plugin"
  | "search"
  | "plus"
  | "sliders"
  | "table"
  | "terminal"
  | "updown"
  | "globe"
  | "sync"
  | "up"
  | "warning";

const PATHS: Record<IconName, ReactElement> = {
  activity: <path d="M3 12h4l3-7 4 14 3-7h4" />,
  ai: <path d="M12 3.5 13.9 9l5.6 1.9-5.6 1.9L12 18.5l-1.9-5.7L4.5 10.9 10.1 9z" />,
  drive: <path d="M7 18.5h10a4 4 0 0 0 .6-7.95A6 6 0 0 0 6.1 11.6 3.5 3.5 0 0 0 7 18.5z" />,
  folder: <path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v7.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />,
  globe: (
    <>
      <circle cx="12" cy="12" r="8" />
      <path d="M4 12h16M12 4c2.5 2.6 2.5 13.4 0 16M12 4c-2.5 2.6-2.5 13.4 0 16" />
    </>
  ),
  home: <path d="M4 10.75 12 4.5l8 6.25V18a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18z" />,
  nas: (
    <>
      <rect x="6" y="3.5" width="12" height="17" rx="2" />
      <path d="M9 8h6M9 11h6" />
      <circle cx="12" cy="16" r="1" fill="currentColor" stroke="none" />
    </>
  ),
  note: (
    <>
      <path d="M6.5 4h11A1.5 1.5 0 0 1 19 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 5 18.5v-13A1.5 1.5 0 0 1 6.5 4z" />
      <path d="M8.5 9h7M8.5 12.5h7M8.5 16h4" />
    </>
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="m20 20-4-4" />
    </>
  ),
  photo: (
    <>
      <rect x="3.5" y="5" width="17" height="14" rx="2" />
      <circle cx="9" cy="10" r="1.75" />
      <path d="m4 17 5-4.5 3.5 3 3-2.5 4.5 4" />
    </>
  ),
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
  // A chip with its pins: a computer's processor.
  cpu: (
    <>
      <rect x="6.5" y="6.5" width="11" height="11" rx="1.5" />
      <rect x="9.5" y="9.5" width="5" height="5" rx="0.5" />
      <path d="M9.5 3.5v3M14.5 3.5v3M9.5 17.5v3M14.5 17.5v3M3.5 9.5h3M3.5 14.5h3M17.5 9.5h3M17.5 14.5h3" />
    </>
  ),
  download: <path d="M12 4v11m-5-5l5 5 5-5M5 19.5h14" />,
  // Six dots: hold and move.
  drag: (
    <>
      <circle cx="9" cy="7" r="1" fill="currentColor" />
      <circle cx="15" cy="7" r="1" fill="currentColor" />
      <circle cx="9" cy="12" r="1" fill="currentColor" />
      <circle cx="15" cy="12" r="1" fill="currentColor" />
      <circle cx="9" cy="17" r="1" fill="currentColor" />
      <circle cx="15" cy="17" r="1" fill="currentColor" />
    </>
  ),
  minus: <path d="M6 12h12" />,
  // A padlock: closed off.
  lock: (
    <>
      <rect x="5.5" y="10.5" width="13" height="9.5" rx="2" />
      <path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5" />
    </>
  ),
  // A plug: something that fits in.
  plugin: <path d="M9 3.5v4M15 3.5v4M6.5 7.5h11v3a5.5 5.5 0 0 1-11 0zM12 16v4.5" />,
  up: <path d="M6 15l6-6 6 6" />,
  sync: <path d="M19.5 12a7.5 7.5 0 0 1-13 5.1M4.5 12a7.5 7.5 0 0 1 13-5.1M17.5 3.5v3.4h-3.4M6.5 20.5v-3.4h3.4" />,
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
  // The Oikome logo: a rounded square open at the foot like a door, with a
  // dot inside. Traced from the project's logo (460px, square 115-345, 30px
  // stroke, dot r 34), scaled to this 24px grid.
  mark: (
    <>
      <path d="M10.4 18H8.6A2.6 2.6 0 0 1 6 15.4V8.6A2.6 2.6 0 0 1 8.6 6h6.8A2.6 2.6 0 0 1 18 8.6v6.8a2.6 2.6 0 0 1-2.6 2.6h-1.8" />
      <circle cx="12" cy="12" r="1.8" fill="currentColor" stroke="none" />
    </>
  ),
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
