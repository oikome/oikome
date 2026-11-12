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

import { execFileSync, spawn } from "node:child_process";
import dgram from "node:dgram";
import {
  appendFileSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import http from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";
import { auditContrast } from "./contrast.mjs";
import { createBucket, startRustfs } from "./rustfs.mjs";
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
async function serveWeb(root, env = ENV) {
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
      // Never the router of the computer the tests run on.
      "--no-port-mapping",
      "--web",
      "--web-listen",
      "127.0.0.1:0",
      "--web-root",
      WEB_ROOT,
    ],
    { stdio: ["ignore", "inherit", "inherit"], env },
  );
  for (let i = 0; !existsSync(join(root, ".oikome", "web", "login")); i++) {
    if (i === 200 || child.exitCode !== null) throw new Error("the node did not start its web app");
    await new Promise((r) => setTimeout(r, 100));
  }
  return child;
}

/**
 * How far the page scrolls sideways, and the innermost elements sticking out
 * past its right edge, so a failure says which.
 */
const sideways = () =>
  page.evaluate(() => {
    const vw = window.innerWidth;
    const over = document.documentElement.scrollWidth - vw;
    const out = (e) => e.getBoundingClientRect().right > vw + 0.5;
    const culprits =
      over <= 0
        ? []
        : [...document.querySelectorAll("body *")]
            .filter((e) => out(e) && ![...e.children].some(out))
            .slice(0, 4)
            .map((e) => `${e.tagName.toLowerCase()}.${String(e.className).split(" ")[0]} "${(e.textContent ?? "").trim().slice(0, 40)}"`);
    return { over, culprits: culprits.join(", ") };
  });

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
  let openerEnv = ENV;
  if (process.platform !== "win32") {
    const openerBin = join(owner, "opener-bin");
    mkdirSync(openerBin);
    const opener = join(openerBin, process.platform === "darwin" ? "open" : "xdg-open");
    writeFileSync(opener, `#!/bin/sh\nprintf '%s|%s\\n' "$#" "$1" > "${join(owner, "open-with.log")}"\n`, { mode: 0o755 });
    openerEnv = { ...ENV, PATH: `${openerBin}:${ENV.PATH ?? ""}` };
  }
  server = await serveWeb(owner, openerEnv);
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
  // The language comes first, as the examples are written in it: English
  // here, like this browser; another one redraws the page at once.
  const language = page.getByLabel("Language");
  await expect(language).toHaveValue("en");
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await language.selectOption("de");
  await expect(page.locator("html")).toHaveAttribute("lang", "de");
  await expect(page.getByRole("heading", { level: 1 })).not.toHaveText("Welcome to Oikome");
  await page.getByLabel("Sprache").selectOption("en");
  await expect(page.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
  // A lazy Continue is a fine answer: the name is filled in already.
  // The storage choices also mention a user name: the field is "Name" exactly.
  const name = page.getByLabel("Name", { exact: true });
  await expect(name).toHaveValue("My data center");
  // Where collections are kept is chosen already: Local, on this computer.
  await expect(page.getByRole("radio", { name: /^Local/ })).toBeChecked();
  // Other places can be added beside it; the dialog starts at its kinds, a
  // folder the owner picks.
  await page.getByRole("button", { name: /^Add another place/ }).click();
  const another = page.getByRole("dialog", { name: "Add Storage" });
  await expect(another.getByRole("radio", { name: /^Folder/ })).toBeChecked();
  await expect(another).toContainText("No folder chosen yet");
  await expect(another.getByRole("button", { name: "Choose…" })).toBeVisible();
  await another.getByRole("button", { name: "Cancel" }).click();
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
  // A folder viewed where it is looks like one, and says where it is first.
  const lisbonCover = page.locator(".screen-cover", { hasText: "lisbon-photos" });
  await expect(lisbonCover.locator(".screen-cover__chip")).toHaveText("Folder");
  await expect(lisbonCover.locator(".screen-cover__meta")).toHaveText(/^Local · 6 photos/);

  await page.getByRole("link", { name: /^lisbon-photos/ }).click();
  await expect(page).toHaveURL(/\/collections\/lisbon-photos$/);
  // A folder viewed where it is, as in a file manager: a tile each, by name,
  // a photo showing itself (real images, decoded).
  const photos = page.locator(".screen-ftile img");
  await expect(photos).toHaveCount(6);
  await expect
    .poll(() => photos.evaluateAll((imgs) => imgs.every((img) => img.naturalWidth > 0)))
    .toBe(true);
  await expect(page.locator(".screen-ftile__name").first()).toHaveText("Alfama from Santa Luzia.jpg");
  // Opened under its file name, it has its caption and credit.
  await page.locator("a.screen-ftile").first().click();
  const alfama = page.getByRole("dialog", { name: "Alfama from Santa Luzia.jpg" });
  await expect(alfama).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(alfama).toHaveCount(0);

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
  await expect(page.locator(".screen-ftile__text")).toHaveCount(7);
  await expect(page.locator(".screen-ftile__text").first()).toHaveText(/^Welcome to Oikome\./);

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

test("the examples are files: edit one, read it again, and it shows", async () => {
  // Like a notes app's vault: plain files in the data folder. Tables are
  // read in by sources; photos and notes are read where they are.
  const spendingCsv = join(owner, "Examples", "spending", "spending.csv");
  expect(existsSync(join(owner, "Examples", "lisbon-photos", "captions.csv"))).toBe(true);
  appendFileSync(spendingCsv, "Coffee at the airport,2025-11-09,Food,2.50\n");
  await page.goto(`${base}/sources`);
  const sources = page.getByRole("table", { name: "Sources" });
  await expect(sources.locator("tbody tr")).toHaveCount(2);
  await expect(sources.getByText("Up to date").first()).toBeVisible();
  await page.getByRole("button", { name: "Sync now: spending.csv" }).click();
  await expect(page.locator(".oik-toasts")).toContainText("Read spending.csv again.");
  await page.goto(`${base}/collections/spending`);
  await expect(page.locator(".screen-facts")).toContainText("Examples/spending/spending.csv");
  await expect(
    page.getByRole("table", { name: "spending" }).getByRole("cell", { name: "Coffee at the airport" }),
  ).toBeVisible();

  // A note added to the guide's folder shows once the folder is read again.
  writeFileSync(join(owner, "Examples", "oikome-guide", "8 Ferry.md"), "Take the ferry to Cacilhas at sunset.\n");
  await page.goto(`${base}/collections/oikome-guide`);
  await expect(page.locator(".screen-facts")).toContainText("Examples/oikome-guide");
  await expect(page.locator(".screen-facts")).toContainText("Read where it is");
  await page.getByRole("button", { name: "Read Again" }).click();
  await expect(page.locator(".oik-toasts")).toContainText("Read oikome-guide again.");
  await expect(page.getByText("Take the ferry to Cacilhas at sunset.")).toBeVisible();

  // Paused, a source stays listed; removed (after a question), its
  // collection stays, and adding the file again puts it back there.
  await page.goto(`${base}/sources`);
  const climate = sources.locator("tbody tr").filter({ hasText: "home-climate" });
  const more = () => page.getByRole("button", { name: "More actions for readings.csv" }).click();
  await more();
  await page.getByRole("menuitem", { name: "Pause" }).click();
  await expect(page.locator(".oik-toasts")).toContainText("Paused readings.csv.");
  await expect(climate).toContainText("Paused");
  await more();
  await page.getByRole("menuitem", { name: "Resume" }).click();
  await expect(climate).toContainText("Up to date");
  await more();
  await page.getByRole("menuitem", { name: "Remove Source…" }).click();
  await page.getByRole("dialog", { name: "Remove readings.csv?" }).getByRole("button", { name: "Remove" }).click();
  await expect(page.locator(".oik-toasts")).toContainText("Removed readings.csv.");
  await expect(sources.locator("tbody tr")).toHaveCount(1);
  const readings = join(owner, "Examples", "home-climate", "readings.csv");
  const back = await api("add_source", { kind: "csv", path: readings });
  expect(back.name).toBe("home-climate");

  // Its page: what it did lately, how often it reads, paused with a switch,
  // and removed from there too.
  await page.reload();
  await more();
  await page.getByRole("menuitem", { name: "Open Details" }).click();
  await expect(page).toHaveURL(`${base}/sources/home-climate`);
  await expect(page.getByRole("heading", { name: "readings.csv", level: 1 })).toBeVisible();
  await expect(page.getByRole("region", { name: "Recent syncs" })).toContainText("Done");
  const pause = page.getByRole("switch", { name: "Pause this source" });
  await pause.click();
  await expect(pause).toHaveAttribute("aria-checked", "true");
  await expect(page.locator(".screen-facts")).toContainText("Paused");
  await pause.click();
  await expect(page.locator(".screen-facts")).toContainText("Up to date");
  const hourly = page.getByRole("radiogroup", { name: "Check for new" }).getByRole("radio", { name: "Hourly" });
  await hourly.click();
  await expect(hourly).toHaveAttribute("aria-checked", "true");
  await page.getByRole("button", { name: "Remove Source" }).click();
  await page.getByRole("dialog", { name: "Remove readings.csv?" }).getByRole("button", { name: "Remove" }).click();
  await expect(page).toHaveURL(`${base}/sources`);
  await expect(sources.locator("tbody tr")).toHaveCount(1);
  await api("add_source", { kind: "csv", path: readings });
});

test("collections are shown as in a file manager: as covers or a list, in the order picked, kept", async () => {
  await page.goto(`${base}/collections`);
  const tools = page.locator(".screen-cols-tools");
  // The newest first, as covers, until the owner picks otherwise.
  await expect(tools.getByRole("button", { name: "Last Changed" })).toBeVisible();
  await tools.getByRole("radio", { name: "List" }).click();
  const list = page.locator(".screen-rows-table").first();
  await expect(list.getByRole("columnheader", { name: /Last Changed/ })).toHaveAttribute("aria-sort", "descending");
  await expect(list.getByRole("row", { name: /spending/ })).toContainText("Local");
  // By name from A, and again the other way round.
  const names = () => list.locator(".screen-rows-table__link").allTextContents();
  await list.getByRole("button", { name: "Name" }).click();
  await expect(list.getByRole("columnheader", { name: /Name/ })).toHaveAttribute("aria-sort", "ascending");
  const fromA = await names();
  expect(fromA).toEqual([...fromA].sort((a, b) => a.localeCompare(b, "en", { numeric: true, sensitivity: "base" })));
  await list.getByRole("button", { name: "Name" }).click();
  await expect.poll(names).toEqual([...fromA].reverse());
  // Folders viewed in place, and only them: the examples of photos and notes.
  await tools.getByRole("radio", { name: "Folders" }).click();
  await expect(list.locator(".screen-rows-table__link")).toHaveText(["oikome-guide", "lisbon-photos"]);
  await tools.getByRole("radio", { name: "All" }).click();
  // A row opens its collection.
  await list.getByRole("row", { name: /spending/ }).getByRole("cell").nth(2).click();
  await expect(page).toHaveURL(/\/collections\/spending$/);
  // Kept in this browser: a list, by name, Z first.
  await page.goto(`${base}/collections`);
  await expect(tools.getByRole("radio", { name: "List" })).toHaveAttribute("aria-checked", "true");
  await expect(list.getByRole("columnheader", { name: /Name/ })).toHaveAttribute("aria-sort", "descending");
  // The sort button picks the same orders.
  await tools.getByRole("button", { name: "Name" }).click();
  await expect(page.getByRole("menuitemradio", { name: "Name" })).toHaveAttribute("aria-checked", "true");
  await page.getByRole("menuitemradio", { name: "Last Changed" }).click();
  await expect(list.getByRole("columnheader", { name: /Last Changed/ })).toHaveAttribute("aria-sort", "descending");
  await tools.getByRole("radio", { name: "Covers" }).click();
  await expect(page.locator(".screen-cover").first()).toBeVisible();
});

test("⌘K searches what the collections hold, and a hit opens that one on its own", async () => {
  await page.goto(`${base}/`);
  const palette = page.getByRole("dialog", { name: "Search" });
  const box = palette.getByRole("combobox", { name: "Search" });
  // ⌘K is listened for once the app's frame is on the page.
  await expect(page.getByRole("button", { name: /Search everything/ })).toBeVisible();
  await page.keyboard.press("Control+k");
  await expect(palette).toBeVisible();

  // A photo by its caption, accents aside, shown with its picture.
  await box.fill("belem");
  const pasteis = palette.getByRole("group", { name: "Photos" }).getByRole("option", { name: /Pastéis de Belém/ });
  await expect(pasteis.locator("mark")).toHaveText("Belém");
  await expect.poll(() => pasteis.locator("img").evaluate((img) => img.naturalWidth)).toBeGreaterThan(0);
  await pasteis.click();
  await expect(page).toHaveURL(/\/collections\/lisbon-photos\?table=[^&]+&item=/);
  const photo = page.getByRole("dialog", { name: "Pastéis de Belém.jpg" });
  await expect(photo.getByRole("img", { name: "Pastéis de Belém" })).toBeVisible();
  await expect(photo).not.toContainText("Kept on");
  await photo.getByRole("button", { name: "Info" }).click();
  await expect(photo).toContainText("Kept on");
  // Its address opens it again; closing it leaves its collection.
  await page.reload();
  await expect(photo).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(photo).toHaveCount(0);
  await expect(page).toHaveURL(/\/collections\/lisbon-photos$/);

  // Searching in one collection: its notes only, the note in full.
  await page.goto(`${base}/collections/oikome-guide`);
  await page.getByRole("button", { name: "Search in here" }).click();
  await expect(palette.getByText("in oikome-guide", { exact: true })).toBeVisible();
  await box.fill("welcome");
  const notes = palette.getByRole("group", { name: "Notes" });
  await expect(notes.getByRole("option").first()).toContainText("Welcome to Oikome");
  await expect(palette.getByRole("group", { name: "Pages" })).toHaveCount(0);
  await notes.getByRole("option").first().click();
  const note = page.getByRole("dialog", { name: "1 Welcome.md" });
  await expect(note).toContainText("Welcome to Oikome.");
  await note.getByRole("button", { name: "Close" }).click();
  await expect(note).toHaveCount(0);
  await expect(page).toHaveURL(/\/collections\/oikome-guide$/);

  // A row of a table, field by field; the column it matched in is named.
  await page.keyboard.press("Control+k");
  await box.fill("airport");
  const metro = palette.getByRole("group", { name: "Rows" }).getByRole("option", { name: /Metro to the airport/ });
  await expect(metro).toContainText("spending");
  await metro.click();
  const row = page.getByRole("dialog", { name: "Row" });
  await expect(row).toContainText("Metro to the airport");
  await expect(row).toContainText("1.85");
  await page.keyboard.press("Escape");
  await expect(page).toHaveURL(/\/collections\/spending$/);

  // A table with no key opens its collection; nothing found says so.
  await page.keyboard.press("Control+k");
  await box.fill("milk");
  await palette.getByRole("group", { name: "Rows" }).getByRole("option", { name: /buy milk/ }).click();
  await expect(page).toHaveURL(/\/collections\/notes$/);
  await page.keyboard.press("Control+k");
  await box.fill("zzqxv");
  await expect(palette.getByText("Nothing matches “zzqxv”.")).toBeVisible();
  await page.keyboard.press("Escape");
});

test("a photo opens large and the arrow keys move through every file; a note opens in full with its tags", async () => {
  await page.goto(`${base}/collections/lisbon-photos`);
  const links = page.locator("a.screen-ftile");
  await expect(links).toHaveCount(7);

  // Opened over the page, which stays where it was. Its full details are
  // behind Info, not beside the picture.
  const fifth = links.nth(4);
  await fifth.scrollIntoViewIfNeeded();
  const y = await page.evaluate(() => window.scrollY);
  await fifth.click();
  const photo = page.getByRole("dialog");
  await expect(photo).toContainText("5 of 7 · ← → to move");
  await expect(photo).not.toContainText("Who can see this");
  await photo.getByRole("button", { name: "Info" }).click();
  await expect(photo).toContainText("Who can see this");
  await photo.getByRole("button", { name: "Info" }).click();
  await expect(page).toHaveURL(/\/collections\/lisbon-photos\?table=files&item=/);
  expect(await page.evaluate(() => window.scrollY)).toBe(y);

  await page.keyboard.press("ArrowRight");
  await expect(photo).toContainText("6 of 7");
  await photo.getByRole("button", { name: "Next" }).click();
  await expect(photo).toContainText("7 of 7");
  await expect(photo.getByRole("button", { name: "Next" })).toBeDisabled();
  await photo.getByRole("button", { name: "Previous" }).click();
  await photo.getByRole("button", { name: "Previous" }).click();
  await expect(photo).toContainText("5 of 7");
  // Moving kept one step in history: closing goes back to the page as it was.
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page).toHaveURL(/\/collections\/lisbon-photos$/);
  expect(await page.evaluate(() => window.scrollY)).toBe(y);

  // The captions are a file too, and open as a table read in place.
  await page.locator("a.screen-ftile", { hasText: "captions.csv" }).click();
  const captions = page.getByRole("dialog", { name: "captions.csv" });
  await expect(captions.getByRole("table", { name: "captions.csv" })).toBeVisible();
  await page.keyboard.press("Escape");

  // A note with a line of tags: the tags show as tags, the note in full.
  const tram = join(owner, "Examples", "oikome-guide", "10 Tram.md");
  writeFileSync(tram, "Took the 28 up to Graça, then walked down.\n\n#lisbon #trams\n");
  await api("refresh_space", { space: "oikome-guide" });
  await page.goto(`${base}/collections/oikome-guide`);
  // Its tile shows its first lines, without the line of tags.
  const card = page.locator("a.screen-ftile").filter({ hasText: "Took the 28" });
  await expect(card.locator(".screen-ftile__text")).toHaveText("Took the 28 up to Graça, then walked down.");
  await card.click();
  const note = page.getByRole("dialog", { name: "10 Tram.md" });
  await expect(note).toContainText("Took the 28 up to Graça, then walked down.");
  await expect(note.locator(".oik-tag")).toHaveText(["#lisbon", "#trams"]);
  await expect(note).toContainText(/\d+ of 9/);
  await note.getByRole("button", { name: "Close" }).click();
  await expect(page).toHaveURL(/\/collections\/oikome-guide$/);
  rmSync(tram);
  await api("refresh_space", { space: "oikome-guide" });
});

