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

// The web app (M1c exit criteria), in Chromium against a real local node:
// `oikome serve --web --web-root apps/web/dist` on an empty data directory.
// First run (name, recovery key), a collection filled through the SQL
// workbench and browsed, a share with a contact (a second node) that is
// then stopped, the contact's reads and refusal in Activity, and every
// screen at desktop and phone width without sideways scrolling and with
// readable text (WCAG AA). No fake data: everything goes through the node.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import { auditContrast } from "./contrast.mjs";
import { BIN, ENV, nodeSql, oikome } from "./node.mjs";

const WEB_ROOT = fileURLToPath(new URL("../apps/web/dist", import.meta.url));

test.describe.configure({ mode: "serial", timeout: 120_000 });

let owner;
let contact;
let server;
let base;
let context;
let page;

const loginLink = () => readFileSync(join(owner, ".oikome", "web", "login"), "utf8").trim();

/** Call the node's API from the page, as the app does. */
const api = (op, body = {}) =>
  page.evaluate(
    async ([op, body]) => {
      const res = await fetch(`/api/${op}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      return res.json();
    },
    [op, body],
  );

test.beforeAll(async ({ browser }) => {
  expect(existsSync(BIN), `${BIN} (cargo build -p oikome-server)`).toBe(true);
  expect(existsSync(join(WEB_ROOT, "index.html")), "build apps/web first (npm run build)").toBe(true);
  owner = mkdtempSync(join(tmpdir(), "oikome-app-"));
  contact = mkdtempSync(join(tmpdir(), "oikome-app-contact-"));
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
      "--web-root",
      WEB_ROOT,
    ],
    { stdio: ["ignore", "inherit", "inherit"], env: ENV },
  );
  for (let i = 0; !existsSync(join(owner, ".oikome", "web", "login")); i++) {
    if (i === 200 || server.exitCode !== null) throw new Error("the node did not start its web app");
    await new Promise((r) => setTimeout(r, 100));
  }
  base = new URL(loginLink()).origin;
  context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  page = await context.newPage();
  page.on("pageerror", (e) => console.error(`[page] ${e.message}`));
});

test.afterAll(async () => {
  await context?.close();
  server?.kill();
  for (const dir of [owner, contact]) if (dir) rmSync(dir, { recursive: true, force: true });
});

test("first run: name the data center and save its recovery key", async () => {
  await page.goto(loginLink());
  await expect(page.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
  await page.getByLabel("Name").fill("Home");
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(page.getByRole("heading", { name: "Save Your Recovery Key" })).toBeVisible();
  const key = page.locator(".oik-copy__value").first();
  await expect(key).toHaveText(/^([0-9A-Z]{4}-){13}[0-9A-Z]{4}$/);
  const continueButton = page.getByRole("button", { name: "Continue" });
  await expect(continueButton).toBeDisabled();
  await page.getByLabel("I've stored it somewhere safe").check();
  await continueButton.click();

  await expect(page.getByRole("heading", { name: "You're All Set" })).toBeVisible();
  await page.getByRole("button", { name: "Go to Collections" }).click();
  await expect(page.getByRole("heading", { name: "Collections", level: 1 })).toBeVisible();
  await expect(page.getByText("No collections yet")).toBeVisible();
  await expect(page.getByText("Back up your recovery key")).toHaveCount(0);
  await expect(page.locator(".screen-brand__subtitle")).toHaveText("Home");
});

test("a collection is filled in the SQL workbench and browsed", async () => {
  await page.getByRole("button", { name: "New Collection" }).first().click();
  await page.getByLabel("Name").fill("notes");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page).toHaveURL(/\/collections\/notes$/);
  await expect(page.getByText("Nothing in notes yet")).toBeVisible();

  await page.getByRole("button", { name: "Open Advanced" }).click();
  await expect(page.getByRole("heading", { name: "SQL Workbench" })).toBeVisible();
  await page
    .getByLabel("Statements")
    .fill(
      "CREATE TABLE notes.memos (id BIGINT NOT NULL, body VARCHAR); " +
        "INSERT INTO notes.memos VALUES (1, 'buy milk'), (2, 'call mum')",
    );
  await page.getByRole("button", { name: "Run" }).click();
  await expect(page.getByRole("status").filter({ hasText: "Done." })).toBeVisible();

  // Back returns to the collection the workbench was opened from.
  await page.getByRole("link", { name: "notes" }).click();
  await expect(page).toHaveURL(/\/collections\/notes$/);
  await page.getByRole("link", { name: "Collections" }).click();
  await page.getByRole("link", { name: /^notes/ }).click();
  const rows = page.getByRole("table", { name: "memos" });
  await expect(rows.getByRole("cell", { name: "buy milk" })).toBeVisible();
  await expect(rows.getByRole("cell", { name: "call mum" })).toBeVisible();
  await expect(page.getByText("Rows 1–2")).toBeVisible();
});

test("a contact reads a shared collection until the share is stopped", async () => {
  const did = oikome(contact, "identity").trim();
  await page.getByRole("link", { name: "Sharing" }).click();
  await page.getByRole("button", { name: "Add Contact" }).click();
  await page.getByLabel("Name").fill("Bob");
  await page.getByLabel("Their ID").fill(did);
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByText("Nothing shared").first()).toBeVisible();

  await page.getByRole("button", { name: "New Share" }).click();
  await page.getByRole("button", { name: "Share notes with Bob" }).click();
  await expect(page.getByRole("heading", { name: "Send this invite code to Bob" })).toBeVisible();
  const token = (await page.locator(".oik-inset-well .oik-copy__value").textContent()).trim();
  await page.getByRole("button", { name: "Done" }).click();
  await expect(page.getByText("Bob can view live")).toBeVisible();

  const { flightUrl } = await api("node_info");
  nodeSql(contact, `CREATE SPACE pub WITH PEER = '${flightUrl}', REMOTE = 'notes', TOKEN = '${token}'`);
  expect(nodeSql(contact, "SELECT body FROM pub.memos ORDER BY id")).toEqual([
    ["buy milk"],
    ["call mum"],
  ]);

  await page.getByRole("button", { name: "Stop Sharing: notes, Bob" }).click();
  await expect(page.getByText("Bob · Stopped")).toBeVisible();
  let refused;
  try {
    nodeSql(contact, "SELECT body FROM pub.memos");
  } catch (e) {
    refused = e;
  }
  expect(String(refused?.stderr)).toMatch(/permission denied/);
});

test("Activity shows the contact's reads and the refusal", async () => {
  await page.getByRole("link", { name: "Activity" }).click();
  await expect(async () => {
    await page.reload();
    await expect(page.getByText("Bob read notes").first()).toBeVisible({ timeout: 1000 });
    await expect(page.getByText("Bob was turned away from notes").first()).toBeVisible({
      timeout: 1000,
    });
  }).toPass({ timeout: 20_000 });
});

test("screens use words for people, also when something fails", async () => {
  // Engine words never reach the screens (PRD success criterion); "SQL" only
  // names the workbench under Settings > Advanced.
  const ENGINE = /\b(space|spaces|ucan|did|token|principal)\b/i;

  await page.goto(`${base}/collections`);
  await page.getByRole("button", { name: "New Collection" }).first().click();
  await page.getByLabel("Name").fill("bad name!");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("alert")).toHaveText(/^Use only letters, numbers/);
  await page.getByLabel("Name").fill("notes");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("alert")).toHaveText("You already have a collection named notes.");
  await page.getByRole("button", { name: "Cancel" }).click();

  await page.goto(`${base}/sharing`);
  await page.getByRole("button", { name: "Add Contact" }).click();
  await page.getByLabel("Name").fill("Nobody");
  await page.getByLabel("Their ID").fill("hello");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByRole("alert")).toHaveText(/^That isn't an Oikome ID/);
  await page.getByLabel("Their ID").fill((await api("node_info")).did);
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByRole("alert")).toHaveText(/^That's your own ID/);
  await page.getByRole("button", { name: "Cancel" }).last().click();

  for (const path of ["/onboarding", "/collections/notes", "/sharing?share=notes", "/activity", "/settings"]) {
    await page.goto(`${base}${path}`);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    const text = await page.locator("body").innerText();
    expect(text, path).not.toMatch(ENGINE);
    if (path !== "/settings") expect(text, path).not.toMatch(/\bsql\b/i);
  }
});

test("a collection is removed from its More menu with the keyboard", async () => {
  await page.goto(`${base}/collections`);
  await page.getByRole("button", { name: "New Collection" }).first().click();
  await page.getByLabel("Name").fill("scratch");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page).toHaveURL(/\/collections\/scratch$/);

  const more = page.getByRole("button", { name: "More actions for scratch" });
  await more.focus();
  await page.keyboard.press("Enter");
  const remove = page.getByRole("menuitem", { name: "Remove Collection" });
  await expect(remove).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(remove).toHaveCount(0);
  await expect(more).toBeFocused();

  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await expect(page.getByText("Remove scratch? Sharing it stops.")).toBeVisible();
  await page.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(page).toHaveURL(/\/collections$/);
  await expect(page.getByRole("link", { name: /^scratch/ })).toHaveCount(0);
  // The removed collection's page is not in the history any more.
  await page.goBack();
  await expect(page).not.toHaveURL(/scratch/);
});

test("Back returns to the page before, and closed panels stay closed", async () => {
  // "/" forwards to Collections without trapping Back.
  await page.goto(`${base}/activity`);
  await page.goto(`${base}/`);
  await expect(page).toHaveURL(/\/collections$/);
  await page.goBack();
  await expect(page).toHaveURL(/\/activity$/);

  // A new page starts at the top.
  await page.setViewportSize({ width: 390, height: 600 });
  await page.goto(`${base}/sharing`);
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await page.getByRole("link", { name: "Activity" }).click();
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await page.setViewportSize({ width: 1280, height: 800 });

  // Share… opens the panel; closing it drops ?share, and Back returns to the collection.
  await page.goto(`${base}/collections/notes`);
  await page.getByRole("button", { name: "Share…" }).click();
  await expect(page).toHaveURL(/\/sharing\?share=notes$/);
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page).toHaveURL(/\/sharing$/);
  await page.reload();
  await expect(page.getByRole("complementary", { name: "New Share" })).toHaveCount(0);
  await page.goBack();
  await expect(page).toHaveURL(/\/collections\/notes$/);

  // The workbench opened directly goes up to Settings.
  await page.goto(`${base}/settings/advanced`);
  await page.getByRole("link", { name: "Settings" }).last().click();
  await expect(page).toHaveURL(/\/settings$/);
});

test("every screen fits desktop and phone widths, and its text is readable", async () => {
  const screens = [
    ["/onboarding", "Save Your Recovery Key"],
    ["/collections/notes", "Collections"],
    ["/sharing", "Sharing"],
    ["/sharing?share=notes", "Sharing"],
    ["/activity", "Activity"],
    ["/settings", "Settings"],
    ["/settings/advanced", "SQL Workbench"],
  ];
  for (const [width, height] of [
    [1280, 800],
    [390, 844],
  ]) {
    await page.setViewportSize({ width, height });
    for (const [path, heading] of screens) {
      await page.goto(`${base}${path}`);
      await expect(page.getByRole("heading", { name: heading, level: 1 })).toBeVisible();
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - window.innerWidth,
      );
      expect(overflow, `${path} at ${width}px scrolls sideways`).toBeLessThanOrEqual(0);
      expect(await page.evaluate(auditContrast), `${path} at ${width}px`).toEqual([]);
    }
  }
});
