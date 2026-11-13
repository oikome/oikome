// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { launchAcceptance } from "./acceptance-node.mjs";

test("acceptance opens real content, storages and a shared peer", async ({ page }) => {
  test.setTimeout(120_000);
  const parent = mkdtempSync(join(tmpdir(), "oikome-acceptance-"));
  const openerBin = join(parent, "opener-bin");
  mkdirSync(openerBin);
  const openerLog = join(parent, "opener.log");
  if (process.platform !== "win32") {
    writeFileSync(
      join(openerBin, process.platform === "darwin" ? "open" : "xdg-open"),
      `#!/bin/sh\nprintf '%s|%s\\n' "$#" "$*" > "${openerLog}"\n`,
      { mode: 0o755 },
    );
  }
  const running = await launchAcceptance({
    root: join(parent, "workspace"),
    env: { PATH: `${openerBin}:${process.env.PATH ?? ""}` },
  });
  try {
    await page.goto(running.loginLink);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    await page.goto(`${running.base}/collections`);
    await expect(page.locator(".screen-cover")).toHaveCount(7);

    await page.goto(`${running.base}/collections/media`);
    await page.locator("a.screen-ftile", { hasText: "tram.webm" }).click();
    const viewer = page.getByRole("dialog", { name: "tram.webm" });
    const video = viewer.locator("video");
    await expect.poll(() => video.evaluate((element) => element.readyState)).toBeGreaterThan(1);
    await viewer.getByRole("button", { name: "Close" }).click();
    await page.locator("a.screen-ftile", { hasText: "route.svg" }).click();
    const svg = page.getByRole("dialog", { name: "route.svg" }).locator(".screen-svg-view img");
    await expect.poll(() => svg.evaluate((element) => element.naturalWidth)).toBe(640);

    await page.goto(`${running.base}/collections/content-files`);
    const opened = page.waitForResponse((response) => response.url().endsWith("/api/show_folder"));
    await page.locator(".screen-facts__source").click();
    expect(await (await opened).json()).toEqual({});
    await expect(page.locator("a.screen-ftile", { hasText: "damaged.json" })).toBeVisible();
    await expect(page.locator("a.screen-ftile", { hasText: "unsafe.svg" })).toBeVisible();
    const pdf = readFileSync(join(running.files, "content-files", "External.pdf"), "utf8");
    expect(pdf).toContain("/Type /Page");
    expect(pdf).toContain("startxref");
    expect(pdf.trimEnd().endsWith("%%EOF")).toBe(true);
    const psd = readFileSync(join(running.files, "content-files", "Portfolio.psd"));
    expect(psd.toString("ascii", 0, 4)).toBe("8BPS");
    expect(psd.readUInt16BE(10)).toBe(3);
    expect(psd.readUInt16BE(38)).toBe(0);

    for (const collection of ["readings", "drive-reading", "rustfs-archive"]) {
      await page.goto(`${running.base}/collections/${collection}`);
      await expect(page.getByRole("heading", { name: collection, level: 1 })).toBeVisible();
      await expect(page.locator(".screen-facts")).toContainText(collection === "readings" ? "Local" : collection === "drive-reading" ? "Drive Sync Folder" : "RustFS Acceptance");
    }
    await page.goto(`${running.base}/collections/readings`);
    await expect(page.locator(".screen-series")).toBeVisible();
    await page.goto(`${running.base}/collections/notes`);
    await expect(page.getByText("Lisbon, slowly").first()).toBeVisible();
    await page.goto(`${running.base}/collections/photos`);
    await expect(page.locator(".screen-facts")).toContainText("NAS Mounted Folder");
    await page.locator("a.screen-ftile", { hasText: "Tram 28E.jpg" }).click();
    const photo = page.getByRole("dialog", { name: "Tram 28E.jpg" }).locator("img").first();
    await expect.poll(() => photo.evaluate((element) => element.naturalWidth)).toBeGreaterThan(512);
    await page.keyboard.press("Escape");
    await page.goto(`${running.base}/collections/drive-reading`);
    await expect(page.getByText("The Book of Disquiet").first()).toBeVisible();
    await page.goto(`${running.base}/collections/rustfs-archive`);
    await expect(page.getByText("Family recipes scanned").first()).toBeVisible();
    await page.goto(`${running.base}/sharing`);
    await expect(page.getByText("Acceptance Peer").first()).toBeVisible();

    await page.goto(running.peerLoginLink);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    await page.goto(`${running.peerBase}/collections/photos-from-acceptance`);
    await expect(page.getByRole("heading", { name: "photos-from-acceptance", level: 1 })).toBeVisible();
    await expect(page.locator(".screen-facts")).toContainText("Oikome Acceptance");
    await expect(page.locator(".screen-facts")).toContainText("View live");
  } finally {
    await running.stop();
    rmSync(parent, { recursive: true, force: true });
  }
});

test("first run really starts empty and creates the ordinary examples through the UI", async ({ page }) => {
  test.setTimeout(120_000);
  const parent = mkdtempSync(join(tmpdir(), "oikome-acceptance-first-"));
  const running = await launchAcceptance({ root: join(parent, "first"), firstRun: true });
  try {
    await page.goto(running.loginLink);
    await expect(page.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
    await page.getByLabel("Name", { exact: true }).fill("Acceptance First Run");
    await page.getByRole("button", { name: "Continue" }).click();
    await expect(page.getByRole("heading", { name: "Save Your Recovery Key" })).toBeVisible();
    await page.getByLabel("I've stored it somewhere safe").check();
    await page.getByRole("button", { name: "Continue" }).click();
    await page.getByRole("button", { name: "Get Started" }).click();
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    await page.goto(`${running.base}/collections`);
    await expect(page.locator(".screen-cover")).toHaveCount(4);
  } finally {
    await running.stop();
    rmSync(parent, { recursive: true, force: true });
  }
});

test("restarting is clean and an unmarked directory is never replaced", async ({ page }) => {
  test.setTimeout(120_000);
  const parent = mkdtempSync(join(tmpdir(), "oikome-acceptance-repeat-"));
  const root = join(parent, "acceptance");
  try {
    for (let visit = 0; visit < 2; visit++) {
      const running = await launchAcceptance({ root, firstRun: true });
      try {
        await page.goto(running.loginLink);
        await expect(page.getByRole("heading", { name: "Welcome to Oikome" })).toBeVisible();
      } finally {
        await running.stop();
      }
    }
    const personal = join(parent, "personal");
    mkdirSync(personal);
    writeFileSync(join(personal, "keep.txt"), "do not replace");
    await expect(launchAcceptance({ root: personal, firstRun: true })).rejects.toThrow("refusing to replace it");
    expect(readFileSync(join(personal, "keep.txt"), "utf8")).toBe("do not replace");
  } finally {
    rmSync(parent, { recursive: true, force: true });
  }
});
