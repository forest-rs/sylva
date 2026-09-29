// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Repeated real-browser workload. Run against the same server, browser and
// machine before/after a change; no FPS or cross-device claims are implied.
import { chromium } from "@playwright/test";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import path from "node:path";
const out = process.argv[2] || "../../.local/workbench/latency.json";
const browser = await chromium.launch({
  channel: process.env.PLAYWRIGHT_CHANNEL || "chrome",
});
const url = process.env.WORKBENCH_URL || "http://127.0.0.1:5173";
const results = {
  browser: browser.version(),
  url,
  viewport: [1440, 1000],
  samples: [],
};
try {
  for (const species of ["birch", "oak", "spruce"]) {
    const page = await browser.newPage({
      viewport: { width: 1440, height: 1000 },
    });
    const recipe = { version: 1, species, seed: 1, density: 1, leaf_size: 1 };
    await page.goto(
      url + "/#recipe=" + encodeURIComponent(JSON.stringify(recipe)),
    );
    for (let iteration = 0; iteration < 3; iteration++) {
      if (iteration) await page.locator("#grow").click();
      await page.waitForFunction(
        () => document.querySelector("#loading").hidden,
        {},
        { timeout: 120000 },
      );
      const status = await page.locator("#status").innerText();
      if (
        await page
          .locator("#status")
          .evaluate((el) => el.classList.contains("error"))
      )
        throw new Error(status);
      await page.locator("#diagnostics").click();
      const event = page.waitForEvent("download");
      await page.locator("#download-report").click();
      const report = JSON.parse(
        await readFile(await (await event).path(), "utf8"),
      );
      await page.locator("#close-report").click();
      results.samples.push({
        species,
        iteration,
        counts: report.counts,
        timings: report.timings,
      });
      console.log(JSON.stringify(results.samples.at(-1)));
      if (species === "oak" && iteration === 2) {
        await mkdir(path.dirname(out), { recursive: true });
        await page.mouse.move(865, 550);
        await page.mouse.wheel(0, -1200);
        await page.waitForTimeout(700);
        await page.screenshot({
          path: out.replace(".json", "-oak-interior.png"),
        });
        await page.locator('[data-mode="structure"]').click();
        await page.screenshot({
          path: out.replace(".json", "-oak-structure.png"),
        });
      }
    }
    await page.close();
  }
  await mkdir(path.dirname(out), { recursive: true });
  await writeFile(out, JSON.stringify(results, null, 2) + "\n");
} finally {
  await browser.close();
}