test("a CSV opens as a table where it is, with its details behind Info", async () => {
  const tips = join(owner, "Examples", "oikome-guide", "tips.csv");
  writeFileSync(tips, "what,where\nTram 28,Graça\nFerry,Cacilhas\n");
  try {
    await api("refresh_space", { space: "oikome-guide" });
    await page.goto(`${base}/collections/oikome-guide`);
    await page.locator("a.screen-ftile", { hasText: "tips.csv" }).click();
    const viewer = page.getByRole("dialog", { name: "tips.csv" });
    const table = viewer.getByRole("table", { name: "tips.csv" });
    await expect(table.getByRole("columnheader", { name: "What" })).toBeVisible();
    await expect(table.getByRole("columnheader", { name: "Where" })).toBeVisible();
    await expect(table.getByRole("cell", { name: "Tram 28" })).toBeVisible();
    await expect(table.getByRole("cell", { name: "Cacilhas" })).toBeVisible();
    await expect(viewer).not.toContainText("Kept on");
    await viewer.getByRole("button", { name: "Info" }).click();
    await expect(viewer).toContainText("Kept on");
    await expect(viewer).toContainText("CSV");
    // Moving goes through notes and data files alike, then Escape comes out.
    await page.keyboard.press("ArrowLeft");
    await expect(page.getByRole("dialog", { name: "8 Ferry.md" })).toBeVisible();
    await page.keyboard.press("ArrowRight");
    await expect(viewer).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
  } finally {
    rmSync(tips, { force: true });
    await api("refresh_space", { space: "oikome-guide" });
  }
});

