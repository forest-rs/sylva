// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Grows every species preset (an oak and a spruce) for several seeds and
//! writes, per species and seed, a skeleton dump and the bark mesh for
//! Blender review.
//!
//! A preset is data: `presets/<name>.ron` (the [`Species`]),
//! `presets/<name>_bark.toml` (its dapple bark recipe) and, optionally,
//! `presets/<name>_leaf.ron` (its leaf colours, the fields of
//! [`LeafRecipe`]).
//!
//! ```sh
//! cargo run --release -p species_gallery -- .local/gallery/species-gallery
//! for d in .local/gallery/species-gallery/oak-*; do
//!   blender --background --python examples/skeleton_dump/tools/render.py -- "$d"
//!   blender --background --python examples/species_gallery/tools/render_bark.py -- "$d"
//!   blender --background --python examples/species_gallery/tools/render_tree.py -- "$d"
//! done
//! d=.local/gallery/species-gallery/oak-seed1
//! blender --background --python examples/species_gallery/tools/render_lods.py -- "$d"
//! blender --background --python examples/species_gallery/tools/measure_lods.py -- "$d"
//! blender --background --python examples/species_gallery/tools/render_glb.py -- "$d"
//! blender --background --python examples/species_gallery/tools/render_glb.py -- "$d" --instanced
//! ```
//!
//! The standard review render is the Cycles beauty scene, a tree in a
//! meadow under a low sun:
//!
//! ```sh
//! cargo run --release -p species_gallery -- --glb-only
//! for shot in hero backlit grove bark; do
//!   blender --background --python examples/species_gallery/tools/render_beauty.py -- \
//!     "$shot" ".local/gallery/beauty/oak-$shot.png"
//! done
//! blender --background --python examples/species_gallery/tools/render_beauty.py -- \
//!   hero .local/gallery/beauty/spruce-hero.png --species spruce --seed 1
//! blender --background --python examples/species_gallery/tools/render_beauty.py -- \
//!   mixed .local/gallery/beauty/mixed-grove.png
//! ```
//!
//! Outputs default to `.local/gallery/`, which is git-ignored and survives
//! `cargo clean`; `target/` holds build artifacts only.
//!
//! `--species oak,spruce` grows only those presets, `--preset FILE` grows
//! a preset read from disk (with its bark and leaf files beside it) instead
//! of the built-in ones, and `--skeleton-only` writes just each seed's
//! `skeleton.json`, the fastest loop for crown shape. `--seeds 1,3` grows
//! only those seeds, and `--tree-only` skips the LOD
//! chain, card bakes and glTF export, for quick crown iteration. `--welded`
//! also meshes the bark with major forks welded, as `bark-welded.obj` with
//! the forks in `forks.json`; `render_forks.py` compares them close up
//! against the embedded bark. Seed 1 writes its LOD meshes, baked atlases and
//! GLBs. `--glb-only` writes, for every seed, just the GLBs with their baked
//! atlases (and a short `stats.json`), which is what `render_beauty.py`
//! reads: the fast loop for beauty review.
//!
//! `bark.obj` carries positions, bark UVs and the authored normals;
//! `render_bark.py` shows it with a UV grid so seams and texel density are
//! visible. `leaves.obj` expands every leaf instance at full detail,
//! `leaf-mask.png` is the first template's coverage mask, and
//! `render_tree.py` renders bark and leaves together, textured.
//!
//! Per species, `textures/<species>-bark` and `textures/<species>-leaf` hold
//! the generated material sets, packed for `lightweald` and `gltf`: KTX2
//! files with full mip chains and PNG images of level 0. The glTF leaf set
//! includes `diffuse_transmission` (tint RGB, weight A), which the GLBs bind
//! through `KHR_materials_diffuse_transmission`; Lightweald has no subsurface
//! slot yet.

mod card;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use dapple_encode::{Filter, PackSettings, Profile, ktx2, pack};
use exedra_mesh::{ExtractAttribute, ExtractParams, NormalsSource, TriMesh};
use sylva_asset::{MaterialRole, TreeMaterials, build_asset};
use sylva_bake::BakeMaterial;
use sylva_foliage::{Foliage, place_leaves};
use sylva_gltf::{ExportOptions, LeafExport, MaterialTextures, export_lod_glb_with};
use sylva_lod::{
    Atlas, AtlasSettings, CardMaterials, Impostor, ImpostorLayout, ImpostorPolicy, LodPolicy,
    bake_clusters, bake_impostor, build_lods,
};
use sylva_mesh::{BRANCH_LAYER, Junction, MeshParams, Weld, mesh_skeleton};
use sylva_texture::{LeafRecipe, bark, leaf};

