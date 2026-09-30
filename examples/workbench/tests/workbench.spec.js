// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

async function ready(page) {
  await expect(page.locator("#loading")).toBeHidden({ timeout: 90_000 });
  await expect(page.locator("#status")).not.toHaveClass(/error/);
  await expect(page.locator("#export")).toBeEnabled();
}
async function download(page, selector) {
  const event = page.waitForEvent("download");
  await page.locator(selector).click();
  return readFile(await (await event).path());
}

test("generate, inspect, edit, export and recover without losing the specimen", async ({
  page,
}, testInfo) => {
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/");
  await ready(page);
  const initialBranches = await page.locator("#branches").innerText();
  const initialPlacements = await page.locator("#placements").innerText();
  const glb = await download(page, "#export");
  expect(glb.subarray(0, 4).toString()).toBe("glTF");
  const jsonLength = glb.readUInt32LE(12);
  const gltf = JSON.parse(glb.subarray(20, 20 + jsonLength).toString());
  expect(gltf.extensionsUsed).toContain("EXT_mesh_gpu_instancing");
  expect(
    gltf.meshes.some((mesh) =>
      mesh.primitives.some((p) => "_BRANCH" in p.attributes),
    ),
  ).toBeTruthy();
  const recipeBytes = await download(page, "#save-recipe");
  const recipe = JSON.parse(recipeBytes);
  expect(recipe.species).toBe("birch");
  await page.screenshot({
    path: testInfo.outputPath("birch.png"),
    fullPage: true,
  });
  await page.locator('[data-mode="structure"]').click();
  // Click the lower trunk in the deterministic initial camera.
  await page.locator("canvas").click({ position: { x: 565, y: 665 } });
  await expect(page.locator("#selection")).toBeVisible();
  await expect(page.locator("#branch-id")).toHaveText(/^[0-9a-f]{16}$/);
  await page.locator("#focus-branch").click();
  await page.screenshot({
    path: testInfo.outputPath("branch-inspection.png"),
    fullPage: true,
  });
  await page.locator("#fit").click();
  await page.locator('[data-mode="canopy"]').click();
  await page.locator("#density").fill("50");
  await page.locator("#grow").click();
  await ready(page);
  expect(await page.locator("#branches").innerText()).toBe(initialBranches);
  expect(
    Number((await page.locator("#placements").innerText()).replaceAll(",", "")),
  ).toBeLessThan(Number(initialPlacements.replaceAll(",", "")));
  // An invalid import must not discard the last successful tree.
  const validCounts = await page.locator("#placements").innerText();
  await page.locator("#recipe-file").setInputFiles({
    name: "invalid.json",
    mimeType: "application/json",
    buffer: Buffer.from('{"version":99}'),
  });
  await expect(page.locator("#status")).toHaveClass(/error/);
  expect(await page.locator("#placements").innerText()).toBe(validCounts);
  await expect(page.locator("#export")).toBeEnabled();
  // Restore the saved authoring recipe and prove deterministic output.
  await page.locator("#recipe-file").setInputFiles({
    name: "birch.json",
    mimeType: "application/json",
    buffer: recipeBytes,
  });
  await ready(page);
  expect(await download(page, "#export")).toEqual(glb);
  await page.locator("#diagnostics").click();
  const report = JSON.parse(await download(page, "#download-report"));
  expect(report.counts.placements).toBe(39818);
  expect(report.counts.templates).toBe(4);
  expect(report.timings.generation_export_ms).toBeGreaterThan(0);
  expect(report.timings.click_to_frame_ms).toBeGreaterThanOrEqual(
    report.timings.generation_export_ms,
  );
  expect(report.timings.click_to_frame_ms).toBeGreaterThanOrEqual(
    report.timings.click_to_submit_ms,
  );
  expect(report.timings.gpu_completion_wait_ms).toBeGreaterThanOrEqual(0);
  expect(report.material_cache.bark_hit).toBe(true);
  expect(report.material_cache.leaf_hit).toBe(true);
  expect(report.worker_memory_bytes).toBeGreaterThan(0);
  for (const phase of ["source_ms", "realization_ms", "export_ms"])
    expect(report.timings[phase]).toBeGreaterThan(0);
  expect(report.timings.textures_ms).toBeGreaterThanOrEqual(0);
  await page.locator("#close-report").click();
  await page.locator('[data-species="spruce"]').click();
  await page.locator("#grow").click();
  await page.locator("#cancel").click();
  await expect(page.locator("#status")).toContainText("cancelled");
  await expect(page.locator("#specimen-name")).toHaveText("Silver birch");
  // Cancellation discards the worker/cache, but a new worker can rebuild the
  // same displayed recipe and export exactly the same specimen.
  await page.locator('[data-species="birch"]').click();
  await page.locator("#grow").click();
  await ready(page);
  expect(await download(page, "#export")).toEqual(glb);
  await page.locator("#diagnostics").click();
  const restarted = JSON.parse(await download(page, "#download-report"));
  expect(restarted.material_cache.bark_hit).toBe(false);
  expect(restarted.material_cache.leaf_hit).toBe(false);
  expect(errors).toEqual([]);
});

test("static views stop drawing and camera motion resumes drawing", async ({
  page,
}) => {
  await page.goto("/");
  await ready(page);
  await page.locator("#diagnostics").click();
  const before = JSON.parse(await download(page, "#download-report"));
  await page.waitForTimeout(500);
  const idle = JSON.parse(await download(page, "#download-report"));
  expect(idle.preview.submitted_frames).toBe(before.preview.submitted_frames);
  await page.locator("#close-report").click();
  await page.locator("#rotate").click();
  await page.waitForTimeout(300);
  await page.locator("#rotate").click();
  await page.locator("#diagnostics").click();
  const moved = JSON.parse(await download(page, "#download-report"));
  expect(moved.preview.submitted_frames).toBeGreaterThan(
    idle.preview.submitted_frames,
  );
});

for (const species of ["oak", "beech", "spruce"]) {
  test(`${species} renders real geometry and saves a report`, async ({
    page,
  }, testInfo) => {
    const recipe = { version: 1, species, seed: 1, density: 1, leaf_size: 1 };
    await page.goto("/#recipe=" + encodeURIComponent(JSON.stringify(recipe)));
    await ready(page);
    await page.screenshot({
      path: testInfo.outputPath(`${species}.png`),
      fullPage: true,
    });
    await page.locator("#diagnostics").click();
    const bytes = await download(page, "#download-report");
    const report = JSON.parse(bytes);
    expect(report.recipe).toEqual(recipe);
    expect(report.counts.placements).toBeGreaterThan(1000);
    expect(report.preview.draw_calls).toBeGreaterThan(0);
    await testInfo.attach("build-report", {
      body: bytes,
      contentType: "application/json",
    });
  });
}

test("narrow viewport keeps controls accessible", async ({
  page,
}, testInfo) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await ready(page);
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(
    390,
  );
  await page.locator("#export").scrollIntoViewIfNeeded();
  await expect(page.locator("#export")).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("mobile.png"),
    fullPage: true,
  });
});
