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
// First run (a name already filled in, the recovery key, the examples it
// leaves), the examples opened as photos, notes and a table, a collection of
// one's own, a share with a contact (a second data center, in a browser of
// its own) who adds the invite, reads the collection and leaves it after
// the share is stopped, the contact's reads and refusal in Activity, and
// every screen at desktop
// and phone width without sideways scrolling and with readable text (WCAG
// AA). No fake data: everything goes through the node.

import { spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import { auditContrast } from "./contrast.mjs";
import { BIN, ENV, oikome } from "./node.mjs";

const WEB_ROOT = fileURLToPath(new URL("../apps/web/dist", import.meta.url));

test.describe.configure({ mode: "serial", timeout: 120_000 });

let owner;
let contact;
let server;
let contactServer;
let base;
let context;
let page;
let contactContext;
/** Bob's page, on his own data center, from the share test on. */
let bob;
let bobBase;

const loginLink = () => readFileSync(join(owner, ".oikome", "web", "login"), "utf8").trim();

/** Start the data center at `root` with its web app; resolves once it can be logged in to. */
async function serveWeb(root) {
  const child = spawn(
    BIN,
    [
      "--root",
      root,
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
  for (let i = 0; !existsSync(join(root, ".oikome", "web", "login")); i++) {
    if (i === 200 || child.exitCode !== null) throw new Error("the node did not start its web app");
    await new Promise((r) => setTimeout(r, 100));
  }
  return child;
}

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
  server = await serveWeb(owner);
  base = new URL(loginLink()).origin;
  context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  page = await context.newPage();
  page.on("pageerror", (e) => console.error(`[page] ${e.message}`));
});

test.afterAll(async () => {
  await context?.close();
  await contactContext?.close();
  server?.kill();
  contactServer?.kill();
  for (const dir of [owner, contact]) if (dir) rmSync(dir, { recursive: true, force: true });
});

test("first run: name the data center and save its recovery key", async () => {
  await page.goto(loginLink());
  await expect(page.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
  // A lazy Continue is a fine answer: the name is filled in already.
  // The storage choices also mention a user name: the field is "Name" exactly.
  const name = page.getByLabel("Name", { exact: true });
  await expect(name).toHaveValue("My data center");
  // Where collections are kept is chosen already: Local, on this computer.
  await expect(page.getByRole("radio", { name: /^Local/ })).toBeChecked();
  await name.fill("Home");
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(page.getByRole("heading", { name: "Save Your Recovery Key" })).toBeVisible();
  const key = page.locator(".oik-copy__value").first();
  await expect(key).toHaveText(/^([0-9A-Z]{4}-){13}[0-9A-Z]{4}$/);
  // One crimson action at a time: Download until the key is stored, then Continue.
  await expect(page.getByRole("button", { name: "Download Key File" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue" })).toHaveCount(0);
  await page.getByLabel("I've stored it somewhere safe").check();
  await page.getByRole("button", { name: "Continue" }).click();

  await expect(page.getByRole("heading", { name: "You're All Set" })).toBeVisible();
  await page.getByRole("button", { name: "Get Started" }).click();
  await expect(page).toHaveURL(`${base}/`);
  await expect(
    page.getByRole("heading", { name: /^Good (morning|afternoon|evening)$/, level: 1 }),
  ).toBeVisible();
  // The data center starts with the examples, once: they show on Home.
  await expect(page.getByText(/^(lisbon-photos|spending|home-climate) · /).first()).toBeVisible();
  await expect(page.getByText("Back up your recovery key")).toHaveCount(0);
  await expect(page.locator(".screen-node__name")).toHaveText("Home");
});

test("the examples open as photos, notes, a table and readings; a collection of one's own", async () => {
  // One example of each kind.
  await page.getByRole("link", { name: "Collections" }).first().click();
  await expect(page.locator(".screen-cover")).toHaveCount(4);
  await expect(page.locator(".screen-cover .oik-tag")).toHaveCount(4);

  await page.getByRole("link", { name: /^lisbon-photos/ }).click();
  await expect(page).toHaveURL(/\/collections\/lisbon-photos$/);
  const photos = page.locator(".screen-photo img");
  await expect(photos).toHaveCount(6);
  // Real images, decoded, in the order they were taken, each with its credit.
  await expect
    .poll(() => photos.evaluateAll((imgs) => imgs.every((img) => img.naturalWidth > 0)))
    .toBe(true);
  await expect(page.locator(".screen-photo__title").first()).toHaveText("Alfama from Santa Luzia");
  await expect(page.locator(".screen-photo__credit")).toHaveCount(6);

  await page.locator(".screen-back").click();
  await page.getByRole("link", { name: /^spending/ }).click();
  const spending = page.getByRole("table", { name: "spending" });
  await expect(spending.getByRole("cell", { name: "Metro to the airport" })).toBeVisible();
  await expect(spending.getByRole("cell", { name: "1.85", exact: true })).toBeVisible();
  // Days are shown as they were written, in any time zone.
  await expect(spending.getByRole("cell", { name: "Nov 9, 2025" }).first()).toBeVisible();

  // The guide reads in order.
  await page.locator(".screen-back").click();
  await page.getByRole("link", { name: /^oikome-guide/ }).click();
  await expect(page.locator(".screen-note")).toHaveCount(7);
  await expect(page.locator(".screen-note__text").first()).toHaveText(/^Welcome to Oikome\./);

  // Readings open at the latest ones, as a line with the rows below it.
  await page.locator(".screen-back").click();
  await page.getByRole("link", { name: /^home-climate/ }).click();
  await expect(page.getByRole("img", { name: /^Living room from .+ °C to .+ °C, / })).toBeVisible();
  await expect(page.getByText("Rows 237–336")).toBeVisible();
  await page.getByRole("radio", { name: "Humidity" }).click();
  await expect(page.getByRole("img", { name: /^Humidity from \d+ % to \d+ %, / })).toBeVisible();
  await expect(page.getByRole("table", { name: "readings" })).toBeVisible();

  // One's own: made here, kept on this computer, filled through the node.
  await page.locator(".screen-back").click();
  await page.getByRole("button", { name: "New Collection" }).first().click();
  await expect(page.getByRole("dialog").getByText("Local", { exact: true })).toBeVisible();
  await page.getByLabel("Name").fill("notes");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page).toHaveURL(/\/collections\/notes$/);
  await expect(page.getByRole("heading", { name: "No Data Yet" })).toBeVisible();
  await api("run_sql", {
    sql:
      "CREATE TABLE notes.memos (id BIGINT NOT NULL, body VARCHAR); " +
      "INSERT INTO notes.memos VALUES (1, 'buy milk'), (2, 'call mum')",
  });
  await page.reload();
  const rows = page.getByRole("table", { name: "memos" });
  await expect(rows.getByRole("cell", { name: "buy milk" })).toBeVisible();
  await expect(rows.getByRole("cell", { name: "call mum" })).toBeVisible();
  // Two rows fit on one page: no pager.
  await expect(page.getByText("Rows 1–2")).toHaveCount(0);
});

test("the examples are files: edit one, sync it, and it shows", async () => {
  // Like a notes app's vault: plain files in the data folder, read by sources.
  const spendingCsv = join(owner, "Examples", "spending", "spending.csv");
  expect(existsSync(join(owner, "Examples", "lisbon-photos", "captions.csv"))).toBe(true);
  appendFileSync(spendingCsv, "Coffee at the airport,2025-11-09,Food,2.50\n");
  await page.goto(`${base}/sources`);
  const sources = page.getByRole("table", { name: "Sources" });
  await expect(sources.locator("tbody tr")).toHaveCount(4);
  await expect(sources.getByText("Up to date").first()).toBeVisible();
  await page.getByRole("button", { name: "Sync now: spending.csv" }).click();
  await expect(page.locator(".oik-toasts")).toContainText("Read spending.csv again.");
  await page.goto(`${base}/collections/spending`);
  await expect(page.locator(".screen-facts")).toContainText("Examples/spending/spending.csv");
  await expect(
    page.getByRole("table", { name: "spending" }).getByRole("cell", { name: "Coffee at the airport" }),
  ).toBeVisible();
});

test("Home is arranged in place: a card is dragged, widened, taken off and put back", async () => {
  await page.goto(`${base}/`);
  await page.getByRole("button", { name: "Arrange" }).click();
  await expect(page.getByRole("button", { name: "Done" })).toBeVisible();
  // The cards themselves, not the rows inside them (inert while arranging).
  const cards = page.getByRole("list", { name: "Cards on Home" }).locator(":scope > [role='listitem']");
  await expect(cards).toHaveCount(6);
  await expect(cards.first()).toHaveAccessibleName("Recently added, 1 of 6");

  // Dragged by the mouse: the photo goes before Recently Added.
  const from = await cards.nth(1).boundingBox();
  const to = await cards.nth(0).boundingBox();
  await page.mouse.move(from.x + from.width / 2, from.y + 40);
  await page.mouse.down();
  await page.mouse.move(from.x + from.width / 2 - 20, from.y + 40, { steps: 3 });
  await page.mouse.move(to.x + 40, to.y + 40, { steps: 12 });
  await page.mouse.up();
  await expect(cards.first()).toHaveAccessibleName("A photo, 1 of 6");

  // The arrow keys move a focused card back.
  await cards.first().focus();
  await page.keyboard.press("ArrowRight");
  await expect(cards.first()).toHaveAccessibleName("Recently added, 1 of 6");

  // Its corner sets how many of the twelve columns it spans.
  const width = page.getByRole("slider", { name: "Width of Recently added" });
  await expect(width).toHaveAttribute("aria-valuenow", "8");
  await width.focus();
  await page.keyboard.press("End");
  await expect(width).toHaveAttribute("aria-valuenow", "12");

  // Taken off, then put back from Add a Card.
  await page.getByRole("button", { name: "Remove Places from Home" }).click();
  await expect(cards).toHaveCount(5);
  await page.getByRole("button", { name: "Add a Card" }).click();
  const dialog = page.getByRole("dialog", { name: "Add a Card" });
  await expect(dialog.getByText("On Home")).toHaveCount(5);
  await dialog.getByRole("listitem").filter({ hasText: "Places" }).getByRole("button", { name: "Add" }).click();
  await expect(dialog.getByText("On Home")).toHaveCount(6);
  await dialog.getByRole("button", { name: "Done" }).click();
  await expect(cards).toHaveCount(6);
  await page.getByRole("button", { name: "Done" }).click();
  await expect(page.getByRole("list", { name: "Cards on Home" })).toHaveCount(0);

  // The node keeps each change.
  const kept = [
    "recent:12",
    "photo:4",
    "totals:spending:7",
    "latest:home-climate:5",
    "activity:7",
    "places:5",
  ];
  await expect
    .poll(async () => (await api("node_info")).homeCards?.map((card) => `${card.id}:${card.width}`))
    .toEqual(kept);
  await page.reload();
  // The charts' cards come once their collections are read.
  await expect
    .poll(() =>
      page
        .locator(".screen-slot")
        .evaluateAll((slots) => slots.map((slot) => getComputedStyle(slot).getPropertyValue("--span").trim())),
    )
    .toEqual(["12", "4", "7", "5", "7", "5"]);
});

test("a place picked on this computer keeps new collections", async () => {
  // The folder browser lists this computer's folders; a web page can't ask
  // the system for one.
  await page.goto(`${base}/settings`);
  await page.getByRole("button", { name: "Add Storage" }).click();
  const add = page.getByRole("dialog", { name: "Add Storage" });
  await add.getByRole("button", { name: "Choose…" }).click();
  const browse = page.getByRole("dialog", { name: "Choose a Folder" });
  await expect(browse.getByText(/^Oikome keeps its files in /)).toBeVisible();
  await browse.getByRole("button", { name: "Cancel" }).click();
  await add.getByRole("button", { name: "Cancel" }).click();

  // A folder picked (here through the node, as the browser would) becomes
  // where new collections go.
  const picked = mkdtempSync(join(tmpdir(), "oikome-app-place-"));
  try {
    const added = await api("add_storage", {
      label: "Projects",
      type: "folder",
      address: picked,
      makeDefault: true,
    });
    expect(added).toMatchObject({ label: "Projects", isDefault: true, type: "folder" });
    expect(existsSync(join(picked, "Oikome"))).toBe(true);
    await page.reload();
    // It shows among the places in the sidebar, and its room can be picked there.
    const place = page.locator(".screen-place", { hasText: "Projects" });
    await expect(place).toBeVisible();
    await place.click();
    await expect(place).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator(".screen-room")).toContainText("Projects");

    await page.goto(`${base}/collections`);
    await page.getByRole("button", { name: "New Collection" }).first().click();
    await expect(page.getByLabel("Store on")).toHaveValue(added.name);
    await page.getByLabel("Name").fill("plans");
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page).toHaveURL(/\/collections\/plans$/);
    await expect(page.locator(".screen-facts")).toContainText("Projects");

    // Back to this computer for what comes next.
    await page.goto(`${base}/settings`);
    await page.getByRole("button", { name: "More actions for Local" }).click();
    await page.getByRole("menuitem", { name: "Use for New Collections" }).click();
    await expect(page.locator(".oik-toasts")).toContainText("New collections go to Local.");

    // A place stays while a collection is on it, and goes once none is.
    await page.getByRole("button", { name: "More actions for Projects" }).click();
    await page.getByRole("menuitem", { name: "Remove Place" }).click();
    await expect(page.getByRole("dialog")).toContainText("Remove the collection on it first: plans.");
    await page.getByRole("dialog").getByRole("button", { name: "Done" }).click();
    await api("drop_space", { space: "plans" });
    await page.reload();
    await page.getByRole("button", { name: "More actions for Projects" }).click();
    await page.getByRole("menuitem", { name: "Remove Place" }).click();
    await page.getByRole("dialog", { name: "Remove Projects?" }).getByRole("button", { name: "Remove" }).click();
    await expect(page.locator(".oik-toasts")).toContainText("Removed Projects.");
    await expect(page.getByRole("button", { name: "More actions for Projects" })).toHaveCount(0);
    // Local has nothing to remove: it is always there.
    await page.getByRole("button", { name: "More actions for Local" }).waitFor({ state: "detached" });
  } finally {
    rmSync(picked, { recursive: true, force: true });
  }
});

test("a contact adds the invite in their own Oikome, reads it, and leaves it once the share stops", async ({
  browser,
}) => {
  // Bob's data center, with a web app of its own.
  contactServer = await serveWeb(contact);
  const did = oikome(contact, "identity").trim();
  await page.getByRole("link", { name: "Sharing" }).click();
  // With no contact yet, New Share offers to add one and opens the form.
  await page.getByRole("button", { name: "New Share" }).click();
  const needContact = page.getByRole("dialog", { name: "New Share" });
  await needContact.getByRole("button", { name: "Add Contact" }).click();
  await expect(needContact).toHaveCount(0);
  await expect(page.getByLabel("Name")).toBeFocused();
  await page.getByLabel("Name").fill("Bob");
  await page.getByLabel("Their ID").fill(did);
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByText("Nothing shared").first()).toBeVisible();

  // New Share opens the share dialog, with the collection to choose.
  await page.getByRole("button", { name: "New Share" }).click();
  const newShare = page.getByRole("dialog", { name: "New Share" });
  await expect(newShare).toBeVisible();
  await newShare.getByLabel("Collection").selectOption("notes");
  await page.getByRole("button", { name: "Share notes with Bob" }).click();
  await expect(page.getByRole("heading", { name: "Send Bob This Invite Code" })).toBeVisible();
  const invite = (await page.locator(".oik-inset-well .oik-copy__value").textContent()).trim();
  expect(invite).toMatch(/^oik1-/);
  await page.getByRole("button", { name: "Done" }).click();
  const shareRow = page.locator(".screen-table-in-card tr", { hasText: "Bob" });
  await expect(shareRow).toContainText("notes");
  await expect(shareRow).toContainText("View live");

  // Bob, past his own first run, adds the invite on Sharing.
  contactContext = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  bob = await contactContext.newPage();
  bob.on("pageerror", (e) => console.error(`[contact page] ${e.message}`));
  const contactLink = readFileSync(join(contact, ".oikome", "web", "login"), "utf8").trim();
  bobBase = new URL(contactLink).origin;
  await bob.goto(contactLink);
  await expect(bob.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
  await bob.evaluate(async () => {
    const call = (op, body) =>
      fetch(`/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    await call("rename_node", { name: "Bob's" });
    await call("confirm_recovery_key", {});
  });
  await bob.goto(`${bobBase}/sharing`);
  await bob.getByRole("button", { name: "Add an Invite" }).click();
  const adding = bob.getByRole("dialog", { name: "Add an Invite" });
  // Half a code says so, in words.
  await adding.getByLabel("Invite code").fill(invite.slice(0, 50));
  await adding.getByRole("button", { name: "Add Invite" }).click();
  await expect(adding).toContainText("That isn't a whole invite code");
  // Pasted from a message that wrapped it.
  await adding.getByLabel("Invite code").fill(`${invite.slice(0, 64)}\n${invite.slice(64)}`);
  await adding.getByRole("button", { name: "Add Invite" }).click();
  await expect(adding).toHaveCount(0);
  const notice = bob.getByRole("region", { name: "Home invited you to \u201cnotes\u201d" });
  await expect(notice).toContainText(/View live · until \w{3} \d+, \d{4}/);
  await notice.getByRole("button", { name: "Accept: notes" }).click();
  await expect(bob.locator(".oik-toasts")).toContainText("notes is in Shared With You.");
  await expect(notice).toHaveCount(0);
  // Home is one of Bob's contacts now, who shares notes with him.
  await expect(bob.getByText("Shares notes with you")).toBeVisible();

  // Under Shared With You, from Home; it reads from Home's data center.
  await bob.getByRole("link", { name: "Collections" }).first().click();
  const shared = bob.getByRole("region", { name: "Shared With You" });
  await expect(shared).toContainText("From Home · View live");
  await shared.getByRole("link", { name: /^notes/ }).click();
  await expect(bob.getByText("Shared with you · from Home")).toBeVisible();
  const rows = bob.getByRole("table", { name: "memos" });
  await expect(rows.getByRole("cell", { name: "buy milk" })).toBeVisible();
  await expect(rows.getByRole("cell", { name: "call mum" })).toBeVisible();

  // Home stops sharing: Bob can't read it any more, and leaves it.
  await page.getByRole("button", { name: "Stop Sharing: notes, Bob" }).click();
  await expect(page.locator(".oik-toasts")).toContainText("Stopped sharing notes with Bob.");
  await expect(shareRow).toContainText("Ended");
  await bob.reload();
  await expect(bob.getByText(/can't be read right now/).first()).toBeVisible();
  await bob.getByRole("button", { name: "Leave" }).click();
  await bob.getByRole("dialog", { name: "Leave notes?" }).getByRole("button", { name: "Leave" }).click();
  await expect(bob.locator(".oik-toasts")).toContainText("Left notes.");
  await expect(bob).toHaveURL(`${bobBase}/collections`);
  await expect(bob.getByRole("region", { name: "Shared With You" })).toHaveCount(0);
});

test("Activity shows the contact's reads and the refusal", async () => {
  await page.getByRole("link", { name: "Activity" }).click();
  await expect(async () => {
    await page.reload();
    const bob = page.locator("tbody tr", { hasText: "Bob" });
    await expect(bob.filter({ hasText: "read notes" }).first()).toBeVisible({ timeout: 1000 });
    await expect(bob.filter({ hasText: "was turned away from notes" }).first()).toBeVisible({
      timeout: 1000,
    });
  }).toPass({ timeout: 20_000 });
});

test("Home lends this computer to Bob, who adds it and sees when lending is paused", async () => {
  await page.goto(`${base}/sharing`);
  await expect(page.getByText("Not lent to anyone")).toBeVisible();
  await page.getByRole("button", { name: "Change" }).click();
  const lend = page.getByRole("dialog", { name: "Lend This Computer" });
  await lend.getByRole("checkbox", { name: /Bob/ }).check();
  await lend.getByRole("radio", { name: "2 tasks" }).click();
  await lend.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("heading", { name: "Send Bob This Invite Code" })).toBeVisible();
  const code = (await page.locator(".oik-inset-well .oik-copy__value").textContent()).trim();
  await page.getByRole("button", { name: "Done" }).click();
  await expect(page.getByText("To Bob · 2 tasks at a time · 30 an hour")).toBeVisible();
  const lending = page.getByRole("switch", { name: "Lend this computer" });
  await expect(lending).toHaveAttribute("aria-checked", "true");

  // Bob adds it in his Oikome: Home's computer is there for him.
  await bob.goto(`${bobBase}/sharing`);
  await bob.getByRole("button", { name: "Add an Invite" }).click();
  await bob.getByRole("dialog", { name: "Add an Invite" }).getByLabel("Invite code").fill(code);
  await bob.getByRole("button", { name: "Add Invite" }).click();
  const notice = bob.getByRole("region", { name: "Home lends you their computer" });
  await expect(notice).toContainText("2 tasks at a time · 30 an hour");
  await notice.getByRole("button", { name: "Accept: Home's computer" }).click();
  await expect(bob.locator(".oik-toasts")).toContainText("You can use Home's computer.");
  const homes = bob.locator(".oik-row", { hasText: "Home's computer" });
  await expect(homes).toContainText("Available");

  // Paused at Home, without new codes: Bob sees it doesn't lend now.
  await lending.click();
  await expect(page.locator(".oik-toasts")).toContainText("Stopped lending this computer.");
  await expect(page.getByText(/^Paused · To Bob/)).toBeVisible();
  await bob.reload();
  await expect(homes).toContainText("Not lending now");

  // Home's Activity says what Bob asked, in words.
  await page.getByRole("link", { name: "Activity" }).click();
  await expect(async () => {
    await page.reload();
    const rows = page.locator("tbody tr", { hasText: "Bob" });
    await expect(rows.filter({ hasText: "checked whether this computer lends to them" }).first()).toBeVisible({
      timeout: 1000,
    });
    await expect(rows.filter({ hasText: "was turned away from this computer" }).first()).toBeVisible({
      timeout: 1000,
    });
  }).toPass({ timeout: 20_000 });
});

test("screens use words for people, also when something fails", async () => {
  // Engine words never reach the screens (PRD success criterion), nor SQL.
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

  for (const path of [
    "/onboarding",
    "/",
    "/collections",
    "/collections/notes",
    "/collections/lisbon-photos",
    "/collections/oikome-guide",
    "/collections/home-climate",
    "/sources",
    "/sharing",
    "/activity",
    "/settings",
  ]) {
    await page.goto(`${base}${path}`);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    const text = await page.locator("body").innerText();
    expect(text, path).not.toMatch(ENGINE);
    expect(text, path).not.toMatch(/\bsql\b/i);
  }

  // The share dialog too: plain words, and text that reads at desktop and phone width.
  for (const [width, height] of [
    [1280, 800],
    [390, 844],
  ]) {
    await page.setViewportSize({ width, height });
    await page.goto(`${base}/collections/notes`);
    await page.getByRole("button", { name: "Share…" }).click();
    const dialog = page.getByRole("dialog", { name: "Share \u201cnotes\u201d" });
    await expect(dialog).toBeVisible();
    expect(await dialog.innerText()).not.toMatch(ENGINE);
    expect(await page.evaluate(auditContrast), `share dialog at ${width}px`).toEqual([]);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  }
  await page.setViewportSize({ width: 1280, height: 800 });
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
  // A modal asks first; focus starts on Cancel, the safe choice.
  const dialog = page.getByRole("dialog", { name: "Remove scratch?" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused();
  await dialog.getByRole("button", { name: "Remove", exact: true }).click();
  // Removed, back to all collections, said in a line at the bottom.
  await expect(page).toHaveURL(/\/collections$/);
  await expect(page.locator(".oik-toasts")).toContainText("Removed scratch.");
  await expect(page.getByRole("link", { name: /^scratch/ })).toHaveCount(0);
  // The removed collection's page is not in the history any more.
  await page.goBack();
  await expect(page).not.toHaveURL(/scratch/);
});

test("Back returns to the page before, and closed panels stay closed", async () => {
  // Home is a page of its own; Back leaves it.
  await page.goto(`${base}/activity`);
  await page.goto(`${base}/`);
  await expect(page.getByRole("link", { name: "Home", exact: true })).toHaveAttribute("aria-current", "page");
  await page.goBack();
  await expect(page).toHaveURL(/\/activity$/);

  // A new page starts at the top.
  await page.setViewportSize({ width: 390, height: 600 });
  await page.goto(`${base}/sharing`);
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await page.getByRole("link", { name: "Activity" }).click();
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await page.setViewportSize({ width: 1280, height: 800 });

  // Share… opens the dialog over the collection; closing it leaves the page as it was.
  await page.goto(`${base}/collections/notes`);
  await page.getByRole("button", { name: "Share…" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page).toHaveURL(/\/collections\/notes$/);
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page).toHaveURL(/\/collections\/notes$/);

  // A collection opened directly goes up to Collections; opened from Home,
  // Back names Home.
  await page.goto(`${base}/collections/notes`);
  await page.locator(".screen-back").click();
  await expect(page).toHaveURL(/\/collections$/);
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await page.locator(".screen-memory").click();
  await expect(page).toHaveURL(/\/collections\/lisbon-photos$/);
  await expect(page.locator(".screen-back")).toHaveText("Home");
  await page.locator(".screen-back").click();
  await expect(page).toHaveURL(`${base}/`);
});

test("every screen fits desktop and phone widths, and its text is readable, in both styles", async () => {
  const screens = [
    ["/onboarding", "Save Your Recovery Key"],
    ["/", /^Good (morning|afternoon|evening)$/],
    ["/collections", "Collections"],
    ["/collections/lisbon-photos", "lisbon-photos"],
    ["/collections/oikome-guide", "oikome-guide"],
    ["/collections/home-climate", "home-climate"],
    ["/sources", "Sources"],
    ["/collections/notes", "notes"],
    ["/sharing", "Sharing"],
    ["/activity", "Activity"],
    ["/settings", "Settings"],
  ];
  // Ink is the default; Bold is picked in Settings and kept by this browser.
  for (const style of ["ink", "bold"]) {
    await page.evaluate((t) => localStorage.setItem("oikome.theme", t), style);
    for (const [width, height] of [
      [1280, 800],
      [390, 844],
    ]) {
      await page.setViewportSize({ width, height });
      for (const [path, heading] of screens) {
        await page.goto(`${base}${path}`);
        await expect(page.locator("html")).toHaveAttribute("data-theme", style);
        await expect(page.getByRole("heading", { name: heading, level: 1 })).toBeVisible();
        const overflow = await page.evaluate(
          () => document.documentElement.scrollWidth - window.innerWidth,
        );
        expect(overflow, `${path} at ${width}px (${style}) scrolls sideways`).toBeLessThanOrEqual(0);
        expect(await page.evaluate(auditContrast), `${path} at ${width}px (${style})`).toEqual([]);
      }
    }
  }
  await page.evaluate(() => localStorage.removeItem("oikome.theme"));
  await page.setViewportSize({ width: 1280, height: 800 });
});

test("the style picked in Settings applies at once and is kept", async () => {
  await page.goto(`${base}/settings`);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "ink");
  await page.getByRole("radio", { name: "Bold" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "bold");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "bold");
  await expect(page.getByRole("radio", { name: "Bold" })).toHaveAttribute("aria-checked", "true");
  await page.getByRole("radio", { name: "Ink" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "ink");
});
