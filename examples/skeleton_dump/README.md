# Skeleton dump

Grows a small keyed test tree, runs the shared `sylva_skeleton` passes
(pipe-model radii, rotation-minimizing frames), adds a leaf site at every tip,
validates the result, and writes debug dumps for visual review.

```sh
cargo run -p skeleton_dump -- .local/gallery/skeleton-dump
blender --background --python examples/skeleton_dump/tools/render.py -- .local/gallery/skeleton-dump
```

- `skeleton.json`: branches (ID, order, parent attachment, nodes with position,
  radius, tangent and normal) and sites.
- `skeleton.obj`: centerlines as OBJ polylines, one object per branch order.
- `stats.json`: skeleton stats, pass reports, and timings (single
  observations, not benchmarks).
- `skeleton.png` and `skeleton-side.png`: Workbench renders from the Blender
  script. Each branch is a curve beveled by the pipe-model radii and colored by
  order; tip sites are green spheres.

The generator is deliberately minimal: three levels, golden-angle phyllotaxis,
keyed jitter and bending. It exercises the IR; it is not a species. The tip
radius is 10 mm so that the 216-tip crown reads at render size.
On macOS the Blender executable can be
`/Applications/Blender.app/Contents/MacOS/Blender`.
