// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { defineConfig } from "@playwright/test";
const production = process.env.WORKBENCH_PREVIEW === "1";
const port = production ? 4173 : 5173;
export default defineConfig({
  testDir: "./tests",
  timeout: 120_000,
  workers: 1,
  use: {
    channel: process.env.PLAYWRIGHT_CHANNEL || "chrome",
    baseURL: `http://127.0.0.1:${port}`,
    viewport: { width: 1440, height: 1000 },
  },
  webServer: {
    command: `npm run ${production ? "preview" : "dev"} -- --port ${port} --strictPort`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: !process.env.CI,
  },
});