use skeleton_dump::{skeleton_json, skeleton_obj};
use sylva_species::Species;

/// One species preset: its growth and foliage, its bark recipe (dapple's
/// format, scaled to a 1 m tile), and optionally its leaf colours.
struct Preset {
    species: String,
    bark: String,
    leaf: Option<String>,
}

/// The built-in presets, as `(species, bark, leaf colours)` sources.
const PRESETS: [(&str, &str, Option<&str>); 2] = [
    (
        include_str!("../presets/oak.ron"),
        include_str!("../presets/oak_bark.toml"),
        None,
    ),
    (
        include_str!("../presets/spruce.ron"),
        include_str!("../presets/spruce_bark.toml"),
        Some(include_str!("../presets/spruce_leaf.ron")),
    ),
];
#[cfg(test)]
const OAK: &str = PRESETS[0].0;
const SEEDS: [u64; 3] = [1, 2, 3];

/// A species' leaf colours: the fields of [`LeafRecipe`] a preset may set,
/// read from `<species>_leaf.ron`.
#[derive(serde::Deserialize)]
#[serde(default)]
struct LeafLook {
    green: [f32; 3],
    vein: [f32; 3],
    translucent: [f32; 3],
    translucency: f32,
    mottle: f32,
    roughness: f32,
}

impl Default for LeafLook {
    fn default() -> Self {
        let r = LeafRecipe::default();
        Self {
            green: r.green,
            vein: r.vein,
            translucent: r.translucent,
            translucency: r.translucency,
            mottle: r.mottle,
            roughness: r.roughness,
        }
    }
}

/// Reads the preset at `path` (`<name>.ron`), with `<name>_bark.toml` and,
/// if present, `<name>_leaf.ron` beside it.
fn read_preset(path: &std::path::Path) -> Result<Preset, Box<dyn std::error::Error>> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("preset file name")?;
    let sibling = |suffix: &str| path.with_file_name(format!("{stem}{suffix}"));
    Ok(Preset {
        species: std::fs::read_to_string(path)?,
        bark: std::fs::read_to_string(sibling("_bark.toml"))?,
        leaf: std::fs::read_to_string(sibling("_leaf.ron")).ok(),
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out_dir = PathBuf::from(".local/gallery/species-gallery");
    let mut seeds = SEEDS.to_vec();
    let mut tree_only = false;
    let mut welded = false;
    let mut glb_only = false;
    let mut skeleton_only = false;
    let mut preset = None;
    let mut only: Option<Vec<String>> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seeds" => {
                let list = args.next().ok_or("--seeds needs a value, e.g. 1,3")?;
                seeds = list.split(',').map(str::parse).collect::<Result<_, _>>()?;
            }
            "--tree-only" => tree_only = true,
            "--welded" => welded = true,
            "--glb-only" => glb_only = true,
            "--skeleton-only" => skeleton_only = true,
            "--species" => {
                let list = args
                    .next()
                    .ok_or("--species needs a value, e.g. oak,spruce")?;
                only = Some(list.split(',').map(str::to_owned).collect());
            }
            "--preset" => preset = Some(args.next().ok_or("--preset needs a RON file")?),
            _ => out_dir = PathBuf::from(arg),
        }
    }
    let presets: Vec<Preset> = match &preset {
        Some(path) => vec![read_preset(std::path::Path::new(path))?],
        None => PRESETS
            .iter()
            .map(|&(species, bark, leaf)| Preset {
                species: species.into(),
                bark: bark.into(),
                leaf: leaf.map(Into::into),
            })
            .collect(),
    };
    for preset in &presets {
        let species: Species = ron::from_str(&preset.species)?;
        if only
            .as_ref()
            .is_some_and(|names| !names.contains(&species.name))
        {
            continue;
        }
        grow_species(
            &out_dir,
            &species,
            preset,
            &seeds,
            Flags {
                tree_only,
                welded,
                glb_only,
                skeleton_only,
            },
        )?;
    }
    println!("wrote {}", out_dir.display());
    Ok(())
}

/// Command-line switches that choose what [`grow_species`] writes.
#[derive(Copy, Clone, Debug)]
struct Flags {
    tree_only: bool,
    welded: bool,
    glb_only: bool,
    skeleton_only: bool,
}

