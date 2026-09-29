// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

export const SPECIES = {
  birch: ["Silver birch", "Betula pendula"],
  oak: ["Pedunculate oak", "Quercus robur"],
  beech: ["European beech", "Fagus sylvatica"],
  spruce: ["Norway spruce", "Picea abies"],
};
export const DEFAULT_RECIPE = Object.freeze({
  version: 1,
  species: "birch",
  seed: 1,
  density: 1,
  leaf_size: 1,
});
export function validateRecipe(value) {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("Open a Sylva recipe JSON object.");
  const keys = Object.keys(DEFAULT_RECIPE);
  if (Object.keys(value).some((key) => !keys.includes(key)))
    throw new Error("This recipe has unsupported fields.");
  if (value.version !== 1)
    throw new Error("This workbench reads recipe version 1.");
  if (!Object.hasOwn(SPECIES, value.species))
    throw new Error("Choose birch, oak, beech or spruce.");
  if (
    !Number.isInteger(value.seed) ||
    value.seed < 0 ||
    value.seed > 4294967295
  )
    throw new Error("Seed must be a whole number from 0 to 4294967295.");
  if (
    !Number.isFinite(value.density) ||
    value.density < 0.35 ||
    value.density > 1.2
  )
    throw new Error("Foliage density must be between 35% and 120%.");
  if (
    !Number.isFinite(value.leaf_size) ||
    value.leaf_size < 0.7 ||
    value.leaf_size > 1.4
  )
    throw new Error("Leaf size must be between 70% and 140%.");
  return Object.fromEntries(keys.map((key) => [key, value[key]]));
}
export function recipeHash(recipe) {
  return (
    "#recipe=" + encodeURIComponent(JSON.stringify(validateRecipe(recipe)))
  );
}
export function parseHash(hash) {
  if (!hash.startsWith("#recipe=")) return { ...DEFAULT_RECIPE };
  return validateRecipe(JSON.parse(decodeURIComponent(hash.slice(8))));
}
