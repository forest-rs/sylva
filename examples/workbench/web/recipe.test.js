// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  DEFAULT_RECIPE,
  validateRecipe,
  recipeHash,
  parseHash,
} from "./recipe.js";
test("share links preserve exact inputs and maximum seed", () => {
  const recipe = {
    ...DEFAULT_RECIPE,
    seed: 4294967295,
    density: 0.35,
    leaf_size: 1.4,
  };
  assert.deepEqual(parseHash(recipeHash(recipe)), recipe);
});
test("invalid imports fail before replacing the specimen", () => {
  for (const patch of [
    { species: "toString" },
    { seed: -1 },
    { seed: 1.2 },
    { density: NaN },
    { leaf_size: 4 },
    { version: 2 },
    { wind: true },
  ]) {
    assert.throws(() => validateRecipe({ ...DEFAULT_RECIPE, ...patch }));
  }
  assert.throws(() => validateRecipe(null));
  assert.throws(() => parseHash("#recipe=%ZZ"));
});