/// Grows, meshes and exports one species for every seed.
fn grow_species(
    out_dir: &std::path::Path,
    species: &Species,
    preset: &Preset,
    seeds: &[u64],
    flags: Flags,
) -> Result<(), Box<dyn std::error::Error>> {
    let Flags {
        tree_only,
        welded,
        glb_only,
        skeleton_only,
    } = flags;
    if skeleton_only {
        for &seed in seeds {
            let grown = species.grow(seed)?;
            let dir = out_dir.join(format!("{}-seed{seed}", species.name));
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("skeleton.json"), skeleton_json(&grown.skeleton)?)?;
            println!(
                "{}-seed{seed}: {:?} branches by level, {} sites",
                species.name, grown.report.branches_by_level, grown.report.sites
            );
        }
        return Ok(());
    }
    let textures = write_textures(&out_dir.join("textures"), species, preset)?;
    for &seed in seeds {
        let started = Instant::now();
        let grown = species.grow(seed)?;
        let elapsed = started.elapsed();
        let dir = out_dir.join(format!("{}-seed{seed}", species.name));
        std::fs::create_dir_all(&dir)?;
        if !glb_only {
            std::fs::write(dir.join("skeleton.json"), skeleton_json(&grown.skeleton)?)?;
            std::fs::write(dir.join("skeleton.obj"), skeleton_obj(&grown.skeleton)?)?;
        }
        let meshed = Instant::now();
        let bark = mesh_skeleton(&grown.skeleton, &MeshParams::default())?;
        let (tri, _) = bark.mesh.to_trimesh(&ExtractParams {
            normals: NormalsSource::CustomOnly,
            attributes: vec![ExtractAttribute::new(BRANCH_LAYER, u32::MAX)],
            ..ExtractParams::default()
        });
        let mesh_elapsed = meshed.elapsed();
        if !glb_only {
            std::fs::write(dir.join("bark.obj"), bark_obj(&tri)?)?;
        }
        if welded {
            write_welded(&dir, &grown.skeleton)?;
        }
        let mesh = &bark.report;
        let leaves = match &species.foliage {
            Some(params) => {
                let placed = Instant::now();
                let foliage = place_leaves(&grown.skeleton, params)?;
                let place_us = placed.elapsed().as_micros();
                if !glb_only {
                    std::fs::write(dir.join("leaves.obj"), leaves_obj(&foliage)?)?;
                    write_mask(&dir.join("leaf-mask.png"), &foliage)?;
                }
                if tree_only {
                    println!(
                        "{}-seed{seed}: {:?} branches by level, {} leaves",
                        species.name, grown.report.branches_by_level, foliage.report.leaves
                    );
                    continue;
                }
                let export = if glb_only {
                    Export::Glb
                } else if seed == 1 {
                    Export::All
                } else {
                    Export::None
                };
                let lods = write_lods(&dir, export, &grown.skeleton, &foliage, &tri, &textures)?;
                if glb_only {
                    let r = &grown.report;
                    std::fs::write(
                        dir.join("stats.json"),
                        format!(
                            "{{\"seed\":{seed},\"branches_by_level\":{:?},\"max_radius_m\":{}{lods}}}\n",
                            r.branches_by_level, r.pipe.max_radius
                        ),
                    )?;
                    continue;
                }
                let card = card::bake_twig_card(
                    &dir.join("twig-card"),
                    &grown.skeleton,
                    &tri,
                    &foliage,
                    &textures,
                )?;
                let r = &foliage.report;
                format!(
                    ",\"foliage\":{{\"leaves\":{},\"templates\":{},\"triangles\":{},\"place_us\":{place_us}}}{card}{lods}",
                    r.leaves, r.templates, r.instanced_triangles
                )
            }
            None => String::new(),
        };
        let report = &grown.report;
        let by_level: Vec<String> = report
            .branches_by_level
            .iter()
            .map(ToString::to_string)
            .collect();
        let stats = format!(
            "{{\"species\":\"{}\",\"seed\":{seed},\"branches_by_level\":[{}],\"nodes\":{},\
             \"truncated\":{},\"removed\":{},\"skipped\":{},\"sites\":{},\
             \"max_radius_m\":{},\"grow_us\":{},\"bark\":{{\"branches\":{},\"rings\":{},\
             \"vertices\":{},\"triangles\":{},\"segments\":[{},{}],\"render_vertices\":{},\
             \"mesh_us\":{}}}{leaves}}}\n",
            species.name,
            by_level.join(","),
            report.nodes,
            report.truncated,
            report.removed,
            report.skipped,
            report.sites,
            report.pipe.max_radius,
            elapsed.as_micros(),
            mesh.branches,
            mesh.rings,
            mesh.vertices,
            mesh.triangles(),
            mesh.min_segments,
            mesh.max_segments,
            tri.positions.len(),
            mesh_elapsed.as_micros(),
        );
        std::fs::write(dir.join("stats.json"), &stats)?;
        print!("{stats}");
    }
    Ok(())
}

