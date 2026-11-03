// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The style the owner picked under Settings > Appearance: Ink (the default)
// or Bold. It is a preference of this browser, kept in its storage, and is
// set on <html> before the first paint so the page never flashes the other.

export type Theme = "ink" | "bold";

const KEY = "oikome.theme";

export function savedTheme(): Theme {
  try {
    return localStorage.getItem(KEY) === "bold" ? "bold" : "ink";
  } catch {
    return "ink";
  }
}

export function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // A private window keeps it for this page only.
  }
}
