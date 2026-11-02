// Copyright 2025 The Oikome Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// The owner's web API (M1b exit criteria), on real local nodes: the test
// starts `oikome serve --web` on an empty data directory, logs in with the
// one-time link, and through @oikome/client creates a Space, writes and
// reads rows, shares the Space with a contact (a second node) and revokes
// it, after which the contact's query fails. The access log shows both.
// Requests without the session, from another origin or for another host are
// refused. No fake data: everything goes through the node.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { request as httpRequest } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { HttpClient } from "@oikome/client";
import { expect, test } from "@playwright/test";
import { BIN, ENV, nodeSql, oikome } from "./node.mjs";

test.describe.configure({ mode: "serial", timeout: 120_000 });

let owner;
let contact;
let server;
let firstLink;
let base;
let cookie;
let client;
let contactDid;
let share;

const loginFile = () => join(owner, ".oikome", "web", "login");

/** fetch as the web app's page sends it: its origin and its session cookie. */
function pageFetch(input, init = {}) {
  const headers = new Headers(init.headers);
  headers.set("origin", base);
  if (cookie) headers.set("cookie", cookie);
  return fetch(input, { ...init, headers });
}

/** A raw request, for headers fetch will not let a test set (Host). */
function rawPost(path, headers) {
  return new Promise((resolve, reject) => {
    const url = new URL(base);
    const req = httpRequest(
      { host: url.hostname, port: url.port, path, method: "POST", headers },
      (res) => {
        res.resume();
        resolve(res.statusCode);
      },
    );
    req.on("error", reject);
    req.end("{}");
  });
}

test.beforeAll(async () => {
  expect(existsSync(BIN), `${BIN} (cargo build -p oikome-server)`).toBe(true);
  owner = mkdtempSync(join(tmpdir(), "oikome-owner-"));
  contact = mkdtempSync(join(tmpdir(), "oikome-contact-"));
  server = spawn(
    BIN,
    [
      "--root",
      owner,
      "serve",
      "--listen",
      "127.0.0.1:0",
      "--p2p-listen",
      "/ip4/127.0.0.1/tcp/0",
      "--no-mdns",
      "--web",
      "--web-listen",
      "127.0.0.1:0",
    ],
    { stdio: ["ignore", "inherit", "inherit"], env: ENV },
  );
  for (let i = 0; !existsSync(loginFile()); i++) {
    if (i === 200 || server.exitCode !== null) throw new Error("the node did not start its web API");
    await new Promise((r) => setTimeout(r, 100));
  }
  firstLink = readFileSync(loginFile(), "utf8").trim();
  base = new URL(firstLink).origin;
  const login = await fetch(firstLink, { redirect: "manual" });
  expect(login.status).toBe(303);
  expect(login.headers.get("location")).toBe("/");
  cookie = login.headers.get("set-cookie").split(";")[0];
  expect(cookie).toMatch(/^oikome_session_\d+=/);
  client = new HttpClient({ baseUrl: base, fetch: pageFetch });
});

test.afterAll(() => {
  server?.kill();
  for (const dir of [owner, contact]) if (dir) rmSync(dir, { recursive: true, force: true });
});

test("the owner creates a space, writes rows and reads them back", async () => {
  const info = await client.nodeInfo();
  expect(info.did).toMatch(/^did:key:z/);
  expect(info.flightUrl).toMatch(/^https:\/\/127\.0\.0\.1:\d+$/);

  const space = await client.createSpace({ name: "notes" });
  expect(space).toMatchObject({
    name: "notes",
    kind: "analytical",
    sharedWith: 0,
    storage: null,
    example: false,
  });
  const written = await client.runSql({
    sql:
      "CREATE TABLE notes.memos (id BIGINT NOT NULL, body VARCHAR); " +
      "INSERT INTO notes.memos VALUES (1, 'buy milk'), (2, 'call mum')",
  });
  expect(written.kind).toBe("done");

  expect(await client.listTables({ space: "notes" })).toEqual([
    { name: "memos", kind: "table", rows: 2 },
  ]);
  const page = await client.readRows({ space: "notes", table: "memos" });
  expect(page.truncated).toBe(false);
  expect(page.table.toArray().map((row) => row.body).sort()).toEqual(["buy milk", "call mum"]);

  const counted = await client.runSql({ sql: "SELECT count(*) AS n FROM notes.memos" });
  expect(counted.kind).toBe("rows");
  expect(Number(counted.table.get(0).n)).toBe(2);
});

test("a contact reads a shared space until the share is revoked", async () => {
  contactDid = oikome(contact, "identity").trim();
  await client.addContact({ did: contactDid, name: "Bob" });
  share = await client.shareSpace({ space: "notes", did: contactDid });
  expect(share.access).toBe("query");
  expect((await client.listSpaces())[0].sharedWith).toBe(1);

  const { flightUrl } = await client.nodeInfo();
  // The contact's node mounts the Space with the token and queries it there.
  nodeSql(
    contact,
    `CREATE SPACE pub WITH PEER = '${flightUrl}', REMOTE = 'notes', TOKEN = '${share.token}'`,
  );
  expect(nodeSql(contact, "SELECT body FROM pub.memos ORDER BY id")).toEqual([
    ["buy milk"],
    ["call mum"],
  ]);

  const revoked = await client.revokeShare({ id: share.id });
  expect(revoked.grantRemoved).toBe(true);
  let refused;
  try {
    nodeSql(contact, "SELECT body FROM pub.memos");
  } catch (e) {
    refused = e;
  }
  expect(String(refused?.stderr)).toMatch(/permission denied/);
  const [listed] = await client.listShares({ space: "notes" });
  expect(listed).toMatchObject({ id: share.id, status: "revoked", contactName: "Bob" });
});

test("the access log shows the contact's reads and the refusal", async () => {
  await expect
    .poll(async () => (await client.listAccess({ did: contactDid })).map((i) => i.outcome), {
      timeout: 15_000,
    })
    .toEqual(expect.arrayContaining(["allowed", "denied"]));
  const items = await client.listAccess({ did: contactDid });
  expect(items.every((i) => i.contactName === "Bob")).toBe(true);
  expect(items.every((i) => i.spaces.includes("notes"))).toBe(true);
  expect(items.find((i) => i.outcome === "allowed").shareId).toBe(share.id);
});

test("requests without the session, from elsewhere or for another host are refused", async () => {
  const post = (headers) =>
    fetch(`${base}/api/list_spaces`, {
      method: "POST",
      headers: { "content-type": "application/json", ...headers },
      body: "{}",
    });
  expect((await post({ origin: base })).status).toBe(401);
  expect((await post({ origin: "http://evil.example", cookie })).status).toBe(403);
  expect((await post({ cookie })).status).toBe(403);
  expect((await post({ origin: base, cookie, "sec-fetch-site": "cross-site" })).status).toBe(403);
  // Another Host (DNS rebinding), even with a matching Origin.
  const port = new URL(base).port;
  expect(
    await rawPost("/api/list_spaces", {
      host: `attacker.example:${port}`,
      origin: `http://attacker.example:${port}`,
      "content-type": "application/json",
      cookie,
    }),
  ).toBe(403);
  // A login link works once.
  expect((await fetch(firstLink, { redirect: "manual" })).status).toBe(401);
  // The page itself still gets in.
  expect((await client.listSpaces()).map((s) => s.name)).toEqual(["notes"]);
});