/** One MCP request from outside the browser, as an AI assistant sends it. */
let rpc = 0;
async function mcp(endpoint, token, method, params = {}, { modern = true, headers = {}, id = true } = {}) {
  const meta = {
    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
    "io.modelcontextprotocol/clientInfo": { name: "e2e", version: "1" },
    "io.modelcontextprotocol/clientCapabilities": {},
  };
  const body = { jsonrpc: "2.0", method, params: modern ? { ...params, _meta: meta } : params };
  if (id) body.id = ++rpc;
  const sent = { "content-type": "application/json", accept: "application/json, text/event-stream" };
  if (token) sent.authorization = `Bearer ${token}`;
  if (modern) {
    sent["mcp-protocol-version"] = "2026-07-28";
    sent["mcp-method"] = method;
    if (method === "tools/call") sent["mcp-name"] = params.name;
  }
  const res = await fetch(endpoint, { method: "POST", headers: { ...sent, ...headers }, body: JSON.stringify(body) });
  const text = await res.text();
  return { status: res.status, body: text ? JSON.parse(text) : null };
}

test("an AI assistant connects over MCP, reads only what it is given, and is cut off at once", async () => {
  await page.goto(`${base}/assistants`);
  await expect(page.getByRole("heading", { name: "AI Assistants", level: 1 })).toBeVisible();
  await page.getByRole("button", { name: "Connect an Assistant" }).click();
  const dialog = page.getByRole("dialog", { name: "Connect an Assistant" });
  await dialog.getByRole("button", { name: "Next", exact: true }).click();
  await dialog.getByRole("checkbox", { name: "oikome-guide" }).check();
  await dialog.getByRole("button", { name: "Next: Connect" }).click();
  // The command to paste, with its key, shown this once.
  const command = (await dialog.locator(".oik-copy__value").textContent()).trim();
  expect(command).toMatch(
    /^claude mcp add --transport http oikome http:\/\/127\.0\.0\.1:\d+\/mcp --header "Authorization: Bearer oik_[A-Za-z0-9_-]+"$/,
  );
  const endpoint = command.split(" ")[6];
  const token = command.match(/Bearer (oik_[A-Za-z0-9_-]+)/)[1];
  await expect(dialog).toContainText("Waiting for Claude Code…");

  // A client of the handshake era: initialize, then on without a session.
  const hello = await mcp(endpoint, token, "initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "e2e", version: "1" } }, { modern: false });
  expect(hello.status).toBe(200);
  expect(hello.body.result.protocolVersion).toBe("2025-06-18");
  expect((await mcp(endpoint, token, "notifications/initialized", {}, { modern: false, id: false })).status).toBe(202);
  const legacy = await mcp(endpoint, token, "tools/list", {}, { modern: false, headers: { "mcp-protocol-version": "2025-06-18" } });
  expect(legacy.body.result.tools.map((t) => t.name)).toEqual(["list_collections", "search", "read_row"]);
  // The page sees it arrive.
  await expect(dialog).toContainText("Claude Code is connected", { timeout: 10_000 });
  await dialog.getByRole("button", { name: "Done" }).click();

  // A client of this revision: discovery, then the three tools and no more.
  const found = await mcp(endpoint, token, "server/discover");
  expect(found.body.result.supportedVersions).toContain("2026-07-28");
  expect(found.body.result.resultType).toBe("complete");
  const call = (name, args = {}) => mcp(endpoint, token, "tools/call", { name, arguments: args });
  const listed = await call("list_collections");
  expect(listed.body.result.structuredContent.collections.map((c) => c.name)).toEqual(["oikome-guide"]);
  const welcome = await call("search", { query: "welcome" });
  const hits = welcome.body.result.structuredContent.hits;
  expect(hits.length).toBeGreaterThan(0);
  expect(hits.every((h) => h.collection === "oikome-guide")).toBe(true);
  expect((await call("search", { query: "belem" })).body.result.structuredContent.hits).toEqual([]);
  const elsewhere = await call("search", { query: "belem", collection: "lisbon-photos" });
  expect(elsewhere.body.result.isError).toBe(true);
  expect(elsewhere.body.result.content[0].text).toBe("there is no collection lisbon-photos");
  const row = await call("read_row", { collection: hits[0].collection, table: hits[0].table, key: hits[0].key });
  expect(row.body.result.structuredContent.fields.text).toMatch(/Welcome to Oikome/);
  const owners = await call("run_sql", { sql: "SELECT 1" });
  expect(owners.body.error.code).toBe(-32602);

  // Refused: a web page, no key or another, a stream, headers that differ from the body.
  expect((await mcp(endpoint, token, "tools/list", {}, { headers: { origin: "https://example.com" } })).status).toBe(403);
  expect((await mcp(endpoint, null, "tools/list")).status).toBe(401);
  expect((await mcp(endpoint, "oik_someone-else", "tools/list")).status).toBe(401);
  expect((await fetch(endpoint, { headers: { authorization: `Bearer ${token}` } })).status).toBe(405);
  const mismatch = await mcp(endpoint, token, "tools/list", {}, { headers: { "mcp-method": "tools/call" } });
  expect(mismatch.status).toBe(400);
  expect(mismatch.body.error.code).toBe(-32020);

  // Given the photos on the page, it sees them, and a photo comes as an image.
  await page.reload();
  await page.getByRole("switch", { name: "Claude Code can read: lisbon-photos" }).click();
  await expect(page.getByRole("switch", { name: "Claude Code can read: lisbon-photos" })).toHaveAttribute("aria-checked", "true");
  const photos = (await call("search", { query: "belem" })).body.result.structuredContent.hits;
  const photo = photos.find((h) => h.collection === "lisbon-photos");
  expect(photo).toBeTruthy();
  const picture = await call("read_row", { collection: photo.collection, table: photo.table, key: photo.key });
  expect(picture.body.result.content.some((c) => c.type === "image" && c.mimeType.startsWith("image/"))).toBe(true);

  // What it searched and read, on its page and in Activity.
  await page.reload();
  await expect(page.getByText("Claude Code searched oikome-guide for “welcome”").first()).toBeVisible();
  await page.goto(`${base}/activity`);
  await page.getByRole("radio", { name: "Assistants" }).click();
  await expect(page.getByText("searched oikome-guide for “welcome”").first()).toBeVisible();
  await expect(page.getByText("was turned away from lisbon-photos").first()).toBeVisible();

  // Its access ended: the very next request is refused; removed, it's gone.
  await page.goto(`${base}/assistants`);
  await page.getByRole("button", { name: "Edit Claude Code" }).click();
  await page.getByRole("button", { name: "End Access" }).click();
  await page.getByRole("dialog", { name: "End Claude Code's access?" }).getByRole("button", { name: "End Access" }).click();
  await expect(page.locator(".oik-toasts")).toContainText("Ended Claude Code's access.");
  expect((await call("list_collections")).status).toBe(401);
  await page.getByRole("button", { name: "Remove Claude Code" }).click();
  await page.getByRole("dialog", { name: "Remove Claude Code?" }).getByRole("button", { name: "Remove" }).click();
  await expect(page.getByText("No assistant is connected yet.", { exact: false })).toBeVisible();
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

    // Collections by place: only what is kept on Projects; nothing shared
    // with you is kept at a place of yours.
    await page.goto(`${base}/collections`);
    const tools = page.locator(".screen-cols-tools");
    await tools.getByRole("button", { name: "All Places" }).click();
    await page.getByRole("menuitemradio", { name: "Projects" }).click();
    await expect(tools.getByRole("button", { name: "Projects" })).toBeVisible();
    await expect(page.locator(".screen-cover")).toHaveCount(1);
    await expect(page.locator(".screen-cover")).toContainText("plans");
    await expect(page.getByRole("heading", { name: "Shared With You" })).toHaveCount(0);

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

test("a bucket on RustFS keeps new collections, encrypted", async () => {
  const rustfs = await startRustfs();
  test.skip(!rustfs, "no RustFS here (brew install rustfs/tap/rustfs)");
  try {
    await createBucket(rustfs, "family");
    await page.goto(`${base}/settings`);
    await page.getByRole("button", { name: "Add Storage" }).click();
    const add = page.getByRole("dialog", { name: "Add Storage" });
    await add.getByRole("radio", { name: /^Object storage/ }).check();
    // The choices' notes mention an address and keys too: the fields by role.
    await add.getByRole("textbox", { name: "Address", exact: true }).fill(rustfs.endpoint);
    await add.getByRole("textbox", { name: "Bucket", exact: true }).fill("family");
    await add.getByRole("textbox", { name: "Access key", exact: true }).fill(rustfs.accessKey);
    await add.getByLabel("Secret key", { exact: true }).fill("not-the-secret-key");
    // Encrypted unless the owner says no, and then told who can read it.
    const encrypt = add.getByLabel("Encrypt files before they leave this computer");
    await expect(encrypt).toBeChecked();
    await encrypt.uncheck();
    await expect(add).toContainText("Without encryption, whoever runs the storage can read the files.");
    await encrypt.check();
    await add.getByRole("button", { name: "Add Storage" }).click();
    await expect(add).toContainText("Couldn't reach it. Check the address, bucket and keys, then try again.");
    await add.getByLabel("Secret key", { exact: true }).fill(rustfs.secretKey);
    await add.getByLabel("Use for New Collections").check();
    await add.getByRole("button", { name: "Add Storage" }).click();
    await expect(add).toHaveCount(0);
    const [added] = (await api("list_storages")).filter((p) => p.type === "s3");
    expect(added).toMatchObject({ label: "family", isDefault: true, encrypted: true });
    await expect(page.locator(".oik-row", { hasText: "family" }).first()).toContainText("RustFS");

    await page.goto(`${base}/collections`);
    await page.getByRole("button", { name: "New Collection" }).first().click();
    await expect(page.getByLabel("Store on")).toHaveValue(added.name);
    await page.getByLabel("Name").fill("trip");
    await page.getByRole("button", { name: "Create" }).click();
    await expect(page).toHaveURL(/\/collections\/trip$/);
    await expect(page.locator(".screen-facts")).toContainText("family");

    // Back to this computer, and the bucket's place goes once nothing is on it.
    await api("set_default_storage", { storage: null });
    await api("drop_space", { space: "trip" });
    await page.goto(`${base}/settings`);
    await page.getByRole("button", { name: "More actions for family" }).click();
    await page.getByRole("menuitem", { name: "Remove Place" }).click();
    await page.getByRole("dialog", { name: "Remove family?" }).getByRole("button", { name: "Remove" }).click();
    await expect(page.locator(".oik-toasts")).toContainText("Removed family.");
  } finally {
    rustfs?.stop();
  }
});

test("a place picked in the sidebar opens its page: what is kept there and how it is reached", async () => {
  await page.goto(`${base}/`);
  await page.getByRole("button", { name: "Local", exact: true }).click();
  await expect(page).toHaveURL(`${base}/places/this-computer`);
  await expect(page.getByRole("heading", { level: 1, name: "Local" })).toBeVisible();
  const kept = page.getByRole("region", { name: "Kept here" });
  await expect(kept.getByRole("link", { name: "spending" })).toBeVisible();
  await expect(page.getByRole("region", { name: "How Oikome reaches it" })).toContainText("Writes in");
  await expect(page.getByRole("region", { name: "If It Breaks" })).toContainText("kept only on this computer");
  // A place that isn't there says so.
  await page.goto(`${base}/places/nowhere`);
  await expect(page.getByText("There Is No Such Place")).toBeVisible();
});

test("Google Drive is connected through the owner's own Google app, on Google's page", async ({ browser }) => {
  // The choice says how to register an app with Google, and asks for it.
  await page.goto(`${base}/settings`);
  await page.getByRole("button", { name: "Add Storage" }).click();
  const add = page.getByRole("dialog", { name: "Add Storage" });
  await expect.poll(() => add.evaluate((el) => Math.round(el.getBoundingClientRect().top))).toBe(64);
  // A cloud drive, then its provider from a scalable menu; Google Drive comes first.
  await add.getByRole("radio", { name: /^Cloud drive/ }).check();
  await expect(add.getByLabel("Cloud drive provider")).toHaveValue("Google Drive");
  // Where Google Drive for desktop is on this computer, its folder comes first.
  const signIn = add.getByRole("radio", { name: /^Sign in on the web/ });
  if (await signIn.count()) await signIn.check();
  else await expect(add).toContainText("Install Google Drive for desktop on this computer");
  // Two steps: register the app, then give it to Oikome.
  await expect(add.getByRole("heading", { name: /Register your app with Google/ })).toBeVisible();
  await expect(add.getByRole("heading", { name: /Give Oikome your app/ })).toBeVisible();
  await expect(add.getByRole("link", { name: "Enable the Google Drive API" })).toHaveAttribute(
    "href",
    "https://console.cloud.google.com/apis/library/drive.googleapis.com",
  );
  await expect(add.getByRole("link", { name: "Name and publish the app" })).toHaveAttribute(
    "href",
    "https://console.cloud.google.com/auth/branding",
  );
  await expect(add.getByRole("link", { name: "Create a Desktop app client and download its file" })).toHaveAttribute(
    "href",
    "https://console.cloud.google.com/auth/clients/create",
  );
  await expect(add).toContainText("The JSON downloaded when you created your Desktop app OAuth client.");
  await add.getByRole("button", { name: "Continue to Google" }).click();
  await expect(add).toContainText("Paste your Google app's Client ID and Client secret.");
  // The file Google lets one download fills both, read in the page only.
  const appFile = add.locator('input[type="file"]');
  await appFile.setInputFiles({
    name: "client_secret_made-up.apps.googleusercontent.com.json",
    mimeType: "application/json",
    buffer: Buffer.from(
      JSON.stringify({
        installed: { client_id: "from-file.apps.googleusercontent.com", client_secret: "from-file-secret" },
      }),
    ),
  });
  await expect(add.getByLabel("Client ID")).toHaveValue("from-file.apps.googleusercontent.com");
  await expect(add.getByLabel("Client secret")).toHaveValue("from-file-secret");
  await expect(add).toContainText("client_secret_made-up.apps.googleusercontent.com.json");
  await appFile.setInputFiles({ name: "notes.json", mimeType: "application/json", buffer: Buffer.from("{}") });
  await expect(add).toContainText("That isn't a Google OAuth client JSON file.");
  await add.getByRole("button", { name: "Cancel" }).click();

  // A new data center, on its first run, with a (made-up) app of the
  // owner's given on the command line: the page asks for nothing, and
  // goes to Google's page, which is never reached here.
  const root = mkdtempSync(join(tmpdir(), "oikome-app-google-"));
  execFileSync(BIN, ["--root", root, "storage", "google-app"], {
    input: "made-up.apps.googleusercontent.com\nmade-up-secret\n",
    env: ENV,
  });
  const node = await serveWeb(root);
  const fresh = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  try {
    const signIns = [];
    await fresh.route("https://accounts.google.com/**", (route) => {
      signIns.push(route.request().url());
      return route.fulfill({ contentType: "text/html", body: "<title>Google</title>" });
    });
    const tab = await fresh.newPage();
    const link = readFileSync(join(root, ".oikome", "web", "login"), "utf8").trim();
    const origin = new URL(link).origin;
    await tab.goto(link);
    await expect(tab.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
    await tab.getByRole("radio", { name: /^Cloud drive/ }).check();
    const tabSignIn = tab.getByRole("radio", { name: /^Sign in on the web/ });
    if (await tabSignIn.count()) await tabSignIn.check();
    await expect(tab.locator(".screen-place-form")).toContainText("Your Google app: made-up.apps.googleusercontent.com");
    await expect(tab.getByLabel("Client secret")).toHaveCount(0);
    await tab.getByRole("button", { name: "Continue to Google" }).click();
    await expect.poll(() => signIns.length).toBe(1);
    const params = new URL(signIns[0]).searchParams;
    // The owner's app; back to this data center on loopback; only the files
    // Oikome makes; the secret stays on the computer.
    expect(params.get("client_id")).toBe("made-up.apps.googleusercontent.com");
    expect(params.get("redirect_uri")).toBe(`${origin}/oauth/google/callback`);
    expect(params.get("scope")).toBe("https://www.googleapis.com/auth/drive.file");
    expect(params.get("code_challenge_method")).toBe("S256");
    expect(signIns[0]).not.toContain("made-up-secret");

    // The owner says no on Google's page: back to the first step, told so.
    await tab.goto(`${origin}/oauth/google/callback?state=${params.get("state")}&error=access_denied`);
    await expect(tab).toHaveURL(`${origin}/onboarding`);
    await expect(tab.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
    await expect(tab.locator("main")).toContainText(
      "Google Drive wasn't connected: access wasn't given on Google's page.",
    );
    // The same way back again finds nothing to finish.
    await tab.goto(`${origin}/oauth/google/callback?state=${params.get("state")}&code=x`);
    await expect(tab).toHaveURL(`${origin}/settings`);
    await expect(tab.locator("main")).toContainText("Couldn't connect Google Drive. Try again.");
  } finally {
    await fresh.close();
    node.kill();
    rmSync(root, { recursive: true, force: true });
  }
});

test("iCloud Drive and OneDrive stay visible before desktop sync is set up", async ({ browser }) => {
  const root = mkdtempSync(join(tmpdir(), "oikome-app-sync-drives-"));
  const node = await serveWeb(root, { ...ENV, HOME: root });
  const fresh = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  try {
    const tab = await fresh.newPage();
    await tab.goto(readFileSync(join(root, ".oikome", "web", "login"), "utf8").trim());
    await expect(tab.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
    await tab.getByRole("radio", { name: /^Cloud drive/ }).check();
    const form = tab.locator(".screen-place-form");
    const next = tab.getByRole("button", { name: "Continue", exact: true });

    const provider = tab.getByLabel("Cloud drive provider");
    await provider.selectOption("iCloud Drive");
    await expect(form).toContainText("Turn on iCloud Drive in your Apple Account settings");
    await expect(next).toBeDisabled();

    await provider.selectOption("OneDrive");
    await expect(form).toContainText("Install OneDrive and sign in");
    await expect(next).toBeDisabled();

    await provider.selectOption("iCloud Drive");
    mkdirSync(join(root, "Library", "Mobile Documents", "com~apple~CloudDocs"), { recursive: true });
    await tab.getByRole("button", { name: "Check again" }).click();
    await expect(next).toBeEnabled();
    await expect(form).toContainText("Oikome keeps its files in");
  } finally {
    await fresh.close();
    node.kill();
    rmSync(root, { recursive: true, force: true });
  }
});

test("Dropbox uses a copied authorization code with PKCE and offline access", async ({ browser }) => {
  const root = mkdtempSync(join(tmpdir(), "oikome-app-dropbox-"));
  execFileSync(BIN, ["--root", root, "storage", "dropbox-app"], {
    input: "made-up-dropbox-app\n",
    env: ENV,
  });
  const node = await serveWeb(root);
  const fresh = await browser.newContext({ viewport: { width: 1280, height: 800 } });
  try {
    const signIns = [];
    await fresh.route("https://www.dropbox.com/**", (route) => {
      signIns.push(route.request().url());
      return route.fulfill({ contentType: "text/html", body: "<title>Dropbox</title>" });
    });
    const tab = await fresh.newPage();
    const link = readFileSync(join(root, ".oikome", "web", "login"), "utf8").trim();
    await tab.goto(link);
    await expect(tab.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
    await tab.getByRole("radio", { name: /^Cloud drive/ }).check();
    await tab.getByLabel("Cloud drive provider").selectOption("Dropbox");
    const signIn = tab.getByRole("radio", { name: /^Sign in on the web/ });
    if (await signIn.count()) await signIn.check();
    await expect(tab.locator(".screen-place-form")).toContainText("Your Dropbox app: made-up-dropbox-app");
    await expect(tab.getByLabel("App secret", { exact: true })).toHaveCount(0);
    await tab.getByRole("button", { name: "Use another app" }).click();
    // No redirect registration: create the app, tick its permissions, then
    // give Oikome its App key. The Permissions tab links to this app directly.
    const form = tab.locator(".screen-place-form");
    await expect(tab.getByRole("heading", { name: "Register your app with Dropbox" })).toBeVisible();
    await expect(tab.getByRole("heading", { name: "Give Oikome your app" })).toBeVisible();
    await expect(tab.getByRole("link", { name: "Create an app" })).toHaveAttribute(
      "href",
      "https://www.dropbox.com/developers/apps/create",
    );
    await expect(form).toContainText("App folder, or Full Dropbox to view your other files too");
    await expect(form).not.toContainText("redirect URI");
    await expect(form).toContainText("On its Permissions tab, tick these, then Save changes at the top right:");
    const listedScopes = await form.locator(".screen-steps__values li").allTextContents();
    await expect(form.locator("details")).toHaveCount(0);
    await expect(form.getByRole("link", { name: /On its Permissions tab/ })).toHaveCount(0);
    await expect(form).not.toContainText("Your other files are viewed read only");
    const readAll = tab.getByRole("checkbox", { name: /Also view your other Dropbox files/ });
    await readAll.check();
    await expect(form).toContainText("Your other files are viewed read only and stay in Dropbox.");
    await readAll.uncheck();
    await tab.getByLabel("App key", { exact: true }).fill("made-up-dropbox-app");
    await expect(form.getByRole("link", { name: /On its Permissions tab/ })).toHaveAttribute(
      "href",
      "https://www.dropbox.com/developers/console/apps/made-up-dropbox-app/permissions",
    );
    await expect(tab.getByLabel("Client ID", { exact: true })).toHaveCount(0);
    await expect(tab.getByLabel("Client secret", { exact: true })).toHaveCount(0);

    const dropboxPage = fresh.waitForEvent("page");
    await tab.getByRole("button", { name: "Continue to Dropbox" }).click();
    await dropboxPage;
    await expect.poll(() => signIns.length).toBe(1);
    const params = new URL(signIns[0]).searchParams;
    expect(params.get("client_id")).toBe("made-up-dropbox-app");
    expect(params.has("redirect_uri")).toBe(false);
    expect(params.has("state")).toBe(false);
    expect(params.get("token_access_type")).toBe("offline");
    expect(params.get("code_challenge_method")).toBe("S256");
    expect(params.get("scope")).toBe(
      "account_info.read files.content.read files.content.write files.metadata.read files.metadata.write",
    );
    expect([...listedScopes, "account_info.read", "files.metadata.read"].sort()).toEqual(
      params.get("scope").split(" ").sort(),
    );

    // Oikome keeps its place and asks for the code shown on Dropbox's page.
    await expect(tab.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
    await expect(tab.getByRole("textbox", { name: "Authorization code", exact: true })).toBeFocused();
    await expect(tab.getByRole("link", { name: "Open Dropbox again" })).toHaveAttribute("href", signIns[0]);
    await tab.getByRole("button", { name: "Connect Dropbox" }).click();
    await expect(tab.locator("main")).toContainText("Paste the authorization code shown on Dropbox's page.");
  } finally {
    await fresh.close();
    node.kill();
    rmSync(root, { recursive: true, force: true });
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
  // First, whether Bob may reach it from outside this network: asked, this once.
  const asked = page.getByRole("dialog", { name: "Let people reach this computer from outside your network?" });
  await expect(asked).toContainText(
    "To share notes with Bob wherever they are, Oikome needs to open a port on your router.",
  );
  await expect(asked).toContainText("Oikome's own pages and your AI assistants stay on this computer only.");
  await expect(asked).toContainText(
    "The port stays open while you share notes with Bob, and closes when nothing needs it.",
  );
  await asked.getByRole("button", { name: "Deny" }).click();
  await expect(page.locator(".oik-toasts")).toContainText("Bob can reach notes only on your network.");
  await expect(page.getByRole("heading", { name: "Send Bob This Invite Code" })).toBeVisible();
  const invite = (await page.locator(".oik-inset-well .oik-copy__value").textContent()).trim();
  expect(invite).toMatch(/^oik1-/);
  await page.getByRole("button", { name: "Done" }).click();
  const shareRow = page.locator(".screen-table-in-card tr", { hasText: "Bob" });
  await expect(shareRow).toContainText("notes");
  await expect(shareRow).toContainText("View live");
  await expect(shareRow).toContainText("Bob can reach this only on your network.");

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

  // The first run is there too, as the owner's own steps on this computer.
  const mine = page.locator("tbody tr", { hasText: "You" });
  await expect(mine.filter({ hasText: "set up this data center" })).toContainText("This computer");
  // In the owner's words, not a stranger's.
  await expect(page.getByRole("button", { name: "set up this data center", exact: true })).toBeVisible();
  await expect(mine.filter({ hasText: "saved the recovery key" })).toHaveCount(1);
  await expect(mine.filter({ hasText: "added the examples" })).toContainText("spending");
  // The prototype's filter: only one's own steps.
  await page.getByRole("radio", { name: "You" }).click();
  await expect(page.locator("tbody tr", { hasText: "Bob" })).toHaveCount(0);
  // A line opens what it means, and where to act on it.
  await page.getByRole("button", { name: /added the examples/ }).click();
  const line = page.getByRole("dialog", { name: /^You · / });
  await expect(line).toContainText("plain files in your folder");
  await line.getByRole("button", { name: "Open Collections" }).click();
  await expect(page).toHaveURL(`${base}/collections`);
});

test("Home lends this computer to Bob, who adds it and sees when lending is paused", async () => {
  await page.goto(`${base}/sharing`);
  await expect(page.getByText("Not lent to anyone")).toBeVisible();
  await page.getByRole("button", { name: "Change" }).click();
  const lend = page.getByRole("dialog", { name: "Lend This Computer" });
  await lend.getByRole("checkbox", { name: /Bob/ }).check();
  await lend.getByRole("radio", { name: "2 tasks" }).click();
  await lend.getByRole("button", { name: "Save" }).click();
  // Allowed this once; the router of a test run is never asked, so it says it didn't open.
  const asked = page.getByRole("dialog", { name: "Let people reach this computer from outside your network?" });
  await expect(asked).toContainText(
    "The port stays open while you lend this computer to Bob, and closes when nothing needs it.",
  );
  await asked.getByRole("button", { name: "Allow" }).click();
  await expect(page.locator(".oik-toasts")).toContainText(
    "Your router didn't open the port. Turn on UPnP in its settings, then try again.",
  );
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

test("Bob lends Home his computer: big summaries of what he may see go there, the rest stay home", async () => {
  // Bob lends his computer to Home.
  await bob.goto(`${bobBase}/sharing`);
  await bob.getByRole("button", { name: "Change" }).click();
  const lend = bob.getByRole("dialog", { name: "Lend This Computer" });
  await lend.getByRole("checkbox", { name: /Home/ }).check();
  await lend.getByRole("button", { name: "Save" }).click();
  await bob
    .getByRole("dialog", { name: "Let people reach this computer from outside your network?" })
    .getByRole("button", { name: "Deny" })
    .click();
  await expect(bob.getByRole("heading", { name: /This Invite Code$/ })).toBeVisible();
  const code = (await bob.locator(".oik-inset-well .oik-copy__value").textContent()).trim();
  await bob.getByRole("button", { name: "Done" }).click();
  const lendingAtBob = bob.getByRole("switch", { name: "Lend this computer" });
  await expect(lendingAtBob).toHaveAttribute("aria-checked", "true");

  // Home adds it.
  await page.goto(`${base}/sharing`);
  await page.getByRole("button", { name: "Add an Invite" }).click();
  await page.getByRole("dialog", { name: "Add an Invite" }).getByLabel("Invite code").fill(code);
  await page.getByRole("button", { name: "Add Invite" }).click();
  const notice = page.getByRole("region", { name: /lends you their computer$/ });
  await notice.getByRole("button", { name: /^Accept: / }).click();
  const bobs = page.locator(".oik-row", { hasText: "Bob's computer" });
  await expect(bobs).toContainText("Available");

  // A big collection: 100,000 rows over ten days, a euro each.
  const days = Array.from({ length: 10 }, (_, i) => `('2025-01-${String(i + 1).padStart(2, "0")}')`).join(", ");
  const digits = "(VALUES (0), (1), (2), (3), (4), (5), (6), (7), (8), (9))";
  await api("create_space", { name: "ledger" });
  const made = await api("run_sql", {
    sql:
      "CREATE TABLE ledger.ledger (what VARCHAR, day VARCHAR, category VARCHAR, euros DOUBLE); " +
      `INSERT INTO ledger.ledger SELECT 'coffee', a.d, 'food', 1.0 FROM (VALUES ${days}) a(d), ` +
      `${digits} b(x), ${digits} c(x), ${digits} d(x), ${digits} e(x)`,
  });
  expect(made.code).toBeUndefined();
  const { homeCards } = await api("node_info");
  await api("set_home_cards", { cards: [...homeCards, { id: "totals:ledger", width: 7 }] });

  // Turned on, from its details: nothing is shared with Bob yet.
  await bobs.getByRole("button", { name: "Details" }).click();
  const details = page.getByRole("dialog", { name: "Bob's computer" });
  await expect(details).toContainText("Share a collection with Bob to work out its summaries there.");
  await expect(details).toContainText("Big jobs stay on your own devices.");
  await details.getByRole("switch", { name: "Use Bob's computer" }).click();
  await expect(page.locator(".oik-toasts")).toContainText("Big summaries now go to Bob's computer.");
  await details.getByRole("button", { name: "Done" }).click();
  await expect(bobs).toContainText("In use");

  const card = page.getByRole("region", { name: "ledger · last 14 days" });
  await page.goto(base);
  await expect(card).toContainText("Worked out here: it isn't shared with Bob");
  await expect(card).toContainText("100,000 in all");

  // Shared with Bob, who may see all of it: worked out on his computer, to the same totals.
  const bobDid = oikome(contact, "identity").trim();
  const shared = await api("share_space", { space: "ledger", did: bobDid });
  expect(shared.id).toBeTruthy();
  await page.reload();
  await expect(card).toContainText("Worked out on Bob's computer");
  await expect(card).toContainText("100,000 in all · 10 days with any");

  // Bob's Activity says Home ran a task, through lending; Home's says what went where.
  await bob.goto(`${bobBase}/activity`);
  await expect(async () => {
    await bob.reload();
    const line = bob.locator("tbody tr", { hasText: "ran a task on this computer" }).first();
    await expect(line).toBeVisible({ timeout: 1000 });
    await expect(line).toContainText("Lending");
  }).toPass({ timeout: 20_000 });
  await page.goto(`${base}/activity`);
  await expect(async () => {
    await page.reload();
    await expect(page.getByText("worked out ledger's daily totals on Bob's computer").first()).toBeVisible({
      timeout: 1000,
    });
  }).toPass({ timeout: 20_000 });
  await page.goto(`${base}/sharing`);
  await bobs.getByRole("button", { name: "Details" }).click();
  await expect(details).toContainText("1 task");
  await expect(details).toContainText("ledger by day");
  await expect(details).toContainText("Summaries of ledger can go there.");
  await details.getByRole("button", { name: "Done" }).click();

  // Bob stops lending: the next summary (the collection grew) stays home.
  await bob.goto(`${bobBase}/sharing`);
  await lendingAtBob.click();
  await expect(lendingAtBob).toHaveAttribute("aria-checked", "false");
  await api("run_sql", { sql: "INSERT INTO ledger.ledger VALUES ('tea', '2025-01-10', 'food', 2.0)" });
  await page.goto(base);
  await expect(card).toContainText("Worked out here: Bob's computer isn't lending now");
  await expect(card).toContainText("100,002 in all");

  // The share stopped: it stays home, and says why.
  await api("revoke_share", { id: shared.id });
  await page.reload();
  await expect(card).toContainText("Worked out here: it isn't shared with Bob");

  // Back as it was for what follows (a plugin picks the one collection that fits it).
  await api("set_home_cards", { cards: homeCards });
  await api("drop_space", { space: "ledger" });
});

test("Settings › Network says what the port is open for, and Don't ask again stops the question", async () => {
  // Lending was paused and the notes share stopped: what was allowed for them ended with them.
  await page.goto(`${base}/settings`);
  const network = page.getByRole("region", { name: "Network" });
  await expect(network).toContainText("Reachable from outside");
  await expect(network).toContainText("Only on your network");
  await expect(network).toContainText("Nothing now");
  await expect(network.getByLabel("When Oikome needs to be reached from outside")).toHaveValue("ask");

  // Shared again, allowed with Don't ask again: the answer becomes the rule.
  await page.goto(`${base}/sharing`);
  await page.getByRole("button", { name: "New Share" }).click();
  await page.getByRole("dialog", { name: "New Share" }).getByLabel("Collection").selectOption("spending");
  await page.getByRole("button", { name: "Share spending with Bob" }).click();
  const asked = page.getByRole("dialog", { name: "Let people reach this computer from outside your network?" });
  await asked.getByRole("checkbox", { name: "Don't ask again" }).check();
  await asked.getByRole("button", { name: "Allow" }).click();
  await expect(page.getByRole("heading", { name: "Send Bob This Invite Code" })).toBeVisible();
  await page.getByRole("button", { name: "Done" }).click();
  const spending = page.locator(".screen-table-in-card tr", { hasText: "spending" });
  await expect(spending).not.toContainText("only on your network");

  // Not asked any more.
  await page.getByRole("button", { name: "New Share" }).click();
  await page.getByRole("dialog", { name: "New Share" }).getByLabel("Collection").selectOption("notes");
  await page.getByRole("button", { name: "Share notes with Bob" }).click();
  await expect(page.getByRole("heading", { name: "Send Bob This Invite Code" })).toBeVisible();
  await expect(asked).toHaveCount(0);
  await page.getByRole("button", { name: "Done" }).click();

  // Settings says what it is open for, why it isn't reached, and changes it back.
  await page.goto(`${base}/settings`);
  await expect(network).toContainText("Your router didn't allow it");
  await expect(network).toContainText("Turn on UPnP in your router's settings");
  await expect(network).toContainText("sharing spending with Bob");
  await expect(network).toContainText("sharing notes with Bob");
  const policy = network.getByLabel("When Oikome needs to be reached from outside");
  await expect(policy).toHaveValue("always");
  await policy.selectOption("ask");
  await expect(page.locator(".oik-toasts")).toContainText("Saved.");
  await expect(network).toContainText("Nothing now");
  const help = network.getByRole("switch", { name: "Help my contacts reach each other" });
  await expect(help).toHaveAttribute("aria-checked", "true");
  await help.click();
  await expect(help).toHaveAttribute("aria-checked", "false");
  await help.click();
  await expect(help).toHaveAttribute("aria-checked", "true");

  // The shares made meanwhile stop, so the tests after start from a clean slate.
  for (const share of await api("list_shares")) {
    if (share.status === "active" && ["spending", "notes"].includes(share.space)) {
      await api("revoke_share", { id: share.id });
    }
  }
});

test("a folder is viewed where it is: browsed, opened, queried, offline, and shared to view", async () => {
  // A folder of the owner's, outside the data center: a photo, a table, notes in a folder.
  const trip = mkdtempSync(join(tmpdir(), "oikome-trip-"));
  const lisbon = join(owner, "Examples", "lisbon-photos");
  const jpeg = readdirSync(lisbon).find((f) => f.endsWith(".jpg"));
  copyFileSync(join(lisbon, jpeg), join(trip, "Tram 28.jpg"));
  writeFileSync(join(trip, "2025.csv"), "day,euros\n2025-01-01,3.5\n2025-01-02,4\n");
  mkdirSync(join(trip, "Notes"));
  writeFileSync(join(trip, "Notes", "Ferry.md"), "Took the ferry to Cacilhas at sunset.\n");
  const away = `${trip}-away`;
  try {
    // The dialog offers a folder of this computer to read where it is.
    await page.goto(`${base}/collections`);
    await page.getByRole("button", { name: "View a Folder" }).click();
    const dialog = page.getByRole("dialog", { name: "View a Folder" });
    await expect(dialog).toContainText("never changes it");
    await expect(dialog.getByRole("button", { name: "View in Place" })).toBeDisabled();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    // The folder picker browses from the home folder; the test's folder is
    // elsewhere, so it is named directly.
    const viewed = await api("view_in_place", { path: trip });
    expect(viewed.inPlace.files).toBe(3);
    const name = viewed.name;

    // Its page: where it is, its photo from the index, its folders.
    await page.goto(`${base}/collections/${name}`);
    await expect(page.locator(".screen-facts")).toContainText("Read where it is");
    // Its folders first, as covers like a collection's: the note in Notes
    // shows. Then its files, each showing some of itself: a CSV its rows.
    const notesFolder = page.locator(".screen-subfolders .screen-cover", { hasText: "Notes" });
    await expect(notesFolder).toContainText("Took the ferry to Cacilhas at sunset.");
    await expect(page.locator(".screen-ftile__name")).toHaveText(["2025.csv", "Tram 28.jpg"]);
    await expect(page.locator(".screen-ftile", { hasText: "2025.csv" })).toContainText("euros");
    const thumb = page.locator(".screen-ftile img").first();
    await expect.poll(() => thumb.evaluate((img) => img.naturalWidth)).toBeGreaterThan(0);
    expect(await thumb.getAttribute("src")).toMatch(/^\/files\/thumb\//);
    // As a list, by size, the largest first; the folder stays first.
    await page.locator(".screen-folder-bar").getByRole("radio", { name: "List" }).click();
    const files = page.getByRole("table", { name: "Files" });
    await files.getByRole("button", { name: "Size" }).click();
    await expect(files.getByRole("columnheader", { name: /Size/ })).toHaveAttribute("aria-sort", "descending");
    await expect(files.locator("tbody tr").first()).toContainText("Notes");
    await expect(files.locator("tbody tr").nth(1)).toContainText("Tram 28.jpg");
    await page.locator(".screen-folder-bar").getByRole("radio", { name: "Covers" }).click();
    await files.waitFor({ state: "detached" });
    // Opened large, from the folder itself.
    await page.locator("a.screen-ftile").filter({ hasText: "Tram 28.jpg" }).click();
    await expect(page).toHaveURL(/table=files&item=Tram\+28\.jpg/);
    const photo = page.getByRole("dialog", { name: "Tram 28.jpg" });
    const large = photo.locator("img").first();
    await expect.poll(() => large.evaluate((img) => img.naturalWidth)).toBeGreaterThan(512);
    expect(await large.getAttribute("src")).toMatch(/^\/files\/original\//);
    await page.keyboard.press("Escape");
    // Into a folder and back.
    // Into a folder: an address of its own, named in the one crumb at the
    // top; Back comes out, and so does the crumb.
    await notesFolder.click();
    await expect(page).toHaveURL(/\?folder=Notes$/);
    await expect(page.locator(".screen-ftile__text")).toHaveText("Took the ferry to Cacilhas at sunset.");
    const crumb = page.getByRole("navigation", { name: "Where you are" });
    await expect(crumb.locator(".screen-crumb__here")).toHaveText("Notes");
    await expect(crumb.getByRole("link", { name: "Collections" })).toBeVisible();
    await page.goBack();
    await expect(page.locator(".screen-ftile img")).toHaveCount(1);
    await notesFolder.click();
    await crumb.locator(".screen-crumb__up").getByRole("link").last().click();
    await expect(page).toHaveURL(new RegExp(`/collections/${name}$`));
    await expect(page.locator(".screen-ftile img")).toHaveCount(1);
    // Back to the order a folder starts in, for what comes next.
    await page.evaluate(() => localStorage.removeItem("oikome.folder.order"));
    // Its CSV opens as a table, read where it is, not through a table picker.
    await page.locator("a.screen-ftile", { hasText: "2025.csv" }).click();
    const csv = page.getByRole("dialog", { name: "2025.csv" });
    await expect(csv.getByRole("table", { name: "2025.csv" }).getByRole("cell", { name: "Jan 2, 2025" })).toBeVisible();
    await page.keyboard.press("Escape");

    // A photo is sent only to the owner's own pages.
    const outside = await fetch(`${base}/files/thumb/${name}/Tram%2028.jpg`);
    expect(outside.status).toBe(401);

    // Offline: the index still shows it, and says why the files don't open.
    renameSync(trip, away);
    await page.goto(`${base}/collections/${name}`);
    await page.getByRole("button", { name: "Read Again" }).click();
    await expect(page.getByRole("status").filter({ hasText: "can't be reached right now" })).toBeVisible();
    await expect.poll(() => page.locator(".screen-ftile img").first().evaluate((img) => img.naturalWidth)).toBeGreaterThan(0);
    renameSync(away, trip);
    await page.getByRole("button", { name: "Read Again" }).click();
    await expect(page.getByRole("status").filter({ hasText: "can't be reached right now" })).toHaveCount(0);

    // Shared with Bob to view: he queries it live, and can't keep a copy.
    const bobDid = oikome(contact, "identity").trim();
    const refused = await api("share_space", { space: name, did: bobDid, allowCopy: true });
    expect(refused.code).toBe("invalid");
    const shared = await api("share_space", { space: name, did: bobDid });
    const rows = await bob.evaluate(
      async ([code, space]) => {
        const call = async (op, body) =>
          (await fetch(`/api/${op}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })).json();
        const invite = await call("add_invite", { invite: code });
        const accepted = await call("accept_invite", { id: invite.id, name: `${space}-from-home` });
        const tables = await call("list_tables", { space: `${space}-from-home` });
        return { accepted, tables: tables.map((t) => t.name) };
      },
      [shared.invite, name],
    );
    expect(rows.tables).toContain("files");
    expect(rows.tables).toContain("t_2025");
    await api("revoke_share", { id: shared.id });
    await api("drop_space", { space: name });
  } finally {
    if (existsSync(away)) renameSync(away, trip);
    rmSync(trip, { recursive: true, force: true });
  }
});

test("a complex local file opens with the system application, not a shell", async () => {
  test.skip(process.platform === "win32", "the fake opener is a POSIX executable");
  if (!(await page.context().cookies()).length) await page.goto(loginLink());
  const trip = mkdtempSync(join(tmpdir(), "oikome-open-with-"));
  try {
    const file = join(trip, "Portfolio; echo not-a-command.psd");
    writeFileSync(file, new Uint8Array([0, 1, 2, 3]));
    const viewed = await api("view_in_place", { path: trip });
    const opened = await api("open_with", { space: viewed.name, path: "Portfolio; echo not-a-command.psd" });
    expect(opened).toEqual({});
    await page.goto(`${base}/collections/${viewed.name}`);
    await page.locator(".screen-ftile", { hasText: "Portfolio; echo not-a-command.psd" }).click();
    const viewer = page.getByRole("dialog", { name: "Portfolio; echo not-a-command.psd" });
    const managerLabel = process.platform === "darwin" ? "Show in Finder" : process.platform === "win32" ? "Show in File Explorer" : "Show in File Manager";
    await viewer.getByRole("button", { name: managerLabel }).waitFor();
    await viewer.getByRole("button", { name: "Open With…" }).click();
    const log = join(owner, "open-with.log");
    await expect.poll(() => (existsSync(log) ? readFileSync(log, "utf8").trim() : "")).toBe(`1|${realpathSync(file)}`);
  } finally {
    rmSync(trip, { recursive: true, force: true });
  }
});

test("plugins go on Home, read only what they may, and reach nothing", async () => {
  // An outside server, and a stand-in for a STUN server, to hear anything that gets out.
  const heard = [];
  const outside = http.createServer((req, res) => {
    heard.push(req.url);
    res.end("ok");
  });
  await new Promise((r) => outside.listen(0, "127.0.0.1", r));
  const udp = dgram.createSocket("udp4");
  const packets = [];
  udp.on("message", (m) => packets.push(m.length));
  await new Promise((r) => udp.bind(0, "127.0.0.1", r));
  const OUT = `http://127.0.0.1:${outside.address().port}`;
  const probe = mkdtempSync(join(tmpdir(), "oikome-probe-"));
  const said = [];
  const listen = (m) => m.text().startsWith("PROBE ") && said.push(m.text().slice(6));
  try {
    // Oikome's own, added from Plugins: the collection that fits is chosen to start.
    await page.goto(`${base}/plugins`);
    await page.getByRole("button", { name: "Add Spending by category" }).click();
    const adding = page.getByRole("dialog", { name: "Add Spending by category" });
    await expect(adding.getByRole("checkbox", { name: "spending" })).toBeChecked();
    await adding.getByRole("button", { name: "Add to Home" }).click();
    await expect(page.locator(".oik-toasts")).toContainText("Spending by category is on Home.");
    const row = page.locator("tbody tr", { hasText: "Spending by category" });
    await expect(row).toContainText("spending");
    await expect(row).toContainText("None");
    await expect(row).toContainText("On Home");

    // On Home, it shows where the money went (its frame loads when near the view).
    await page.goto(`${base}/`);
    await page.getByRole("region", { name: "Spending by category" }).scrollIntoViewIfNeeded();
    const card = page.frameLocator('iframe[title="Spending by category"]');
    await expect(card.getByText("in 30 days")).toBeVisible();
    await expect(card.getByText("Food", { exact: true })).toBeVisible();

    // The node lets it read nothing else.
    expect((await api("plugin_read", { plugin: "spending-by-category", space: "lisbon-photos" })).code).toBe(
      "permission_denied",
    );

    // A plugin from a folder that tries every way out, through its page.
    writeFileSync(
      join(probe, "oikome-plugin.json"),
      JSON.stringify({ id: "probe", name: "Probe", by: "Test", version: "0.1.0", reads: { what: "Anything", shape: "any" } }),
    );
    writeFileSync(join(probe, "index.html"), '<!doctype html><html><head></head><body><script src="probe.js"></script></body></html>');
    writeFileSync(
      join(probe, "probe.js"),
      `const say = (k, v) => console.log("PROBE " + k + " " + v);
say("rtc", typeof RTCPeerConnection);
try { new RTCPeerConnection({ iceServers: [{ urls: "stun:127.0.0.1:${udp.address().port}" }] }); } catch {}
try { const f = document.createElement("iframe"); document.body.append(f); say("rtc-nested", typeof f.contentWindow.RTCPeerConnection); } catch { say("rtc-nested", "blocked"); }
fetch("/api/node_info", { method: "POST" }).then((r) => say("fetch-api", r.status), () => say("fetch-api", "blocked"));
fetch("${OUT}/fetch").then(() => say("fetch-out", "reached"), () => say("fetch-out", "blocked"));
const img = new Image(); img.onload = () => say("img-out", "reached"); img.onerror = () => say("img-out", "blocked"); img.src = "${OUT}/img";
try { say("cookie", document.cookie); } catch { say("cookie", "blocked"); }
try { say("storage", localStorage.length); } catch { say("storage", "blocked"); }
try { say("parent", parent.document.title); } catch { say("parent", "blocked"); }
oikome.onData(({ collections }) => { say("data", collections.map((c) => c.name).join(",")); oikome.open("lisbon-photos"); });
setTimeout(() => { location.href = "${OUT}/navigate"; }, 1000);`,
    );
    await api("install_plugin", { path: probe, collections: ["spending"] });
    const { homeCards } = await api("node_info");
    await api("set_home_cards", { cards: [...homeCards, { id: "plugin:probe", width: 4 }] });
    page.on("console", listen);
    await page.goto(`${base}/`);
    await page.getByRole("region", { name: "Probe" }).scrollIntoViewIfNeeded();
    await expect.poll(() => said.find((l) => l.startsWith("data ")), { timeout: 15_000 }).toBe("data spending");
    await page.waitForTimeout(2000);
    for (const line of [
      "rtc undefined",
      "rtc-nested blocked",
      "fetch-api blocked",
      "fetch-out blocked",
      "img-out blocked",
      "cookie blocked",
      "storage blocked",
      "parent blocked",
    ]) {
      expect(said).toContain(line);
    }
    expect(heard).toEqual([]);
    expect(packets).toHaveLength(0);
    // Its ask to open a collection it may not read was ignored.
    expect(new URL(page.url()).pathname).toBe("/");
    page.off("console", listen);

    // Activity says which plugin read what.
    await page.goto(`${base}/activity`);
    await expect(
      page.locator("tbody tr", { hasText: "Spending by category" }).filter({ hasText: "read spending" }).first(),
    ).toContainText("Plugin");

    // Removed from Plugins: it reads nothing any more.
    await page.goto(`${base}/plugins`);
    await page.getByRole("button", { name: "Settings: Probe" }).click();
    await page.getByRole("dialog", { name: "Probe" }).getByRole("button", { name: "Remove Plugin" }).click();
    await page.getByRole("dialog", { name: "Remove Probe?" }).getByRole("button", { name: "Remove" }).click();
    await expect(page.locator(".oik-toasts")).toContainText("Removed Probe.");
    expect((await api("plugin_read", { plugin: "probe", space: "spending" })).code).toBe("not_found");
  } finally {
    page.off("console", listen);
    outside.close();
    udp.close();
    rmSync(probe, { recursive: true, force: true });
  }
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
    "/places/this-computer",
    "/collections",
    "/collections/notes",
    "/collections/lisbon-photos",
    "/collections/oikome-guide",
    "/collections/home-climate",
    "/sources",
    "/sources/spending",
    "/sharing",
    "/assistants",
    "/plugins",
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
    ["/places/this-computer", "Local"],
    ["/collections/lisbon-photos", "lisbon-photos"],
    ["/collections/oikome-guide", "oikome-guide"],
    ["/collections/home-climate", "home-climate"],
    ["/sources", "Sources"],
    ["/sources/spending", "spending.csv"],
    ["/collections/notes", "notes"],
    ["/sharing", "Sharing"],
    ["/assistants", "AI Assistants"],
    ["/plugins", "Plugins"],
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
        const { over, culprits } = await sideways();
        expect(over, `${path} at ${width}px (${style}) scrolls sideways: ${culprits}`).toBeLessThanOrEqual(0);
        expect(await page.evaluate(auditContrast), `${path} at ${width}px (${style})`).toEqual([]);
      }
    }
  }
  await page.evaluate(() => localStorage.removeItem("oikome.theme"));
  await page.setViewportSize({ width: 1280, height: 800 });
});

test("the language picked in Settings applies at once and is kept, and every screen fits in each one", async () => {
  await page.goto(`${base}/settings`);
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await page.getByLabel("Language").selectOption("zh-Hans");
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-Hans");
  await expect(page.getByRole("heading", { name: "设置", level: 1 })).toBeVisible();
  await expect(page.getByRole("link", { name: "资料库" })).toBeVisible();
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-Hans");
  await expect(page.getByRole("heading", { name: "设置", level: 1 })).toBeVisible();

  // Longer words (German, Russian) and other scripts still fit, at both
  // widths, in Bold, whose type is the largest.
  const screens = [
    "/onboarding",
    "/",
    "/places/this-computer",
    "/collections",
    "/collections/lisbon-photos",
    "/collections/oikome-guide",
    "/collections/home-climate",
    "/sources",
    "/sources/spending",
    "/sharing",
    "/assistants",
    "/plugins",
    "/activity",
    "/settings",
  ];
  await page.evaluate(() => localStorage.setItem("oikome.theme", "bold"));
  for (const language of ["en-GB", "de", "fr", "ru", "ja", "ko", "zh-Hans", "zh-Hant"]) {
    await page.evaluate((l) => localStorage.setItem("oikome.language", l), language);
    for (const [width, height] of [
      [1280, 800],
      [390, 844],
    ]) {
      await page.setViewportSize({ width, height });
      for (const path of screens) {
        await page.goto(`${base}${path}`);
        await expect(page.locator("html")).toHaveAttribute("lang", language);
        await expect(page.getByRole("heading", { level: 1 }).first()).toBeVisible();
        const { over, culprits } = await sideways();
        expect(over, `${path} at ${width}px in ${language} scrolls sideways: ${culprits}`).toBeLessThanOrEqual(0);
      }
    }
  }
  // With no language picked, the browser's: English here.
  await page.evaluate(() => {
    localStorage.removeItem("oikome.theme");
    localStorage.removeItem("oikome.language");
  });
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto(`${base}/settings`);
  await expect(page.locator("html")).toHaveAttribute("lang", "en");
  await expect(page.getByRole("heading", { name: "Settings", level: 1 })).toBeVisible();
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
