# Sylva: procedural trees and forests for real-time worlds

Design draft, 2026-09-23.

## Purpose

Generate SpeedTree-class vegetation assets for real-time rendering, and place
them as forests:

- species described as data, grown deterministically from a seed;
- hero-quality LOD0 geometry with a LOD chain down to impostors;
- per-vertex wind data with a documented encoding;
- generated textures: bark, leaves, twig/cluster cards and impostor atlases;
- forest placement with variant pools and ecological competition;
- export to glTF, and a real-time lightweald demo.

Engineering properties are part of the goal. Output must be deterministic
across platforms. Every stage must report stats and timings. Editing one
parameter must regenerate only the stages it affects.

## Non-goals (for now)

- A runtime renderer. Lightweald owns rendering; sylva ships data plus
  reference shader functions for wind.
- An editor UI. Authoring is Rust builders and serde data; an editor comes later.
- Scientific botany. We borrow models from botany where they buy realism or
  control, not for fidelity to plant physiology.

## Pipeline

```
Species (data) + seed
   │
   ├─ growth ─────────► Skeleton IR ◄──── environment (obstacles, light, neighbours)
   │   hierarchical rules │
   │   or simulation      │ shared post-passes: pipe-model radii, tropism,
   │                      │ gravity sag, pruning envelope, smoothing
   ▼                      ▼
   ├─ branch meshing ──► bark surfaces (rings, junction collars, root flare, bark UVs)
   ├─ foliage ─────────► attachment sites → leaves / fronds / cluster cards
   ├─ wind ────────────► hierarchical pivot + weight attributes
   ├─ LOD ─────────────► LOD0…LODn from the skeleton, then impostor
   ├─ textures ────────► bark set, leaf set, card bakes, impostor atlas
   ▼
TreeAsset (render buffers per LOD + materials + textures + metadata)
   │
   ├─ glTF adapter (via exedra_gltf)
   └─ lightweald adapter
Forest = placement over terrain × variant pool of TreeAssets
```

Each arrow is an explicit value with a fingerprint, so every stage can be cached
and inspected on its own.

## Crates

The crate list is a target; crates appear when the milestone that needs them
starts.

| Crate | Owns | `no_std` |
|---|---|---|
| `sylva_skeleton` | Skeleton IR, stable branch IDs, keyed randomness, shared post-passes | yes |
| `sylva_grow` | Hierarchical rule-based generator (art-directable) | yes |
| `sylva_sim` | Environment-aware growth simulation | yes |
| `sylva_mesh` | Branch surface meshing, junctions, bark UVs | yes |
| `sylva_foliage` | Leaf shapes, leaf meshes, phyllotaxis on twigs, cards, fronds | yes |
| `sylva_wind` | Wind attribute encoding, CPU reference evaluator, reference shader | yes |
| `sylva_lod` | LOD policy, skeleton-driven reduction, leaf budget, transitions | yes |
| `sylva_bake` | Deterministic CPU rasterizer / ray caster for card and impostor bakes | yes |
| `sylva_texture` | Species texture recipes (bark, leaf) built on dapple | yes |
| `sylva_asset` | Species → `TreeAsset` orchestration, stage caching, reports | yes |
| `sylva_forest` | Placement, variant pools, ecosystem competition, instance tiles | yes |
| `sylva_gltf` | glTF adapter | std |
| `sylva` | Leaf-only facade (exedra convention) | yes |
| `examples/*`, `apps/*` | Blender review scripts, species gallery, lightweald forest demo | std |

General material synthesis lives in [dapple](../../dapple), a separate
forest-rs procedural material engine that exedra, joiner and lightweald
consumers also use. Sylva keeps only tree-specific recipes and the
geometry-driven bakes.

## Key contracts

### Species

A `Species` value holds the growth description, bark recipe, leaf recipe,
foliage placement, LOD policy and wind stiffness. It is plain data with an
optional `serde` feature. Presets live as data files in an examples crate, not
in core crates. Rust builders come first; the data format follows because
presets are data.

The growth description is either `Hierarchical(levels…)` or
`Simulated(params…)`. Everything below the skeleton ignores which one produced
it.

### Skeleton IR

- **Branches:** a tree of branches. Each branch is a sampled centerline of
  nodes, where a node holds position, radius, a rotation-minimizing frame, age
  and order.
- **Parent attachment:** each branch records its parent ID and the parameter
  `t` where it attaches.
- **Attachment sites** for foliage and fruit: branch, `t`, frame, kind, scale.
- **Stable IDs:** an ID is a hash of the generation path (parent ID, level,
  ordinal), not a storage index. Regeneration can then diff, and caches and
  provenance survive edits that do not change the topology upstream.