/// Meshes the bark again with major forks welded, writing it as
/// `bark-welded.obj` and the forks as `forks.json`: one entry per major fork
/// (welded or refused) with its center, parent radius and child branch.
fn write_welded(
    dir: &std::path::Path,
    skeleton: &sylva_skeleton::Skeleton,
) -> Result<(), Box<dyn std::error::Error>> {
    let params = MeshParams {
        junction: Junction::Welded(Weld::default()),
        ..MeshParams::default()
    };
    let started = Instant::now();
    let bark = mesh_skeleton(skeleton, &params)?;
    let (tri, _) = bark.mesh.to_trimesh(&ExtractParams {
        normals: NormalsSource::CustomOnly,
        attributes: vec![ExtractAttribute::new(BRANCH_LAYER, u32::MAX)],
        ..ExtractParams::default()
    });
    let elapsed = started.elapsed();
    std::fs::write(dir.join("bark-welded.obj"), bark_obj(&tri)?)?;
    let branches = skeleton.branches();
    let fork = |branch: u32, welded: bool| -> Option<String> {
        let child = branches.get(branch as usize)?;
        let attachment = child.parent?;
        let sample = skeleton.branch(attachment.parent)?.sample(attachment.t);
        let (p, t, d) = (
            sample.position,
            sample.frame.tangent,
            child.nodes[0].frame.tangent,
        );
        Some(format!(
            "{{\"branch\":{branch},\"welded\":{welded},\"center\":[{},{},{}],\
             \"parent_tangent\":[{},{},{}],\"child_tangent\":[{},{},{}],\
             \"parent_radius\":{},\"child_radius\":{}}}",
            p.x, p.y, p.z, t.x, t.y, t.z, d.x, d.y, d.z, sample.radius, child.nodes[0].radius
        ))
    };
    let forks: Vec<String> = bark
        .welds
        .iter()
        .filter_map(|&b| fork(b, true))
        .chain(
            bark.weld_refusals
                .iter()
                .filter_map(|r| fork(r.branch, false)),
        )
        .collect();
    let r = &bark.report;
    let json = format!(
        "{{\"welded\":{},\"fallbacks\":{},\"builds\":{},\"triangles\":{},\"skin_quads\":{},\"skin_triangles\":{},\"mesh_us\":{},\"forks\":[{}]}}\n",
        r.welded_junctions,
        r.weld_fallbacks,
        r.builds,
        r.triangles(),
        r.skin_quads,
        r.skin_triangles,
        elapsed.as_micros(),
        forks.join(",")
    );
    std::fs::write(dir.join("forks.json"), &json)?;
    let mut reasons: Vec<String> = bark
        .weld_refusals
        .iter()
        .map(|r| format!("{:?}", r.error))
        .map(|e| {
            e.split([' ', '{', '('])
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    reasons.sort();
    reasons.dedup();
    println!(
        "welded {} forks, {} refused ({})",
        r.welded_junctions,
        r.weld_fallbacks,
        reasons.join(", ")
    );
    Ok(())
}

/// Writes extracted bark as OBJ with UVs and normals, one group per branch.
fn bark_obj(tri: &TriMesh) -> Result<String, std::fmt::Error> {
    let mut out = String::from("# sylva bark\n");
    for p in &tri.positions {
        writeln!(out, "v {} {} {}", p[0], p[1], p[2])?;
    }
    // Sylva and glTF put `v = 0` at the image top; OBJ puts it at the bottom.
    for uv in &tri.uvs {
        writeln!(out, "vt {} {}", uv[0], 1.0 - uv[1])?;
    }
    for n in &tri.normals {
        writeln!(out, "vn {} {} {}", n[0], n[1], n[2])?;
    }
    for face in tri.indices.as_chunks::<3>().0 {
        let [a, b, c] = face.map(|i| i + 1);
        writeln!(out, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}")?;
    }
    Ok(out)
}

/// Writes every leaf instance at full detail as one OBJ with UVs.
fn leaves_obj(foliage: &Foliage) -> Result<String, std::fmt::Error> {
    let templates: Vec<&exedra_mesh::Mesh> = foliage.templates.iter().map(|t| &t.mesh).collect();
    instances_obj(&templates, &foliage.instances)
}

/// Writes leaf instances of `templates` as one OBJ with UVs and, as vertex
/// normals, each leaf's canopy normal, so the crown shades as one soft volume.
fn instances_obj(
    templates: &[&exedra_mesh::Mesh],
    instances: &[sylva_foliage::LeafInstance],
) -> Result<String, std::fmt::Error> {
    let templates: Vec<TriMesh> = templates
        .iter()
        .map(|t| t.to_trimesh(&ExtractParams::default()).0)
        .collect();
    let mut out = String::from("# sylva leaves\n");
    let mut faces = String::new();
    let mut base = 1_u32;
    for leaf in instances {
        let tri = &templates[leaf.template as usize];
        for (p, uv) in tri.positions.iter().zip(&tri.uvs) {
            let world = leaf.position
                + leaf.rotation * (sylva_skeleton::glam::Vec3::from_array(*p) * leaf.scale);
            let n = leaf.canopy_normal;
            writeln!(out, "v {} {} {}", world.x, world.y, world.z)?;
            writeln!(out, "vt {} {}", uv[0], 1.0 - uv[1])?;
            writeln!(out, "vn {} {} {}", n.x, n.y, n.z)?;
        }
        for face in tri.indices.as_chunks::<3>().0 {
            let [a, b, c] = face.map(|i| i + base);
            writeln!(faces, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}")?;
        }
        base += u32::try_from(tri.positions.len()).expect("small templates");
    }
    out.push_str(&faces);
    Ok(out)
}

/// What [`write_lods`] writes beyond its stats.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Export {
    /// Stats only.
    None,
    /// The baked atlases and GLBs.
    Glb,
    /// Everything: also the review OBJs and the octahedral impostor.
    All,
}

/// Builds the LOD chain and returns a stats fragment.
///
/// [`Export::Glb`] also bakes each level's cluster cards into
/// `lods/lod<n>-cards/` and the impostor into `lods/impostor/`, and writes
/// the GLBs. [`Export::All`] adds each level as `lods/lod<n>-bark.obj` and
/// `lods/lod<n>-leaves.obj`, the cards as `lods/lod<n>-cards.obj`, and the
/// octahedral impostor.
fn write_lods(
    dir: &std::path::Path,
    export: Export,
    skeleton: &sylva_skeleton::Skeleton,
    foliage: &Foliage,
    bark: &TriMesh,
    textures: &card::Textures,
) -> Result<String, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let chain = build_lods(
        skeleton,
        foliage,
        &MeshParams::default(),
        &LodPolicy::default(),
    )?;
    let elapsed = started.elapsed();
    let materials = CardMaterials {
        bark: BakeMaterial {
            base_color: Some(&textures.bark),
            ..BakeMaterial::default()
        },
        leaf: BakeMaterial {
            base_color: textures.leaf.as_ref(),
            opacity: textures.leaf_opacity.as_ref(),
            ..BakeMaterial::default()
        },
    };
    let out = dir.join("lods");
    let mut levels = Vec::new();
    for (n, lod) in chain.levels.iter().enumerate() {
        let r = lod.report;
        levels.push(format!(
            "{{\"screen_size\":{},\"branches\":{},\"pruned\":{},\"bark_triangles\":{},\
             \"leaves\":{},\"leaf_triangles\":{},\"cluster_cards\":{},\"clustered_leaves\":{},\
             \"card_triangles\":{},\"triangles\":{}}}",
            lod.level.screen_size,
            r.branches,
            r.pruned_branches,
            r.bark_triangles,
            r.leaves,
            r.leaf_triangles,
            r.cluster_cards,
            r.clustered_leaves,
            r.card_triangles,
            r.triangles()
        ));
        if export != Export::None
            && let Some(clusters) = &lod.clusters
        {
            let baked = Instant::now();
            let atlas = bake_clusters(
                skeleton,
                bark,
                foliage,
                clusters,
                &materials,
                &AtlasSettings::default(),
            )?;
            println!(
                "lod{n}: {} cluster variants baked in {} ms",
                clusters.variants.len(),
                baked.elapsed().as_millis()
            );
            std::fs::create_dir_all(&out)?;
            write_atlas(&out.join(format!("lod{n}-cards")), &atlas)?;
            std::fs::write(
                out.join(format!("lod{n}-cards.obj")),
                bark_obj(&clusters.geometry())?,
            )?;
        }
        if export == Export::All {
            std::fs::create_dir_all(&out)?;
            let (tri, _) = lod.bark.mesh.to_trimesh(&ExtractParams {
                normals: NormalsSource::CustomOnly,
                ..ExtractParams::default()
            });
            std::fs::write(out.join(format!("lod{n}-bark.obj")), bark_obj(&tri)?)?;
            let templates: Vec<&exedra_mesh::Mesh> = lod.templates.iter().collect();
            std::fs::write(
                out.join(format!("lod{n}-leaves.obj")),
                instances_obj(&templates, &lod.leaves)?,
            )?;
        }
    }
    if export != Export::None
        && let Some(impostor) = &chain.impostor
    {
        let baked = Instant::now();
        let atlas = bake_impostor(
            bark,
            foliage,
            impostor,
            &materials,
            &AtlasSettings {
                cell: [512, 512],
                samples: 4,
            },
        )?;
        println!(
            "impostor: {} planes baked in {} ms",
            impostor.views.len(),
            baked.elapsed().as_millis()
        );
        write_atlas(&out.join("impostor"), &atlas)?;
        std::fs::write(out.join("impostor.obj"), bark_obj(&impostor.geometry())?)?;
    }
    if export == Export::All && chain.impostor.is_some() {
        // An octahedral impostor for review beside the crossed one: 8 x 8
        // frames over the upper hemisphere.
        let octahedral = Impostor::fit(
            skeleton,
            foliage,
            ImpostorPolicy {
                layout: ImpostorLayout::Octahedral { frames: 8 },
                ..ImpostorPolicy::default()
            },
        );
        let baked = Instant::now();
        let atlas = bake_impostor(
            bark,
            foliage,
            &octahedral,
            &materials,
            &AtlasSettings {
                cell: [256, 256],
                samples: 4,
            },
        )?;
        println!(
            "octahedral impostor: {} frames baked in {} ms",
            octahedral.views.len(),
            baked.elapsed().as_millis()
        );
        write_atlas(&out.join("octahedral"), &atlas)?;
        let view = &octahedral.views[0];
        std::fs::write(
            out.join("octahedral.json"),
            format!(
                "{{\"frames\":8,\"center\":[{},{},{}],\"radius\":{}}}\n",
                view.center.x, view.center.y, view.center.z, view.half.x
            ),
        )?;
    }
    if export != Export::None {
        write_glbs(dir, skeleton, foliage, &chain)?;
    }
    Ok(format!(
        ",\"lods\":{{\"build_us\":{},\"levels\":[{}],\"impostor_planes\":{}}}",
        elapsed.as_micros(),
        levels.join(","),
        chain.impostor.as_ref().map_or(0, |i| i.views.len())
    ))
}

