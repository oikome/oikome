// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// What a language's catalog uses to write numbers and plurals its own way.

type Forms = Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };

/** The form for `n` by the language's plural rules: `{ one, few, many, other }` as it has them. */
export function plural(locale: string): (n: number, forms: Forms) => string {
  const rules = new Intl.PluralRules(locale);
  return (n, forms) => forms[rules.select(n)] ?? forms.other;
}

/** A whole number as the language writes it: 12,345 / 12.345 / 12 345. */
export function count(locale: string): (n: number) => string {
  const format = new Intl.NumberFormat(locale);
  return (n) => format.format(n);
}

type Changes<T> = {
  [K in keyof T]?: T[K] extends (...args: never[]) => unknown
    ? T[K]
    : T[K] extends object
      ? Changes<T[K]>
      : T[K];
};

/** `base` with some of its words changed: for a variety of a language (British English). */
export function overlay<T>(base: T, changes: Changes<T>): T {
  const out: Record<string, unknown> = { ...(base as Record<string, unknown>) };
  for (const [key, value] of Object.entries(changes as Record<string, unknown>)) {
    const current = out[key];
    out[key] =
      typeof value === "object" && value !== null && typeof current === "object" && current !== null
        ? overlay(current, value as Changes<typeof current>)
        : value;
  }
  return out as T;
}
