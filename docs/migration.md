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

Construct a `sylva_asset::GeneratedTree` from a completed skeleton, optional
`FoliageParams`, and `TreeMaterials`. It generates placements once and retains
immutable access to the skeleton, shapes, placements, and authored materials.
Growth recipes and seeds remain with the authoring caller. It does not define
a persistence format or an OpenUSD schema.

```rust,ignore
let tree = GeneratedTree::new(grown.skeleton, species.foliage.as_ref(), materials)?;
let chain = build_lods(tree.skeleton(), tree.foliage(), &mesh_params, &policy)?;
let raster = build_asset(&tree, &chain, &RasterOptions::default())?;
let detailed = build_detailed(&tree, &mesh_params)?;
```

`build_asset` now takes the generated tree, its LOD chain, and `RasterOptions`.
The chain must be built from that source snapshot. `TreeMaterials::alpha_cutoff`
has moved to `RasterOptions`; alpha testing is a realization choice. Merged
foliage defaults to the existing canopy-normal treatment; select
`LeafNormals::Surface` to use transformed template normals. Retained instances
always preserve surface normals in their templates and carry canopy normals
separately.

`AssetInstance` adds `leaf: Option<LeafId>` and `canopy_normal`. Cluster cards
have no individual leaf identity. Both `TreeAsset` and `DetailedAsset` contain
a `branches` table mapping compiled branch indices back to stable `BranchId`s.
Pruned LOD bark, welds, and weld refusals now use the original skeleton's index
space, correcting the previous compacted-index provenance.

`build_detailed` needs no `LodChain` and never constructs merged foliage. It
meshes each reusable shape with `tissue_mesh`, preserving geometric leaflet
gaps, shape UVs, source identity, and placement. The first tissue realization
uses thin surfaces with overlapping attachment patches; it is not a watertight
union, volumetric needle model, or area-measurement reference. Cards and the
existing envelope blade remain available from the same shape description.

`export_detailed_glb` writes shared tissue geometry through
`EXT_mesh_gpu_instancing`. Leaf coverage is geometric (`OPAQUE` alpha mode),
while thin-walled transmission remains enabled. As in the existing instanced
export, branch indices use `_SEED`; stable organ IDs and canopy normals remain
in the Sylva asset and are not persisted in glTF.

For a texture-free geometry inspection asset:

```sh
cargo run -p species_gallery --bin detailed -- spruce /tmp/sylva-detailed
```

The example supports `oak`, `spruce`, `beech`, and `birch`, using seed 1. It
prints source/build/export times, stored template triangles, placement counts,
and the equivalent expanded triangle count. The ordinary species gallery
continues to use the conventional raster path.
