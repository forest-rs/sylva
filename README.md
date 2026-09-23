# Sylva

Procedural trees and forests for real-time worlds: species described as data,
grown deterministically from a seed, meshed with a LOD chain down to impostors,
with wind data and generated textures.

The design lives in [`docs/design.md`](docs/design.md).

## Crates

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
  UV frame; keyed template variants are placed on skeleton sites as
  instances with a canopy normal.
- **[sylva_species](crates/sylva_species/)**: species descriptions as data
  (with the `serde` feature), naming a growth backend and its parameters,
  plus optional foliage.

Examples live in `examples/`:

- **[skeleton_dump](examples/skeleton_dump/)**: writes a debug OBJ of a small
  generated skeleton and renders it with Blender for visual review.
- **[species_gallery](examples/species_gallery/)**: grows the species presets in
  `presets/` (an oak so far) for several seeds, dumps each skeleton for the
  same Blender review script, meshes its bark (`bark.obj`, rendered with a UV
  grid by `tools/render_bark.py`), and places its leaves (`leaves.obj`,
  `leaf-mask.png`; `tools/render_tree.py` renders bark and leaves).

## Minimum supported Rust version

Sylva's MSRV is 1.92.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
