# Sylva workbench

Generate, inspect, save and export the actual Sylva species in a browser.
Rust runs in a dedicated Web Worker through WebAssembly; Three.js displays the
normal detailed GLB realization. There is no server-side generation or separate
JavaScript growth model.

## Run

Use Node 22.12+ (or 24+), the workspace Rust toolchain, and the matching
`wasm-bindgen` CLI:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.100 --locked
cd examples/workbench
npm ci
npm run wasm
npm run dev
```

Open the local address printed by Vite (normally <http://127.0.0.1:5173>).
Rust changes require `npm run wasm`; browser source changes reload automatically.
The first Rust build compiles the shared geometry and material crates.

```sh
npm run build
npm run preview
```

`dist/` is a static deployable site, including the WASM module. Serve it over
HTTP(S), with `.wasm` as `application/wasm`; it does not run from a `file:` URL.
Relative asset paths support hosting under a subdirectory. No deployment or
public hosting is configured here. Fonts optionally load from Google Fonts;
system fonts work when that service is unavailable. Tree data stays on-device.

## Use

- Choose birch, oak, beech or spruce. Change the seed, foliage site density or
  blade size, then **Generate tree**. Exports always use the displayed specimen,
  even when there are pending edits. Camera and sun changes do not regenerate.
- Drag to orbit, right-drag to pan, and scroll to approach. **Fit tree** restores
  the whole-tree camera. **Turntable** rotates the camera, not the tree.
- **Structure** hides foliage. Click bark to highlight one branch, read its
  stable source ID and parent, then **Frame branch** to inspect its geometry.
- **Save recipe** downloads the displayed tree's versioned inputs. **Open recipe**
  validates before starting generation. **Copy link** encodes those same inputs
  in the URL fragment; it does not upload an asset.
- **Export GLB** downloads the bytes loaded by the preview: geometric tissue,
  `EXT_mesh_gpu_instancing`, Dapple textures, metres, Y up. **Capture PNG**
  captures the canvas at its current camera and preview mode.
- **Build report** separates stored geometry from expanded instance geometry,
  growth, meshing, material baking and export from preview loading, and reports the most recent
  frame's renderer counters. Download its JSON for recipe, counts, bounds,
  branch-index-to-ID mapping, camera and rendering context.

Cancellation terminates the worker, keeping the last successful specimen.
Failed generation or invalid imports also retain the displayed tree. Within a
species, the worker retains one encoded bark bake and one leaf bake. Seed changes
reuse bark; density changes reuse both; leaf size changes rebuild the leaf bake.
The report exposes cache hits, encoded cache bytes and WASM memory capacity.
Regeneration still frees source/mesh data and disposes the previous preview's
geometry, materials, textures and image bitmaps. WASM capacity stays at its high
water mark until cancellation, a species change or page reload replaces the
worker. This is reserved capacity, not a measurement of live allocations.

## What this demonstration proves

The existing `GeneratedTree → build_detailed → export_detailed_glb` path works
in a browser, with real branch provenance and deterministic recipe round trips.
The four bundled presets are shared with `species_gallery`; no species fork is
maintained here. The Rust adapter is an example crate, leaving the core APIs and
dependencies unchanged.

Dapple bakes 256² representative bark tiles and 128² leaf maps. Leaf silhouette
comes from tissue geometry; its material has no alpha test. The GLB retains
`KHR_materials_diffuse_transmission`; the pinned Three.js loader does not
implement it, so the preview currently uses metallic-roughness shading without
that response. Bark uses one representative tile across trunk and branches;
young branches do not yet have a distinct bark appearance.

This is a static detailed-geometry viewer. It does not implement wind, automatic
LOD, motion bounds, layerstack integration, or hierarchical visibility. A draw
batch per template reduces draw calls but still processes every submitted
placement. Beech and spruce are intentionally demanding inputs, especially on
phones. Lower foliage density changes the authored specimen; it is not LOD.

The first bridge uses GLB for preview as well as export. Every growth edit fully
regenerates and serializes the tree. Timings expose that baseline rather than
claiming incremental generation or uploads. Generate-to-first-frame measures wall
time through GPU completion, checked with a fence after the visible and shadow
passes. It does not measure display scanout. The GPU completion wait includes
queueing and polling latency; it is not an isolated GPU execution time. Render
CPU submission time is reported separately. Static views stop submitting frames; camera motion, lighting, selection,
mode, resize and specimen changes redraw. Shadows are cached until the specimen,
mode or sun changes.

Recipes are interpreted against this workbench's bundled presets; version 1
must change if their interpretation changes. A share link needs the same build
to reproduce identical geometry. The exported report maps GLB branch indices
back to source IDs; GLB alone does not preserve stable leaf identities.

## Verify

```sh
cargo test -p sylva_workbench
npm test
npm run format:check
npm run build
npm run test:browser
```

Browser tests use installed Google Chrome by default. Alternatively:

```sh
npx playwright install chromium
PLAYWRIGHT_CHANNEL=chromium npm run test:browser
```

They exercise all four species, branch picking, bounded inputs, cancellation,
invalid-import recovery, a byte-identical save/reopen/export round trip, reports,
and the narrow-screen layout. Screenshots are saved under ignored `test-results/`.
Use `WORKBENCH_PREVIEW=1 npm run test:browser` to test the production bundle.

After `npm run build`, repeat the generation workload and capture review images with:

```sh
node tools/render.mjs ../../.local/workbench/latency.json
node tools/render.mjs ../../.local/workbench/oak-review.json oak
node tools/render.mjs ../../.local/workbench/beech-seed3.json beech --seed=3
```

This records three runs each of birch, oak and spruce by default. Supply species
names after the output path for targeted reviews, and `--seed=N` for another seed.
The default density and blade size are used. Reports retain the recipe,
source revision, browser version, camera, counts and phase timings alongside whole
crown, whole structure and interior captures. Omitting the output path creates a
timestamped review directory. The script starts and closes its own production
preview server; set `WORKBENCH_URL` to review an already running server. Compare
on the same machine, browser and server build;
the first run per species includes a fresh page. These are generation and first
frame measurements, not steady-state rendering benchmarks.

For native CPU profiling, build the same workload with release optimizations
and debug symbols:

```sh
CARGO_PROFILE_RELEASE_DEBUG=1 cargo build --release -p sylva_workbench --bin profile
../../target/release/profile oak generate 30 > ../../.local/workbench/oak-native.jsonl
```

The executable prints its PID to stderr. While it runs, capture a macOS stack
sample in another terminal with `sample <pid> 5 1 -file /tmp/oak.sample.txt`.
Use `oak export 30` to isolate repeated export of one retained detailed asset;
that mode omits material images, while `generate` runs the complete workbench
path. Both use seed 1 and default parameters. An optional fourth argument saves
the final GLB after timing, for byte comparisons. Compare timings without a
sampler or competing builds running; native profiles locate CPU work but do
not substitute for browser latency measurements.
