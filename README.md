# Sylva

Procedural trees and forests for real-time worlds: species described as data,
grown deterministically from a seed, and compiled into conventional raster
assets or detailed instanced geometry. Rendering and storage strategies stay
separate from the generated vegetation.

The design lives in [`docs/design.md`](docs/design.md).

## Crates

- **[sylva_asset](crates/sylva_asset/)**: a mesh-free `GeneratedTree` with
  stable organ identity, plus conventional raster and detailed instanced
  compilers. Detailed tissue preserves leaflet gaps without silhouette masks;
  both outputs retain provenance and separate surface/canopy shading data.
- **[sylva_gltf](crates/sylva_gltf/)**: conventional LOD and detailed instanced
  glTF export, including thin-walled leaf transmission.
- **[sylva_skeleton](crates/sylva_skeleton/)**: the skeleton IR shared by every
  growth backend. Branches, stable path-hashed IDs, keyed randomness, and the
  shared pipe-model radius and rotation-minimizing frame passes.
- **[sylva_grow](crates/sylva_grow/)**: hierarchical rule-based growth. A trunk
  and per-level branching (count, phyllotaxis, angle and length curves, bending,
  tropisms, crown-envelope pruning) grown into the skeleton IR from a seed.
- **[sylva_mesh](crates/sylva_mesh/)**: bark surfaces. One tube per branch
  in an Exedra mesh, with curvature-adaptive rings, bark UVs of uniform texel
  density, embedded junction collars, root flare, authored normals, and a
  per-vertex branch index for provenance.
- **[sylva_foliage](crates/sylva_foliage/)**: leaves. One contour per leaf
  shape yields the full blade mesh, a card and its coverage mask in a shared
  UV frame; compound shapes (needle sprays, fronds) cut their outline into a
  midrib and leaflets through the mask; keyed template variants are placed
  on skeleton sites as identified instances with a canopy normal. Meshes are
  built only by consumers; `tissue_mesh` realizes compound leaflets explicitly.
- **[sylva_texture](crates/sylva_texture/)**: species texture recipes on
  [dapple](https://github.com/forest-rs/dapple): a tileable bark set sized to
  the bark UVs, and a leaf set in a leaf shape's own UV frame, as OpenPBR
  maps ready to pack for lightweald or glTF.
- **[sylva_bake](crates/sylva_bake/)**: deterministic CPU baking of geometry
  into card textures (base colour, coverage, card-frame normals, depth), for
  twig-cluster cards and impostors.
- **[sylva_lod](crates/sylva_lod/)**: level-of-detail chains regenerated from
  the skeleton: coarser bark, pruned twig bark, nested leaf subsets that keep
  canopy leaf area, and leaf cards, with screen-size thresholds.
- **[sylva_measure](crates/sylva_measure/)**: measured realism. Allometry
  (height, crown width, stem diameter and their ratios), stem taper, branch
  angles per order and crown silhouette statistics of a grown skeleton, in
  `dapple_lab`'s report format, bounded by species reference ranges from
  forestry literature, which also serve as `dapple_lab::fit` targets.
- **[sylva_species](crates/sylva_species/)**: species descriptions as data
  (with the `serde` feature), naming a growth backend and its parameters,
  plus optional foliage.

Examples live in `examples/`:

- **[skeleton_dump](examples/skeleton_dump/)**: writes a debug OBJ of a small
  generated skeleton and renders it with Blender for visual review.
- **[species_gallery](examples/species_gallery/)**: grows the species presets in
  `presets/` (open-grown oak, Norway spruce, European beech and silver
  birch, each a species RON file with its bark (a dapple recipe or one of
  dapple's calibrated bark modules), optional leaf colours, an LOD chain
  with a triangle and byte budget per level, and reference ranges beside
  it) for several seeds, dumps each skeleton for the
  same Blender review script, meshes its bark (`bark.obj`, rendered with a UV
  grid by `tools/render_bark.py`), places its leaves (`leaves.obj`,
  `leaf-mask.png`), and generates its bark and leaf texture sets
  (`textures/`) and a twig-cluster card of its leafiest branchlet
  (`twig-card/`), and builds its LOD chain (`lods/`); `tools/render_tree.py`
  renders the textured tree and `tools/render_lods.py` the chain side by
  side.

For direct detailed geometry inspection without an LOD chain or textures:

```sh
cargo run -p species_gallery --bin detailed -- spruce /tmp/sylva-detailed
```

This writes an instanced GLB and reports template reuse and build costs.

For repeatable species and branch review, start with the command's help or a
branch survey. Branch selectors use stable hexadecimal IDs, not buffer indices:

```sh
cargo run --release -p species_gallery --bin review -- --help
cargo run --release -p species_gallery --bin review -- --species beech --list --order 2
cargo run --release -p species_gallery --bin review -- --species birch --out .local/birch-before
cargo run --release -p species_gallery --bin review -- --species birch --out .local/birch-after --frame .local/birch-before/review.ron
```

`review` renders opaque tissue geometry and embedded bark, saves the exact
species source, and reports camera settings, work counts and projected coverage
in `review.ron`. `--branch ID` selects a subtree; `--obj` also exports that
subtree for external inspection. `--frame` reuses a previous capture's cameras.
Coverage is the sampled, visible union of bark and foliage in those views; it
is resolution-dependent and is neither total leaf area nor a realism score.
The images use plain colours and simple lighting to expose geometry.

For isolated branch joins, capture both bark strategies without texture bakes:

```sh
cargo run --release -p species_gallery -- --bark-only --welded --species oak --seeds 1 .local/joins
blender --background --python-exit-code 1 --python examples/species_gallery/tools/render_forks.py -- .local/joins/oak-seed1 --list
blender --background --python-exit-code 1 --python examples/species_gallery/tools/render_forks.py -- .local/joins/oak-seed1
```

`forks.json` distinguishes successful welds, solver refusals and selection
exclusions for every child. The renderer accepts `--branch ID`, `--azimuth`
and a fixed `--width` in metres for matched comparisons. Each image shows
embedded/welded bark in the left/right columns and clay/wireframe in the
top/bottom rows. Its JSON sidecar records the camera, input hashes and Blender
version. Only the chosen child and parent are shown, without camera cutaways;
other branches and foliage are intentionally absent. Crowded forks stay
embedded because the current weld strategy constructs only three-arm junctions.

## Minimum supported Rust version

Sylva's MSRV is 1.92.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
