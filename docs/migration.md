# Migration notes

## Generated vegetation and compiled representations

`place_leaves` now produces shape descriptions and placements without meshing.
`LeafTemplate::mesh` and `LeafTemplate::triangles` have been removed. Call
`leaf_mesh(&template.shape)` when compiling the conventional blade, or
`card_mesh(&template.shape)` for a card. Triangle counts belong to the chosen
realization; `FoliageReport::instanced_triangles` and the gallery's corresponding
foliage statistic have been removed. LOD reports still contain triangle counts.

`LeafInstance` now carries a `LeafId`: branch ID, site kind, site ordinal, and
whorl member. Preserve this value when copying or reducing a leaf. Buffer and
site indices remain local references, not durable identities. IDs are scoped
to one generated tree, and do not imply identity across different trees.
