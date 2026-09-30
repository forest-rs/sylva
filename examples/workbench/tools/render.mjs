// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Repeated real-browser workload. Run against the same server, browser and
// machine before/after a change; no FPS or cross-device claims are implied.
import { chromium } from "@playwright/test";
import { preview } from "vite";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import path from "node:path";
const out =
  process.argv[2] ||
  `../../.local/workbench/reviews/${new Date().toISOString().replaceAll(":", "-")}/report.json`;
const args = process.argv.slice(3);
const seedOption = args.find((arg) => arg.startsWith("--seed="));
const seed = Number(seedOption?.slice(7) || process.env.WORKBENCH_SEED || 1);
if (!Number.isInteger(seed) || seed < 0 || seed > 0xffffffff)
  throw new Error("Seed must be an unsigned 32-bit integer.");
const speciesList = args.filter((arg) => !arg.startsWith("--seed="));
if (!speciesList.length) speciesList.push("birch", "oak", "spruce");
if (speciesList.some((s) => !["birch", "beech", "oak", "spruce"].includes(s)))
  throw new Error("Species must be birch, beech, oak or spruce.");
let server, browser;
try {
  // Own the production preview's lifetime unless reviewing an explicit URL.
  if (!process.env.WORKBENCH_URL)
    server = await preview({ preview: { host: "127.0.0.1", port: 0 } });
  const url = process.env.WORKBENCH_URL || server.resolvedUrls.local[0];
  browser = await chromium.launch({
    channel: process.env.PLAYWRIGHT_CHANNEL || "chrome",
  });
  const results = {
    browser: browser.version(),
    url,
    viewport: [1440, 1000],
    samples: [],
  };
  for (const species of speciesList) {
    const page = await browser.newPage({
      viewport: { width: 1440, height: 1000 },
    });
    const recipe = { version: 1, species, seed, density: 1, leaf_size: 1 };
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
        recipe: report.recipe,
        source_revision: report.source_revision,
        preview: report.preview,
        counts: report.counts,
        timings: report.timings,
        material_cache: report.material_cache,
        worker_memory_bytes: report.worker_memory_bytes,
      });
      console.log(JSON.stringify(results.samples.at(-1)));
      if (iteration === 2) {
        await mkdir(path.dirname(out), { recursive: true });
        await page.screenshot({
          path: out.replace(".json", `-${species}-canopy.png`),
        });
        await page.locator('[data-mode="structure"]').click();
        await page.screenshot({
          path: out.replace(".json", `-${species}-whole-structure.png`),
        });
        await page.locator('[data-mode="canopy"]').click();
        await page.mouse.move(865, 550);
        await page.mouse.wheel(0, -1200);
        await page.waitForTimeout(700);
        await page.screenshot({
          path: out.replace(".json", `-${species}-interior.png`),
        });
        await page.locator('[data-mode="structure"]').click();
        await page.screenshot({
          path: out.replace(".json", `-${species}-structure.png`),
        });
      }
    }
    await page.close();
  }
  await mkdir(path.dirname(out), { recursive: true });
  await writeFile(out, JSON.stringify(results, null, 2) + "\n");
} finally {
  await browser?.close();
  await server?.close();
}
