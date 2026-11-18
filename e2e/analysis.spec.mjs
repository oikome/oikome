// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { launchAcceptance } from "./acceptance-node.mjs";

test("an owner reviews fields and creates a derived analysis without changing the source", async ({ page }) => {
  test.setTimeout(120_000);
  const parent = mkdtempSync(join(tmpdir(), "oikome-analysis-"));
  const running = await launchAcceptance({ root: join(parent, "workspace") });
  try {
    await page.goto(running.loginLink);
    const call = (operation, body) => page.evaluate(async ([operation, body]) => {
      const response = await fetch(`/api/${operation}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!response.ok) throw new Error(await response.text());
      return response.json();
    }, [operation, body]);
    await call("create_space", { name: "spending", kind: "transactional" });
    await call("run_sql", {
      sql: "CREATE TABLE spending.entries (id BIGINT, day DATE, amount DOUBLE, category VARCHAR, title VARCHAR); INSERT INTO spending.entries VALUES (1, '2026-01-01', 12.5, 'food', 'lunch'), (2, '2026-01-01', 17.5, 'food', 'dinner'), (3, '2026-01-02', 8, 'travel', 'metro');",
    });
    await page.goto(`${running.base}/collections/spending`);
    await page.getByRole("button", { name: "More actions for spending" }).click();
    await page.getByRole("menuitem", { name: "Analyze…" }).click();
    const dialog = page.getByRole("dialog", { name: "Analyze spending.entries" });
    await expect(dialog.getByLabel("Date field")).toHaveValue("day");
    await expect(dialog.getByLabel("Amount field")).toHaveValue("amount");
    await expect(dialog.getByLabel("Category field")).toHaveValue("category");
    await dialog.getByRole("button", { name: "Create analysis" }).click();
    await expect(page).toHaveURL(/\/collections\/spending-analysis$/);
    await expect(page.getByRole("heading", { name: "spending-analysis", level: 1 })).toBeVisible();
    await expect(page.locator(".oik-table").first()).toContainText("30");
    await expect(page.locator(".oik-table").first()).toContainText("food");
    const source = await call("list_tables", { space: "spending" });
    expect(source.find((table) => table.name === "entries").rows).toBe(3);
    const profile = await call("analysis_profile", { space: "spending-analysis" });
    expect(profile.sourceChanged).toBe(false);
  } finally {
    await running.stop();
    rmSync(parent, { recursive: true, force: true });
  }
});
