// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Every string the screens show, in the language in use. It is one object,
// filled in place when the language changes (see `i18n/languages.ts`), so a
// module's `const s = strings.home` stays current; the app is drawn again
// after a change. English is `i18n/en.ts`, and every other language follows
// its keys.

import { en } from "./i18n/en";
import type { Strings } from "./i18n/en";

export type { Strings };

type Plain = Record<string, unknown>;

function isPlain(value: unknown): value is Plain {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function copy<T>(value: T): T {
  if (!isPlain(value)) return value;
  const out: Plain = {};
  for (const [key, v] of Object.entries(value)) out[key] = copy(v);
  return out as T;
}

export const strings: Strings = copy(en);

/** Put `catalog`'s words in place of the ones in use, keeping each section's object. */
export function fillStrings(catalog: Strings) {
  const fill = (target: Plain, source: Plain) => {
    for (const [key, value] of Object.entries(source)) {
      const current = target[key];
      if (isPlain(value) && isPlain(current)) fill(current, value);
      else target[key] = copy(value);
    }
  };
  fill(strings as unknown as Plain, catalog as unknown as Plain);
}
