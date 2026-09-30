// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
const here = fileURLToPath(new URL("..", import.meta.url));
const root = path.resolve(here, "../..");
const revision = spawnSync("git", ["describe", "--always", "--dirty"], {
  cwd: root,
  encoding: "utf8",
});
const env = {
  ...process.env,
  SYLVA_WORKBENCH_REVISION: revision.stdout?.trim() || "unknown",
};
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit", env });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
run("cargo", [
  "build",
  "-p",
  "sylva_workbench",
  "--lib",
  "--target",
  "wasm32-unknown-unknown",
  "--release",
]);
run("wasm-bindgen", [
  path.join(root, "target/wasm32-unknown-unknown/release/sylva_workbench.wasm"),
  "--target",
  "web",
  "--out-dir",
  path.join(here, "pkg"),
]);