- **Keyed randomness.** No sequential RNG stream is shared across the tree. Each
  decision draws from `hash(seed, branch ID, purpose)` (counter-based, e.g.
  PCG/SplitMix over a hash). Changing branch 17's angle then does not reshuffle
  every leaf on branches 18–400. This is what makes incremental regeneration
  and art direction feel stable.
- **Radius** uses the pipe model (da Vinci rule, `r_parent^e = Σ r_child^e`,
  with an exponent of about 2–2.5), computed from the tips down. Both growth
  backends share it.

### Growth backends

**`sylva_grow` (hierarchical)** works per level: trunk, branches, sub-branches,
twigs. Each level has:
- a count and distribution along the parent: spiral / golden angle, opposite,
  whorled, alternate;
- angle, length and radius curves over position on the parent;
- curvature with seeded gnarl noise, gravity and phototropism;
- a crown envelope used for pruning.

It follows Weber & Penn (1995) and SpeedTree's generator-per-level model. It is
fast, predictable and directly art-directable.

**`sylva_sim` (simulation)** grows the tree bud by bud:
- competition for space: space colonization (Runions et al. 2007);
- competition for light: shadow propagation (Palubicki et al. 2009);
- tropisms, apical dominance and yearly iterations;
- environment queries for obstacles, neighbouring crowns and a light direction
  field.

It produces the same Skeleton IR. It is also what lets trees in a forest shape
each other.

We will check both against the tree architecture models of Hallé–Oldeman.
Excurrent conifers, decurrent broadleaves and single-axis palms are the
minimum spread.

### Branch meshing

- **Stations:** rings are placed adaptively. The number of ring segments follows
  radius and screen budget, and station spacing follows curvature. Ring phase
  is aligned along the branch so there is no twist.
- **Bark UVs:** U goes around the branch with an explicit seam. V follows arc
  length, scaled by circumference so the texel density matches the trunk.
  Detail across LODs stays continuous because each LOD re-meshes the same
  skeleton.
- **Junctions**, as a replaceable strategy:
  - `Embedded`: the child base sits inside the parent with a flared collar and
    blended normals. This is the real-time standard; it is cheap and works at
    every LOD.
  - `Welded`: a watertight quad junction for hero trunks and major forks. The
    skeletal quad mesh approach of Bærentzen et al. (2012) fits the half-edge
    kernel. The alternative, a local implicit smooth union through
    `exedra_isosurface` in a small box per fork, loses UV charts; it would be
    fallback evidence only.
- **Root flare / buttresses** use a radial profile at the trunk base, driven by
  the number of major roots.

Output is an `exedra_mesh::Mesh` with attributes. That gives us validation,
deterministic triangulation, and exedra's mesh ops for finishing.

### Foliage

- **Leaf shape:** a closed contour plus a midrib/vein curve and bend/fold
  parameters. It produces both the leaf mesh (LOD0) and the leaf texture mask,
  so shape and texture cannot disagree.
- **Placement:** phyllotaxis on twigs, orientation toward light and away from
  the twig, and size jitter.
- **Card types:**
  - single leaf quads, bent;
  - cluster cards baked from a real twig and leaf cluster;
  - fronds for conifer needles and palms.
- **Canopy normals:** a normal-bending option (per crown, spherical or hull
  based) that gives the soft foliage shading real-time trees rely on.
- **Storage:** leaves stay instances (template × transform) inside the asset
  until LOD packaging merges them into buffers. A leaf never becomes a
  half-edge face set.

### Wind

The wind encoding is a versioned contract. It is inspired by Pivot Painter 2
and SpeedTree wind. Each vertex carries:
- the pivot position and axis for up to three hierarchy levels (trunk, branch,
  twig);
- a bend weight per level (0 at the pivot, rising along the branch);
- per-leaf flutter phase, stiffness, and the leaf's attachment pivot.

`sylva_wind` also ships a CPU reference evaluator, used by tests and non-GPU
consumers, and a reference Slang/WGSL function. The evaluator shows what
correct motion is. Culling needs worst-case displacement per instance to
inflate its bounds, and the asset reports that value too.

### LOD

LODs are regenerated from the skeleton, not decimated from LOD0. This gives
cleaner silhouettes and stable UVs:
- fewer ring segments and stations;
- branches below a projected-size threshold are pruned and their leaves handed
  to the parent's cluster cards;
- leaf reduction preserves area: remaining leaves scale up to keep canopy
  density;
