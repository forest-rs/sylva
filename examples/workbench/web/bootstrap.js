// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import("./main.js").catch((error) => {
  document.getElementById("loading").hidden = true;
  const status = document.getElementById("status");
  status.classList.add("error");
  status.textContent = `The preview could not start: ${error.message}. Check that WebGL 2 and hardware acceleration are enabled, then reload.`;
  document.getElementById("grow").disabled = true;
});