/// Builds the tree asset and writes each level, the impostor last, as
/// `glb/lod<n>.glb` (and `glb/lod<n>-instanced.glb` for levels with
/// individual leaves), with the species' texture sets and the baked atlases
/// already written beside it.
fn write_glbs(
    dir: &std::path::Path,
    skeleton: &sylva_skeleton::Skeleton,
    foliage: &Foliage,
    chain: &sylva_lod::LodChain,
) -> Result<(), Box<dyn std::error::Error>> {
    let asset = build_asset(skeleton, foliage, chain, &TreeMaterials::default())?;
    let species = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.split("-seed").next())
        .ok_or("seed directory name")?;
    let textures_dir = dir.parent().ok_or("gallery directory")?.join("textures");
    // Encoded PNG images per material, in asset material order.
    let sets: Vec<PathBuf> = asset
        .materials
        .iter()
        .map(|m| match m.role {
            MaterialRole::Bark => textures_dir.join(format!("{species}-bark/gltf")),
            MaterialRole::Leaf => textures_dir.join(format!("{species}-leaf/gltf")),
            MaterialRole::Cards { level } => dir.join(format!("lods/lod{level}-cards")),
            MaterialRole::Impostor => dir.join("lods/impostor"),
        })
        .collect();
    let read =
        |set: &std::path::Path, name: &str| std::fs::read(set.join(format!("{name}.png"))).ok();
    let images: Vec<[Option<Vec<u8>>; 4]> = sets
        .iter()
        .map(|set| {
            [
                read(set, "base_color"),
                read(set, "orm"),
                read(set, "normal"),
                read(set, "diffuse_transmission"),
            ]
        })
        .collect();
    let textures: Vec<MaterialTextures<'_>> = images
        .iter()
        .map(
            |[base_color, orm, normal, diffuse_transmission]| MaterialTextures {
                base_color: base_color.as_deref(),
                orm: orm.as_deref(),
                normal: normal.as_deref(),
                diffuse_transmission: diffuse_transmission.as_deref(),
            },
        )
        .collect();
    let out = dir.join("glb");
    std::fs::create_dir_all(&out)?;
    for (level, lod) in asset.lods.iter().enumerate() {
        let mut exports = vec![(format!("lod{level}.glb"), LeafExport::Merged)];
        if lod.leaves.is_some() {
            exports.push((format!("lod{level}-instanced.glb"), LeafExport::Instanced));
        }
        for (name, leaves) in exports {
            let started = Instant::now();
            let options = ExportOptions::default().with_leaves(leaves);
            let glb = export_lod_glb_with(&asset, level, &textures, options)?;
            println!(
                "{name}: {} KiB, {} images, written in {} ms",
                glb.bytes.len() / 1024,
                glb.stats.images,
                started.elapsed().as_millis()
            );
            std::fs::write(out.join(name), &glb.bytes)?;
        }
    }
    Ok(())
}

