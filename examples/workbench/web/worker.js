// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import init, { generate } from "../pkg/sylva_workbench.js";
const ready = init();
self.onmessage = async ({ data }) => {
  try {
    await ready;
    self.postMessage({
      type: "progress",
      text: "Growing branches, realizing tissue and baking materials…",
    });
    const started = performance.now();
    const specimen = generate(JSON.stringify(data.recipe));
    try {
      const report = JSON.parse(specimen.report());
      const glb = specimen.take_glb();
      report.timings = { generation_export_ms: performance.now() - started };
      self.postMessage({ type: "result", report, glb: glb.buffer }, [
        glb.buffer,
      ]);
    } finally {
      specimen.free();
    }
  } catch (error) {
    self.postMessage({
      type: "error",
      message: String(error?.message ?? error),
    });
  }
};
