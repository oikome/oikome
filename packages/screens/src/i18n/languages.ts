// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The languages the app is written in, and the one in use. Like the style,
// it is a choice of this device: kept in this browser's storage, and set
// before the first paint. With no choice made, it is the browser's first
// language Oikome is written in, else English. English is always here;
// another language's words load when it is picked.

import { useSyncExternalStore } from "react";
import { en } from "./en";
import type { Strings } from "./en";
import { fillStrings } from "../strings";

export type Language = "en" | "en-GB" | "de" | "fr" | "ja" | "ko" | "ru" | "zh-Hans" | "zh-Hant";

/** Each language in its own words, as a language menu lists them. */
export const LANGUAGES: { code: Language; name: string }[] = [
  { code: "en", name: "English (US)" },
  { code: "en-GB", name: "English (UK)" },
  { code: "de", name: "Deutsch" },
  { code: "fr", name: "Français" },
  { code: "ru", name: "Русский" },
  { code: "ja", name: "日本語" },
  { code: "ko", name: "한국어" },
  { code: "zh-Hans", name: "中文（简体）" },
  { code: "zh-Hant", name: "中文（繁體）" },
];

const CATALOGS: Record<Exclude<Language, "en">, () => Promise<Strings>> = {
  "en-GB": () => import("./en-GB").then((m) => m.enGB),
  de: () => import("./de").then((m) => m.de),
  fr: () => import("./fr").then((m) => m.fr),
  ja: () => import("./ja").then((m) => m.ja),
  ko: () => import("./ko").then((m) => m.ko),
  ru: () => import("./ru").then((m) => m.ru),
  "zh-Hans": () => import("./zh-Hans").then((m) => m.zhHans),
  "zh-Hant": () => import("./zh-Hant").then((m) => m.zhHant),
};

const KEY = "oikome.language";

let current: Language = "en";
const listeners = new Set<() => void>();

function isLanguage(value: unknown): value is Language {
  return LANGUAGES.some((l) => l.code === value);
}

/** The language in use. */
export function language(): Language {
  return current;
}

/** The language in use as `Intl` takes it. */
export function locale(): string {
  return current === "en" ? "en-US" : current;
}

/** The language picked on this device, if one was. */
export function savedLanguage(): Language | undefined {
  try {
    const saved = localStorage.getItem(KEY);
    return isLanguage(saved) ? saved : undefined;
  } catch {
    return undefined;
  }
}

/** Where English is written the British way. */
const BRITISH = new Set(["GB", "IE", "AU", "NZ", "IN", "ZA", "SG", "HK", "MY", "MT", "CY", "PK", "LK", "NG", "KE"]);
const BRITISH_ZONES =
  /^(Europe|Africa|Australia|Indian)\/|^Pacific\/(Auckland|Chatham|Fiji|Port_Moresby)$|^Atlantic\/(Bermuda|Stanley|St_Helena)$|^Asia\/(Kolkata|Calcutta|Karachi|Dhaka|Colombo|Kathmandu|Singapore|Hong_Kong|Kuala_Lumpur|Brunei|Dubai|Qatar|Bahrain|Yangon)$/;

/** English, British in places that write it so (by this computer's time zone). */
function englishHere(): Language {
  let zone = "";
  try {
    zone = Intl.DateTimeFormat().resolvedOptions().timeZone ?? "";
  } catch {
    // No zone: American.
  }
  return BRITISH_ZONES.test(zone) ? "en-GB" : "en";
}

/** The language of `tag` ("zh-TW", "de-AT") the app is written in, if any. */
export function languageOf(tag: string): Language | undefined {
  let given: Intl.Locale;
  try {
    given = new Intl.Locale(tag);
  } catch {
    return undefined;
  }
  switch (given.language) {
    case "en":
      if (!given.region) return englishHere();
      return BRITISH.has(given.region) ? "en-GB" : "en";
    case "zh":
      return given.maximize().script === "Hant" ? "zh-Hant" : "zh-Hans";
    case "de":
    case "fr":
    case "ja":
    case "ko":
    case "ru":
      return given.language;
    default:
      return undefined;
  }
}

/** The browser's first language the app is written in, else English. */
export function browserLanguage(): Language {
  const tags = typeof navigator === "undefined" ? [] : navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of tags) {
    const found = tag ? languageOf(tag) : undefined;
    if (found) return found;
  }
  return englishHere();
}

/**
 * Use `code`: load its words, put them in place, and draw the app again.
 * `remember` keeps the choice on this device (a pick in Settings, not the
 * browser's language at start).
 */
export async function setLanguage(code: Language, remember = true): Promise<void> {
  const catalog = code === "en" ? en : await CATALOGS[code]();
  fillStrings(catalog);
  current = code;
  if (typeof document !== "undefined") document.documentElement.lang = code;
  if (remember) {
    try {
      localStorage.setItem(KEY, code);
    } catch {
      // A private window keeps it for this page only.
    }
  }
  for (const listener of listeners) listener();
}

/** Before the first paint: the language picked here, else the browser's. */
export function startLanguage(): Promise<void> {
  return setLanguage(savedLanguage() ?? browserLanguage(), false).catch(() => setLanguage("en", false));
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The language in use; a component using it is drawn again when it changes. */
export function useLanguage(): Language {
  return useSyncExternalStore(subscribe, language, language);
}