/// Packs a baked atlas for glTF (colour with coverage alpha, normals) and
/// writes PNG and KTX2, plus the depth map.
fn write_atlas(dir: &std::path::Path, atlas: &Atlas) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let bundle = pack(
        &atlas.baked.maps(),
        Profile::Gltf,
        &PackSettings {
            filter: Filter::Kaiser,
            alpha_cutoff: Some(0.5),
            ..PackSettings::default()
        },
    )?;
    for texture in &bundle.textures {
        std::fs::write(
            dir.join(format!("{}.png", texture.name)),
            dapple_encode::png::write(texture)?,
        )?;
        std::fs::write(
            dir.join(format!("{}.ktx2", texture.name)),
            ktx2::write(texture),
        )?;
    }
    card::write_gray(&dir.join("depth.png"), &atlas.baked.depth)?;
    Ok(())
}

/// Writes the first template's coverage mask as an 8-bit grayscale PNG.
fn write_mask(path: &std::path::Path, foliage: &Foliage) -> Result<(), Box<dyn std::error::Error>> {
    let mask = sylva_texture::leaf_mask(&foliage.templates[0].shape, 256)?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "coverage in [0, 1], in steps of 1/255"
    )]
    let coverage: Vec<u8> = mask
        .values()
        .iter()
        .map(|&c| (c * 255.0).round() as u8)
        .collect();
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), mask.width(), mask.height());
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    // Row 0 (`v = 0`, the leaf base) is the top row, as in dapple's PNG output.
    encoder.write_header()?.write_image_data(&coverage)?;
    Ok(())
}