- the final level is an impostor. First a camera-facing billboard set, then
  octahedral impostors (albedo, normal, depth, alpha atlases baked by
  `sylva_bake`).

Each level records a screen-size threshold and a crossfade band.

### Textures

Everything is CPU-generated, deterministic, and fingerprint-cached.

- **Bark:** a tileable height field built from species-specific layers:
  - cellular fissures stretched along V (oak);
  - plates (pine);
  - horizontal lenticels over a pale ground (birch);
  - ring scars (palm).

  Normal, AO, roughness and albedo are derived from that one height field so
  they stay consistent.
- **Leaves:**
  - an alpha mask from the leaf contour;
  - veins, procedural or venation-grown (Runions et al. 2005);
  - albedo gradients and variation;
  - translucency/thickness, and a normal map.
- **Cards and impostors:** baked by rasterizing real geometry with
  `sylva_bake`.
- **Mips:** we generate mip chains ourselves with coverage-preserving alpha
  (Castaño 2010). Without that, alpha-tested foliage thins out into nothing at
  distance. Outputs are PNG for glTF and KTX2 for lightweald.

### TreeAsset

`TreeAsset` is sylva's own render-ready value:
- per-LOD vertex and index buffers with a declared vertex layout (position,
  normal, tangent, UV0, UV1, color, wind);
- material descriptions with texture references;
- bounds, worst-case wind displacement and LOD thresholds;
- a per-vertex provenance map back to skeleton branch IDs;
- a stats report.

Adapters convert it to glTF and to lightweald. For forests, one asset per
variant is placed many times; forest instances are never `exedra_assembly`
instances. The survey showed assembly's per-instance flatten is
O(instances × vertices), and its string paths are the wrong granularity here.

### Forest

- **Placement:** Poisson-disc sampling over a terrain height function, with
  density maps driven by slope, altitude and moisture.
- **Ecosystem competition:** size classes, shade tolerance and self-thinning,
  after Deussen et al. (1998).
- **Variant pool:** each species gets a small pool (e.g. 8 seeds × age
  classes), instanced with rotation and scale jitter. Optionally, `sylva_sim`
  can grow neighbouring trees together for a hero grove.
- **Output:** compact instance arrays, tiled so an edit regenerates only the
  affected tiles.

## Introspection

Every stage returns a report with:
- timings;
- counts: branches, nodes, rings, triangles per LOD, leaves, cards, texels,
  bytes;
- cache hits and misses;
- typed diagnostics, e.g. clamped radius inversions, pruned-branch counts and
  junction fallbacks.

Debug dumps show the skeleton as polylines, attachment frames, wind pivots as
colors, and LOD overlays. The Blender review scripts render these views next to
the shaded asset.

## Upstream work this needs

### exedra

These gaps matter well beyond trees, so each one is fixed in exedra, in its
owning crate. Sylva does not work around any of them.

1. `exedra_mesh` attributes: add a `[f32; 4]` layer type. Extraction then
   carries declared extra layers (UV1, color, vec4 data), splitting render
   vertices where any carried layer differs. `TriMesh` gains the matching
   optional streams.
2. Tangents: MikkTSpace-compatible tangent generation, available at extraction
   for any textured mesh.
3. `exedra_gltf` attributes: `TEXCOORD_1`, `COLOR_0`, `TANGENT` and custom
   `_`-prefixed attributes.
4. `exedra_gltf` materials: `normalTexture`, `occlusionTexture` and
   `metallicRoughnessTexture`, plus `texCoord` selection.
5. `exedra_gltf` extensions: an explicit allowlist of KHR material extensions
   (at least `KHR_materials_diffuse_transmission`, `KHR_materials_transmission`
   and `KHR_texture_transform`), and `EXT_mesh_gpu_instancing` for repeated
   placements.
6. `exedra_constructive` tapering: sweeps with radius/scale/twist laws along
   the path. Lofts get arc-length V charts, so uneven section spacing no longer
   stretches the texture.
7. `exedra_constructive` / `exedra_mesh_ops` branching: a junction or
   blend primitive for tubes meeting tubes, with chart ancestry, as the exedra
   home for the welded-junction strategy.
8. `exedra_mesh_ops`: Booleans preserve corner UVs from the operand that owns
   each face (chart ancestry).
9. `exedra_isosurface`: capsule/cone fields and an N-ary smooth union with
   sound intervals.
10. `exedra_assembly` LOD: LOD sets on parts (levels + screen thresholds),
    carried through compilation, flattening and glTF (e.g. `MSFT_lod` or
    documented extras).
