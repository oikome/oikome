// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Native Oikome nodes for the specs: the binary, the environment it runs in
// and owner SQL.
//
// OIKOME_BIN: the binary (default <target>/debug/oikome, where <target> is
// CARGO_TARGET_DIR or the repository's target/). It loads the prebuilt
// libduckdb cargo downloaded into <target>/duckdb-download.

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = fileURLToPath(new URL("..", import.meta.url));
const TARGET = resolve(REPO, process.env.CARGO_TARGET_DIR ?? "target");
export const BIN = process.env.OIKOME_BIN ?? join(TARGET, "debug", "oikome");

/** Directories under target/duckdb-download/<target>/<version> with libduckdb. */
function libDirs() {
  const root = join(TARGET, "duckdb-download");
  const subdirs = (dir) => (existsSync(dir) ? readdirSync(dir).map((d) => join(dir, d)) : []);
  return subdirs(root)
    .flatMap(subdirs)
    .filter((dir) => ["libduckdb.so", "libduckdb.dylib"].some((f) => existsSync(join(dir, f))));
}
const LIB_DIRS = libDirs().join(":");

/**
 * The environment of every `oikome` process: libduckdb on the library path,
 * no `OIKOME_*` variable of the caller's, so each node keeps its own key
 * (`OIKOME_NODE_KEY`) and settings, and credentials in each node's private
 * file, never in this computer's keychain.
 */
export const ENV = {
  ...Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("OIKOME_"))),
  OIKOME_SECRET_STORE: "file",
  LD_LIBRARY_PATH: [LIB_DIRS, process.env.LD_LIBRARY_PATH].filter(Boolean).join(":"),
  DYLD_LIBRARY_PATH: [LIB_DIRS, process.env.DYLD_LIBRARY_PATH].filter(Boolean).join(":"),
};

/** Run `oikome --root <root> <args>` and return its standard output. */
export function oikome(root, ...args) {
  return execFileSync(BIN, ["--root", root, ...args], {
    encoding: "utf8",
    env: ENV,
    stdio: ["ignore", "pipe", "pipe"],
  });
}

/** Run SQL as the owner of the node at `root`; rows as arrays of strings. */
export function nodeSql(root, text) {
  return oikome(root, "sql", "--raw", text)
    .split("\n")
    .filter((line) => line.length > 0)
    .map((line) => line.split("\t"));
}
