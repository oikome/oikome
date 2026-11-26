// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// `npm run oiql`: Oikome SQL against the local acceptance data center
// (`npm run acceptance`), as its owner, through the same session the web
// app's charts use. With no statement it is a prompt (oiql>); `-` reads the
// statements from standard input (`pbpaste | npm run oiql -- -` runs what
// Show SQL copied); else it runs the statements given.

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { BIN, ENV, EXTENSIONS } from "./node.mjs";

const generated = resolve("tests", ".generated", "acceptance");
const root = join(generated, "node");
if (!existsSync(join(root, ".oikome"))) {
  console.error("There is no local data center yet: start one with `npm run acceptance`.");
  process.exit(1);
}
if (!existsSync(BIN)) {
  console.error(`There is no ${BIN} yet: build it with \`cargo node\`.`);
  process.exit(1);
}
const run = spawnSync(BIN, ["--root", root, "--extension-dir", EXTENSIONS, "sql", ...process.argv.slice(2)], {
  stdio: "inherit",
  env: { ...ENV, HOME: join(generated, "home") },
});
process.exit(run.status ?? 1);
