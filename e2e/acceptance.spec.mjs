// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
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
  const pdfRequests = [];
  page.on("request", (request) => {
    if (/PdfView|pdf\.worker|\/pdf\//.test(request.url())) pdfRequests.push(request.url());
  });
  try {
    await page.goto(running.loginLink);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    await page.goto(`${running.base}/collections`);
    await expect(page.locator(".screen-cover")).toHaveCount(7);
    expect(pdfRequests).toHaveLength(0);
    const mediaCover = page.locator('a.screen-cover[href="/collections/media"]');
    await expect(mediaCover.locator(".screen-file-collage")).toBeVisible();
    await expect(mediaCover.locator(".screen-cover__empty")).toHaveCount(0);
    await expect(mediaCover).toContainText("3 files");
    await expect.poll(() => mediaCover.locator(".screen-svg-thumbnail img").evaluate((image) => image.naturalWidth)).toBe(640);
    const coverVideo = mediaCover.locator(".screen-video-thumbnail video");
    await expect.poll(() => coverVideo.evaluate((element) => element.readyState)).toBeGreaterThan(0);
    await expect.poll(() => coverVideo.evaluate((element) => element.videoWidth)).toBe(480);
    await expect(coverVideo.evaluate((element) => element.paused)).resolves.toBe(true);
    await expect(mediaCover.locator(".screen-audio-thumbnail")).toBeVisible();

    await page.goto(`${running.base}/collections/media`);
    const routeTile = page.locator("a.screen-ftile", { hasText: "route.svg" });
    await expect.poll(() => routeTile.locator(".screen-svg-thumbnail img").evaluate((image) => image.naturalWidth)).toBe(640);
    const videoTile = page.locator("a.screen-ftile", { hasText: "tram.webm" });
    await expect.poll(() => videoTile.locator("video").evaluate((element) => element.videoWidth)).toBe(480);
    await expect(videoTile.locator("video").evaluate((element) => element.paused)).resolves.toBe(true);
    await expect(page.locator("a.screen-ftile", { hasText: "field-recording.wav" }).locator(".screen-audio-thumbnail")).toBeVisible();
    await videoTile.click();
    const viewer = page.getByRole("dialog", { name: "tram.webm" });
    const video = viewer.locator("video");
    await expect.poll(() => video.evaluate((element) => element.readyState)).toBeGreaterThan(1);
    await viewer.getByRole("button", { name: "Close" }).click();
    await page.locator("a.screen-ftile", { hasText: "route.svg" }).click();
    const svg = page.getByRole("dialog", { name: "route.svg" }).locator(".screen-svg-view img");
    await expect.poll(() => svg.evaluate((element) => element.naturalWidth)).toBe(640);

    await page.goto(`${running.base}/collections/content-files`);
    await expect(page.locator(".screen-facts__source")).toHaveCSS("border-bottom-style", "solid");
    await expect(page.locator(".screen-facts__source")).toHaveCSS("white-space", "nowrap");
    const fullSource = realpathSync(join(running.files, "content-files"));
    const sourceRoot = /^\/(?:Users|home)\/[^/]+(?=\/|$)/.test(fullSource) ? "~/" : "/";
    await expect(page.locator(".screen-facts__source .screen-path-value__root")).toHaveText(sourceRoot);
    const sourceTail = page.locator(".screen-facts__source .screen-path-value__tail");
    await expect(sourceTail).toHaveText("files/content-files");
    await expect.poll(() => sourceTail.evaluate((element) => element.scrollWidth <= element.clientWidth)).toBe(true);
    await expect(page.locator(".screen-facts__source")).toHaveAttribute("title", realpathSync(join(running.files, "content-files")));
    const opened = page.waitForResponse((response) => response.url().endsWith("/api/show_folder"));
    await page.locator(".screen-facts__source").click();
    expect(await (await opened).json()).toEqual({});
    await expect(page.locator("a.screen-ftile", { hasText: "damaged.json" })).toBeVisible();
    const unsafeTile = page.locator("a.screen-ftile", { hasText: "unsafe.svg" });
    await expect(unsafeTile).toBeVisible();
    const unsafeThumbnail = unsafeTile.locator(".screen-svg-thumbnail img");
    await expect(unsafeThumbnail).toHaveAttribute("src", /^blob:/);
    await expect.poll(() => unsafeThumbnail.evaluate((image) => image.naturalWidth)).toBe(320);
    const externalTile = page.locator("a.screen-ftile", { hasText: "External.pdf" });
    const thumbnail = externalTile.locator(".screen-pdf-thumbnail__canvas");
    await expect(thumbnail).toBeVisible();
    await expect
      .poll(() =>
        thumbnail.evaluate((canvas) => {
          const context = canvas.getContext("2d");
          if (!context || !canvas.width || !canvas.height) return false;
          const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
          for (let i = 0; i < pixels.length; i += 4) if (pixels[i] < 100 && pixels[i + 3] > 0) return true;
          return false;
        }),
      )
      .toBe(true);
    await expect(externalTile.locator(".screen-pdf__toolbar")).toHaveCount(0);
    const brokenTile = page.locator("a.screen-ftile", { hasText: "Broken.pdf" });
    await expect(brokenTile.locator(".screen-pdf-thumbnail__canvas")).toHaveCount(0);
    await expect(brokenTile.locator(".screen-pdf-thumbnail .oik-icon")).toBeVisible();
    expect(pdfRequests.length).toBeGreaterThan(0);
    await page.locator("a.screen-ftile", { hasText: "Plain.txt" }).click();
    const plain = page.getByRole("dialog", { name: "Plain.txt" });
    const copyButton = plain.getByRole("button", { name: "Copy", exact: true });
    const editButton = plain.getByRole("button", { name: "Edit", exact: true });
    await expect(editButton).toBeVisible();
    expect((await copyButton.boundingBox()).x).toBeLessThan((await editButton.boundingBox()).x);
    await expect.poll(() => plain.locator(".screen-fileview__actions").evaluate((element) => element.getBoundingClientRect().left > element.parentElement.getBoundingClientRect().width / 2)).toBe(true);
    await page.keyboard.press("Escape");
    for (const name of ["example.ts", "config.yaml"]) {
      await page.locator("a.screen-ftile", { hasText: name }).click();
      const source = page.getByRole("dialog", { name });
      await expect(source.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
      await expect(source.locator(".cm-content")).toBeVisible();
      await page.keyboard.press("Escape");
    }
    await page.locator("a.screen-ftile", { hasText: "rows.csv" }).click();
    const csv = page.getByRole("dialog", { name: "rows.csv" });
    await csv.getByRole("radio", { name: "Source", exact: true }).click();
    await expect(csv.locator(".cm-content")).toContainText("day,category,amount");
    await expect(csv.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    await page.locator("a.screen-ftile", { hasText: "Read only.txt" }).click();
    const readOnly = page.getByRole("dialog", { name: "Read only.txt" });
    await readOnly.getByRole("button", { name: "Edit", exact: true }).click();
    await expect(readOnly.getByRole("button", { name: "Save a Copy", exact: true })).toBeVisible();
    await expect(readOnly.getByRole("button", { name: "Save Original", exact: true })).toHaveCount(0);
    await expect(readOnly.getByText("The original is read-only. Save a copy to keep your changes.")).toBeVisible();
    await page.keyboard.press("Escape");
    await page.locator("a.screen-ftile", { hasText: "External.pdf" }).click();
    const pdfViewer = page.getByRole("dialog", { name: "External.pdf" });
    await expect(pdfViewer.locator(".screen-pdf__canvas")).toBeVisible();
    await expect(pdfViewer.locator(".screen-pdf__text")).toContainText("Oikome acceptance document");
    await expect(pdfViewer.locator(".screen-pdf__position").first()).toHaveText("Page 1 of 3");
    await expect(pdfViewer.getByRole("button", { name: "Edit", exact: true })).toHaveCount(0);
    await pdfViewer.getByRole("button", { name: "Next page", exact: true }).click();
    await expect(pdfViewer.locator(".screen-pdf__text")).toContainText("A second page");
    await pdfViewer.getByRole("button", { name: "Next page", exact: true }).click();
    await expect(pdfViewer.locator(".screen-pdf__text")).toContainText("中文预览");
    expect(pdfRequests.some((url) => url.includes("/pdf/cmaps/"))).toBe(true);
    await page.setViewportSize({ width: 390, height: 844 });
    await expect.poll(() => pdfViewer.locator(".screen-pdf__canvas").evaluate((canvas) => {
      const frame = canvas.parentElement?.parentElement;
      return canvas.isConnected && frame ? canvas.getBoundingClientRect().width <= frame.clientWidth : false;
    })).toBe(true);
    await page.setViewportSize({ width: 1440, height: 1000 });
    await pdfViewer.getByRole("button", { name: "Zoom in", exact: true }).click();
    await expect(pdfViewer.locator(".screen-pdf__zoom")).toContainText("125%");
    expect(pdfRequests.length).toBeGreaterThan(0);
    await page.keyboard.press("Escape");
    const originalResponse = await page.request.get(`${running.base}/files/original/content-files/External.pdf`, { headers: { Range: "bytes=0-3" } });
    expect(originalResponse.status()).toBe(206);
    expect(originalResponse.headers()["content-security-policy"]).toBe("default-src 'none'");
    expect(await originalResponse.text()).toBe("%PDF");
    await page.locator("a.screen-ftile", { hasText: "Broken.pdf" }).click();
    await expect(page.getByRole("dialog", { name: "Broken.pdf" })).toContainText("This PDF can’t be previewed. Try opening it in its default app.");
    await page.keyboard.press("Escape");
    const pdf = readFileSync(join(running.files, "content-files", "External.pdf"), "utf8");
    expect(pdf).toContain("/Type /Page");
    expect(pdf).toContain("startxref");
    expect(pdf.trimEnd().endsWith("%%EOF")).toBe(true);
    const psd = readFileSync(join(running.files, "content-files", "Portfolio.psd"));
    expect(psd.toString("ascii", 0, 4)).toBe("8BPS");
    expect(psd.readUInt16BE(10)).toBe(3);
    expect(psd.readUInt16BE(38)).toBe(0);

    for (const collection of ["readings", "reading-list", "rustfs-archive"]) {
      await page.goto(`${running.base}/collections/${collection}`);
      await expect(page.getByRole("heading", { name: collection, level: 1 })).toBeVisible();
      await expect(page.locator(".screen-facts")).toContainText(collection === "readings" ? "Local" : collection === "reading-list" ? "Drive Sync Folder" : "RustFS Acceptance");
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
    await page.goto(`${running.base}/collections/reading-list`);
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
