// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import init, { Generator } from "../pkg/sylva_workbench.js";
const ready = init();
let generator = null;
self.onmessage = async ({ data }) => {
  try {
    const wasm = await ready;
    generator ??= new Generator();
    self.postMessage({
      type: "progress",
      text: "Growing branches, realizing tissue and baking materials…",
    });
    const started = performance.now();
    const specimen = generator.generate(JSON.stringify(data.recipe));
    try {
      const report = JSON.parse(specimen.report());
      const glb = specimen.take_glb();
      report.worker_memory_bytes = wasm.memory.buffer.byteLength;
      report.timings.generation_export_ms = performance.now() - started;
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