/// Generates the species' bark and leaf sets and writes them per profile.
fn write_textures(
    dir: &std::path::Path,
    species: &Species,
    preset: &Preset,
) -> Result<card::Textures, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let recipe: dapple_graph::Recipe = toml::from_str(&preset.bark)?;
    let bark_set = bark(&recipe)?;
    let mut textures = card::Textures {
        bark: bark_set
            .maps
            .base_color
            .clone()
            .expect("bark sets have colour"),
        leaf: None,
        leaf_opacity: None,
    };
    let mut sets = vec![(format!("{}-bark", species.name), bark_set.maps)];
    if let Some(foliage) = &species.foliage {
        let look: LeafLook = match &preset.leaf {
            Some(source) => ron::from_str(source)?,
            None => LeafLook::default(),
        };
        let leaf_set = leaf(&LeafRecipe {
            shape: foliage.shape,
            green: look.green,
            vein: look.vein,
            translucent: look.translucent,
            translucency: look.translucency,
            mottle: look.mottle,
            roughness: look.roughness,
            ..LeafRecipe::default()
        })?;
        textures.leaf = leaf_set.maps.base_color.clone();
        textures.leaf_opacity = leaf_set.maps.opacity.clone();
        sets.push((format!("{}-leaf", species.name), leaf_set.maps));
    }
    let generated = started.elapsed();
    for (name, maps) in &sets {
        for (profile, label) in [(Profile::Lightweald, "lightweald"), (Profile::Gltf, "gltf")] {
            let settings = PackSettings {
                filter: Filter::Kaiser,
                alpha_cutoff: maps.opacity.as_ref().map(|_| 0.5),
                ..PackSettings::default()
            };
            let bundle = pack(maps, profile, &settings)?;
            let out = dir.join(name).join(label);
            std::fs::create_dir_all(&out)?;
            for texture in &bundle.textures {
                std::fs::write(
                    out.join(format!("{}.ktx2", texture.name)),
                    ktx2::write(texture),
                )?;
                std::fs::write(
                    out.join(format!("{}.png", texture.name)),
                    dapple_encode::png::write(texture)?,
                )?;
            }
        }
    }
    println!(
        "textures for {} generated in {} ms",
        species.name,
        generated.as_millis()
    );
    Ok(textures)
}

