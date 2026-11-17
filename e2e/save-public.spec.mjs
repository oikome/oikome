// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { launchAcceptance } from "./acceptance-node.mjs";

test("Save To and public sharing work end to end without an owner session", async ({ page, browser }) => {
  test.setTimeout(120_000);
  const parent = mkdtempSync(join(tmpdir(), "oikome-save-public-"));
  const running = await launchAcceptance({ root: join(parent, "workspace") });
  try {
    await page.goto(running.loginLink);
    await page.goto(`${running.base}/collections/notes`);
    await page.getByRole("button", { name: "More actions for notes" }).click();
    await page.getByRole("menuitem", { name: "Save To…" }).click();
    const save = page.getByRole("dialog", { name: "Save notes" });
    await save.getByLabel("Collection name").fill("weekend-saved");
    await save.getByLabel("Title", { exact: true }).fill("Weekend");
    await save.getByRole("button", { name: "Save once" }).click();
    await expect(page).toHaveURL(/\/collections\/weekend-saved$/);
    await expect(page.getByRole("heading", { name: "weekend-saved", level: 1 })).toBeVisible();
    await expect(page.getByText("Lisbon, slowly").first()).toBeVisible();

    await page.getByRole("button", { name: "More actions for weekend-saved" }).click();
    await page.getByRole("menuitem", { name: "Public share…" }).click();
    const publish = page.getByRole("dialog", { name: "Share weekend-saved publicly" });
    await publish.getByLabel("Visitor access").selectOption("copy");
    await publish.getByRole("button", { name: "Publish", exact: true }).click();
    const reach = page.getByRole("dialog", { name: "Let people reach this computer from outside your network?" });
    await expect(reach).toBeVisible();
    await reach.getByRole("button", { name: "Deny", exact: true }).click();
    const ready = page.getByRole("dialog", { name: "Public share ready" });
    await expect(ready).toBeVisible();
    const link = await ready.locator(".oik-copy__value").textContent();

    const visitor = await browser.newPage();
    await visitor.goto(link);
    await expect(visitor.getByRole("heading", { name: "Weekend", level: 1 })).toBeVisible();
    await expect(visitor.getByRole("link", { name: "Download signed snapshot" })).toBeVisible();
    await visitor.getByRole("button", { name: "Use a temporary key" }).click();
    await expect(visitor.getByText("Signed in; live queries are available")).toBeVisible();
    await expect(visitor.getByText("The live query returned 1 files.")).toBeVisible();
    const id = new URL(link).pathname.split("/").at(-1);
    const snapshot = await visitor.request.get(`${running.base}/public/${id}/snapshot.zip`);
    expect(snapshot.status()).toBe(200);
    expect(snapshot.headers()["content-type"]).toBe("application/zip");
    const original = await visitor.request.get(`${running.base}/public/${id}/files/Reading.md`);
    expect(original.status()).toBe(200);
    expect(await original.text()).toContain("Lisbon, slowly");
    await visitor.close();

    await ready.getByRole("button", { name: "Done" }).click();
    await page.getByRole("button", { name: "More actions for weekend-saved" }).click();
    await page.getByRole("menuitem", { name: "Public share…" }).click();
    await page.getByRole("button", { name: "End share", exact: true }).click();
    await page.getByRole("button", { name: "Close", exact: true }).click();
    const revoked = await page.request.get(`${running.base}/public/${id}/manifest.json`);
    expect(revoked.status()).toBe(404);

    await page.goto(`${running.base}/collections/notes`);
    await page.getByRole("button", { name: "More actions for notes" }).click();
    await page.getByRole("menuitem", { name: "Save To…" }).click();
    const plain = page.getByRole("dialog", { name: "Save notes" });
    await plain.getByLabel("Destination", { exact: true }).selectOption("files");
    await plain.getByLabel("Destination folder").fill(join(running.files, "exports"));
    await plain.getByLabel("Folder naming rule").fill("");
    await plain.getByRole("button", { name: "Save once" }).click();
    await expect(plain).toHaveCount(0);
    expect(readFileSync(join(running.files, "exports", "reading.md"), "utf8")).toContain("Lisbon, slowly");
  } finally {
    await running.stop();
    rmSync(parent, { recursive: true, force: true });
  }
});
