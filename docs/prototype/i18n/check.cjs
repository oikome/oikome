// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// node i18n/check.cjs <lang>  —  compares i18n/<lang>.js with i18n/en.js.
// Reports keys missing (not for English variants) or extra, placeholders that differ, and plural forms the language needs (Intl.PluralRules).
const fs = require('fs');
const path = require('path');
const vm = require('vm');
const load = (lang) => {
  const ctx = { window: {} };
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, `${lang}.js`), 'utf8'), ctx);
  const dict = ctx.window.OIKOME_I18N && ctx.window.OIKOME_I18N[lang];
  if (!dict) throw new Error(`${lang}.js does not set window.OIKOME_I18N['${lang}']`);
  return dict;
};
const lang = process.argv[2];
// en-GB and other English variants only list the strings that differ; the rest falls back to en.js.
const overlay = /^en-/.test(lang);
const en = load('en'), tr = load(lang);
const base = (k) => k.split('#')[0];
const holes = (s) => [...s.matchAll(/\{(\w+)(?::lower)?\}/g)].map((m) => m[1]).sort().join(',');
const enBase = new Map();
for (const [k, v] of Object.entries(en)) { const b = base(k); if (!enBase.has(b)) enBase.set(b, { plural: k.includes('#'), holes: new Set() }); holes(v).split(',').filter(Boolean).forEach((h) => enBase.get(b).holes.add(h)); }
const cats = new Intl.PluralRules(lang).resolvedOptions().pluralCategories;
const problems = [];
const trBase = new Set(Object.keys(tr).map(base));
for (const [b, info] of enBase) {
  if (!trBase.has(b)) { if (!overlay) problems.push(`missing: ${b}`); continue; }
  if (info.plural) {
    for (const c of cats) if (!(`${b}#${c}` in tr) && !(`${b}#other` in tr)) problems.push(`plural ${b}: no #${c} or #other`);
    if (!(`${b}#other` in tr)) problems.push(`plural ${b}: #other is required`);
  }
  const keys = Object.keys(tr).filter((k) => base(k) === b);
  for (const k of keys) {
    const hs = holes(tr[k]).split(',').filter(Boolean);
    for (const h of hs) if (!info.holes.has(h)) problems.push(`${k}: unknown placeholder {${h}}`);
    const need = [...info.holes].filter((h) => h !== 'n' || !info.plural);
    for (const h of need) if (!hs.includes(h) && !(h === 'n' && info.plural)) problems.push(`${k}: placeholder {${h}} is missing`);
  }
}
for (const k of Object.keys(tr)) if (!enBase.has(base(k))) problems.push(`extra: ${k}`);
console.log(problems.length ? problems.join('\n') : `${lang}: OK (${Object.keys(tr).length} strings, plural forms ${cats.join('/')})`);
process.exit(problems.length ? 1 : 0);