#[cfg(test)]
mod tests {
    use sylva_species::Species;

    use super::{LeafLook, OAK, PRESETS};

    /// Every built-in preset parses, with its bark recipe and leaf colours,
    /// and grows.
    #[test]
    fn presets_parse_and_grow() {
        for (species, bark, leaf) in PRESETS {
            let species: Species = ron::from_str(species).expect("species");
            let _: dapple_graph::Recipe = toml::from_str(bark).expect("bark recipe");
            if let Some(leaf) = leaf {
                let _: LeafLook = ron::from_str(leaf).expect("leaf colours");
            }
            let grown = species.grow(1).expect("grow");
            assert!(grown.report.sites > 0, "{} carries foliage", species.name);
        }
    }

    /// The spruce is excurrent: its trunk is the leader, reaching above
    /// every branch, and its crown narrows upward into a cone.
    #[test]
    fn spruce_keeps_a_single_leader_and_a_conical_crown() {
        let species: Species = ron::from_str(PRESETS[1].0).expect("preset");
        assert_eq!(species.name, "spruce");
        for seed in 1..=4 {
            let grown = species.grow(seed).expect("grow");
            let branches = grown.skeleton.branches();
            let top = |b: &sylva_skeleton::Branch| {
                b.nodes
                    .iter()
                    .map(|n| n.position.z)
                    .fold(f32::MIN, f32::max)
            };
            let leader = top(&branches[0]);
            for branch in &branches[1..] {
                assert!(
                    top(branch) < leader,
                    "seed {seed}: a branch overtops the leader"
                );
            }
            // Horizontal reach of nodes in the lower and upper thirds.
            let reach = |lo: f32, hi: f32| {
                branches
                    .iter()
                    .flat_map(|b| &b.nodes)
                    .filter(|n| (lo * leader..hi * leader).contains(&n.position.z))
                    .map(|n| n.position.truncate().length())
                    .fold(0.0, f32::max)
            };
            let (lower, upper) = (reach(0.1, 0.4), reach(0.7, 1.0));
            assert!(upper < 0.6 * lower, "seed {seed}: {upper} vs {lower}");
        }
    }

    /// Scaffold and secondary branches of the oak must not curl back on
    /// themselves: their overall turn stays under 135 degrees (a limb may
    /// rise, then arch down), and any
    /// stretch of centerline at least a metre long spans at least half its
    /// arc length (a closed loop spans none, a semicircle about 64%).
    #[test]
    fn oak_branches_do_not_loop() {
        let species: Species = ron::from_str(OAK).expect("preset");
        for seed in 1..=8 {
            let grown = species.grow(seed).expect("grow");
            for branch in grown.skeleton.branches() {
                if branch.order == 0 || branch.order > 2 {
                    continue;
                }
                let p: Vec<_> = branch.nodes.iter().map(|n| n.position).collect();
                let first = (p[1] - p[0]).normalize();
                let last = (p[p.len() - 1] - p[p.len() - 2]).normalize();
                let turn = libm_acos(first.dot(last)).to_degrees();
                assert!(
                    turn < 135.0,
                    "seed {seed}: order {} turns {turn}",
                    branch.order
                );
                let arcs = branch.arc_lengths();
                for i in 0..p.len() {
                    for j in i + 2..p.len() {
                        let arc = arcs[j] - arcs[i];
                        if arc >= 1.0 {
                            let ratio = p[i].distance(p[j]) / arc;
                            assert!(
                                ratio >= 0.5,
                                "seed {seed}: order {} curls back (chord/arc {ratio})",
                                branch.order
                            );
                        }
                    }
                }
            }
        }
    }

    fn libm_acos(x: f32) -> f32 {
        x.clamp(-1.0, 1.0).acos()
    }
}