11. `exedra_assembly` scale: flattening and bounds that scale to large
    placement counts. World bounds come from part bounds × placement, not
    per-vertex transforms, and there is a compact bulk-placement path.
12. `exedra_math`: quaternions and vec4 helpers where exedra itself needs them,
    plus keyed deterministic randomness (hash-based). Constructive already has
    Bézier evaluation; it stays there unless others need it.

Once these land, sylva uses exedra for more than the bark mesh container:
tapered sweeps become the reference branch surface, the junction primitive
serves hero forks, and assembly LOD and instancing carry the export path.

### lightweald
1. A vertex format beyond position/normal/uv (the pending `lw-forward.13`
   decision) that carries a tangent, UV1/color and wind data.
2. A vertex deformation hook applied identically in the prepass, shadow and
   forward shaders to keep `@invariant` depth-equal. It also needs:
   - bounds inflation for culling;
   - cached-shadow invalidation for animated instances.
3. LOD selection with dithered crossfade, and impostor rendering.
4. True instanced draws (`instance_count > 1` per visible run). Today every
   instance is one indirect draw.
5. Foliage shading: verify thin-walled OpenPBR back-lighting under direct
   light, or add a cheap two-sided foliage model.
6. A terrain heightfield (a small crate or the demo itself), plus fog or aerial
   perspective. Forests read as flat without depth cues.

We should agree these with the owners of each repo before starting; they are
changes to shared contracts, not sylva internals.

## Milestones

Each milestone is a vertical slice, finished to the quality bar rather than
stubbed.

1. **Foundations.**
   - Repo scaffold, the skeleton IR, stable IDs and keyed randomness.
   - The report/stats spine and a skeleton debug dump with Blender review.
2. **One broadleaf, end to end.**
   - One deciduous species: hierarchical growth, pipe radii, tropism and
     envelope pruning.
   - Branch meshing with embedded collars and bark UVs, leaf shape → mesh +
     texture, bark texture set, coverage-preserving mips.
   - `TreeAsset` → GLB, and Blender renders.
   - Upstream: exedra items 1–6 (attributes, tangents, glTF, tapered sweeps).
3. **Real-time: wind, LOD and impostors in lightweald.**
   - Wind encoding + reference evaluator, the skeleton-driven LOD chain, the
     card and impostor baker.
   - A lightweald single-tree demo with wind and LOD transitions.
   - Upstream: exedra items 10–12 (assembly LOD and scale, math); lightweald
     vertex format, deformation hook, LOD/crossfade.
4. **Range of forms.**
   - A conifer (whorls, needle fronds, plate bark), birch (pale lenticel bark,
     weeping twigs) and a palm.
   - This proves `Species` covers different tree architectures without special
     cases.
5. **Simulation backend.**
   - `sylva_sim` with space colonization and shadow propagation, obstacle
     avoidance, and the same downstream pipeline.
6. **Forest.**
   - Terrain, placement, variant pools, ecosystem competition and instance
     tiles.
   - A lightweald forest demo with thousands of trees, wind, LOD and impostors,
     plus a measured frame budget.
   - Upstream: instanced draws, terrain, fog.
7. **Hero quality.**
   - Welded junctions for major forks, root systems and buttresses, and
     seasons: leaf color, leaf fall and bare-branch LODs.

## Decisions

1. **Math:** glam inside sylva, converting at the exedra boundary.
2. **First species:** an oak-like decurrent broadleaf.
3. **Material synthesis** lives in the dapple repo; sylva depends on it.
4. **Lightweald changes** are made in lightweald by its maintainers, including
   the sylva work, and agreed there first.

## References

- Weber & Penn, *Creation and Rendering of Realistic Trees*, SIGGRAPH 1995.
- Runions, Lane & Prusinkiewicz, *Modeling Trees with a Space Colonization
  Algorithm*, 2007.
- Palubicki et al., *Self-organizing Tree Models for Image Synthesis*,
  SIGGRAPH 2009.
- Bærentzen, Misztal & Wełnicka, *Converting Skeletal Structures to Quad
  Dominant Meshes*, 2012.
- Deussen et al., *Realistic Modeling and Rendering of Plant Ecosystems*,
  SIGGRAPH 1998.
- Runions et al., *Modeling and Visualization of Leaf Venation Patterns*,
  SIGGRAPH 2005.
- Hallé, Oldeman & Tomlinson, *Tropical Trees and Forests: An Architectural
  Analysis*, 1978.
- Castaño, *Computing Alpha Mipmaps*, 2010.
- Epic Games, Pivot Painter 2; Ryan Brucks, octahedral impostors.
