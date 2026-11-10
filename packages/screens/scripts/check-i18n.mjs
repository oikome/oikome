// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Every language's words against English (src/i18n/en.ts). TypeScript
// already checks that each catalog has every key and that each function
// takes the same values; this finds what types can't: words left in
// English, a function copied unchanged, and a value (a name, a count) a
// translation forgot to say. British English is English with a few words
// changed, so only its spelling is checked.
//
//   node packages/screens/scripts/check-i18n.mjs

import { build } from "esbuild";
import { fileURLToPath } from "node:url";

const I18N = fileURLToPath(new URL("../src/i18n", import.meta.url));
const LANGUAGES = { de: "de", fr: "fr", ja: "ja", ko: "ko", ru: "ru", zhHans: "zh-Hans", zhHant: "zh-Hant" };

// Words that are the same in every language: names of the product, of
// things people type or see as they are, and of the two styles.
const SAME = new Set([
  "Oikome",
  "oikome-recovery.key",
  "Claude Code",
  "MCP",
  "NAS",
  "Ink",
  "Bold",
  "WebDAV",
  "RustFS",
  "Google Drive",
  "iCloud Drive",
  "Google Drive, Dropbox, OneDrive, iCloud Drive",
  "CSV",
  "OK",
  "flomo",
]);

// German and French share many words with English (Plugins, Status,
// Version): there only a phrase of several words left as it is counts.
const LATIN = new Set(["de", "fr"]);

/** The words a function writes, outside the values it is given. */
function wordsOf(fn) {
  const literals = fn.toString().match(/`(?:[^`\\]|\\.)*`|"(?:[^"\\]|\\.)*"/g) ?? [];
  return literals.map((l) => l.slice(1, -1).replace(/\$\{[^}]*\}/g, " ")).join(" ");
}

const imports = [
  `export { en } from "./en";`,
  `export { enGB } from "./en-GB";`,
  ...Object.keys(LANGUAGES).map((v) => `export { ${v} } from "./${LANGUAGES[v]}";`),
].join("\n");
const bundle = await build({
  stdin: { contents: imports, resolveDir: I18N, loader: "ts" },
  bundle: true,
  format: "esm",
  platform: "neutral",
  charset: "utf8",
  write: false,
  logLevel: "error",
});
const catalogs = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`);

/** The parameters of a function, as written: `(name, n) =>` or `name =>`. */
function params(fn) {
  const text = fn.toString();
  const m = /^\s*(?:function\s*\w*\s*)?\(([^)]*)\)|^\s*(\w+)\s*=>/.exec(text);
  const list = m ? (m[1] ?? m[2] ?? "") : "";
  return list
    .split(",")
    .map((p) => p.trim().replace(/[=:].*$/, "").replace(/^\{|\}$/g, "").trim())
    .flatMap((p) => p.split(/\s*,\s*/))
    .filter((p) => /^\w+$/.test(p));
}

function body(fn) {
  const text = fn.toString();
  const arrow = text.indexOf("=>");
  return arrow >= 0 ? text.slice(arrow + 2) : text;
}

const uses = (text, name) => new RegExp(`\\b${name}\\b`).test(text);

/** Each leaf of a catalog with its key path. */
function* leaves(value, path = []) {
  if (typeof value === "object" && value !== null && !Array.isArray(value)) {
    for (const [key, v] of Object.entries(value)) yield* leaves(v, [...path, key]);
  } else {
    yield [path.join("."), value];
  }
}

const english = new Map(leaves(catalogs.en));
const problems = [];
for (const [variable, code] of Object.entries(LANGUAGES)) {
  for (const [key, value] of leaves(catalogs[variable])) {
    const en = english.get(key);
    if (typeof value === "string") {
      const english = LATIN.has(code) ? /[A-Za-z]+\s+[A-Za-z]+/.test(value) : /[A-Za-z]/.test(value);
      if (value === en && english && !SAME.has(value)) problems.push(`${code} ${key}: still English: ${JSON.stringify(value)}`);
    } else if (typeof value === "function" && typeof en === "function") {
      // The same function is right when it writes no words of its own ("${day}, ${time}").
      const words = wordsOf(en);
      const english = LATIN.has(code) ? /[A-Za-z]+\s+[A-Za-z]+/.test(words) : /[A-Za-z]{2,}/.test(words);
      if (value.toString() === en.toString() && english) {
        problems.push(`${code} ${key}: the English function, unchanged`);
      }
      // Parameters are named as in English; a value English says should be said.
      const said = body(en);
      for (const name of params(en)) {
        if (uses(said, name) && !uses(body(value), name)) problems.push(`${code} ${key}: never says ${name}`);
      }
    }
  }
}
for (const [key, value] of leaves(catalogs.enGB)) {
  if (typeof value === "string" && /\bdata center\b|\bData Center\b/.test(value)) problems.push(`en-GB ${key}: American spelling`);
}

if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n${problems.length} to look at.`);
  process.exit(1);
}
console.log(`Every language says everything English says (${english.size} keys, ${Object.keys(LANGUAGES).length + 1} languages).`);
