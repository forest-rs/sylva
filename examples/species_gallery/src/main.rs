// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Grows a species preset for several seeds and writes, per seed, a skeleton
//! dump and the bark mesh for Blender review.
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
//! ```
//!
//! Outputs default to `.local/gallery/`, which is git-ignored and survives
//! `cargo clean`; `target/` holds build artifacts only.
//!
//! `--seeds 1,3` grows only those seeds, and `--tree-only` skips the LOD
//! chain, card bakes and glTF export, for quick crown iteration.
//!
//! `bark.obj` carries positions, bark UVs and the authored normals;
//! `render_bark.py` shows it with a UV grid so seams and texel density are
//! visible. `leaves.obj` expands every leaf instance at full detail,
//! `leaf-mask.png` is the first template's coverage mask, and
//! `render_tree.py` renders bark and leaves together, textured.
//!
//! Per species, `textures/<species>-bark` and `textures/<species>-leaf` hold
//! the generated material sets, packed for `lightweald` and `gltf`: KTX2
//! files with full mip chains and PNG images of level 0. The leaf's transmitted
//! colour is `translucency.png`, since dapple's profiles carry no
//! transmission slot yet.

mod card;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use dapple_encode::{Filter, PackSettings, Profile, ktx2, pack};
use exedra_mesh::{ExtractAttribute, ExtractParams, NormalsSource, TriMesh};
use sylva_asset::{MaterialRole, TreeMaterials, build_asset};
use sylva_bake::BakeMaterial;
use sylva_foliage::{Foliage, leaf_mask, place_leaves};
use sylva_gltf::{MaterialTextures, export_lod_glb};
use sylva_lod::{
    Atlas, AtlasSettings, CardMaterials, LodPolicy, bake_clusters, bake_impostor, build_lods,
};
use sylva_mesh::{BRANCH_LAYER, MeshParams, mesh_skeleton};
use sylva_texture::{BarkRecipe, LeafRecipe, bark, leaf};

use skeleton_dump::{skeleton_json, skeleton_obj};
use sylva_species::Species;

const OAK: &str = include_str!("../presets/oak.ron");
const SEEDS: [u64; 3] = [1, 2, 3];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out_dir = PathBuf::from(".local/gallery/species-gallery");
    let mut seeds = SEEDS.to_vec();
    let mut tree_only = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seeds" => {
                let list = args.next().ok_or("--seeds needs a value, e.g. 1,3")?;
                seeds = list.split(',').map(str::parse).collect::<Result<_, _>>()?;
            }
            "--tree-only" => tree_only = true,
            _ => out_dir = PathBuf::from(arg),
        }
    }
    let species: Species = ron::from_str(OAK)?;
    let textures = write_textures(&out_dir.join("textures"), &species)?;
    for seed in seeds {
        let started = Instant::now();
        let grown = species.grow(seed)?;
        let elapsed = started.elapsed();
        let dir = out_dir.join(format!("{}-seed{seed}", species.name));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("skeleton.json"), skeleton_json(&grown.skeleton)?)?;
        std::fs::write(dir.join("skeleton.obj"), skeleton_obj(&grown.skeleton)?)?;
        let meshed = Instant::now();
        let bark = mesh_skeleton(&grown.skeleton, &MeshParams::default())?;
        let (tri, _) = bark.mesh.to_trimesh(&ExtractParams {
            normals: NormalsSource::CustomOnly,
            attributes: vec![ExtractAttribute::new(BRANCH_LAYER, u32::MAX)],
            ..ExtractParams::default()
        });
        let mesh_elapsed = meshed.elapsed();
        std::fs::write(dir.join("bark.obj"), bark_obj(&tri)?)?;
        let mesh = &bark.report;
        let leaves = match &species.foliage {
            Some(params) => {
                let placed = Instant::now();
                let foliage = place_leaves(&grown.skeleton, params)?;
                let place_us = placed.elapsed().as_micros();
                std::fs::write(dir.join("leaves.obj"), leaves_obj(&foliage)?)?;
                write_mask(&dir.join("leaf-mask.png"), &foliage)?;
                if tree_only {
                    println!(
                        "{}-seed{seed}: {:?} branches by level, {} leaves",
                        species.name, grown.report.branches_by_level, foliage.report.leaves
                    );
                    continue;
                }
                let lods = write_lods(&dir, seed, &grown.skeleton, &foliage, &tri, &textures)?;
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
    println!("wrote {}", out_dir.display());
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

/// Builds the LOD chain and returns a stats fragment; seed 1 also writes
/// each level as `lods/lod<n>-bark.obj` and `lods/lod<n>-leaves.obj`, its
/// cluster cards as `lods/lod<n>-cards.obj` with their baked atlas in
/// `lods/lod<n>-cards/`, and the impostor as `lods/impostor.obj` with
/// `lods/impostor/`.
fn write_lods(
    dir: &std::path::Path,
    seed: u64,
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
        if seed == 1
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
        if seed == 1 {
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
    if seed == 1
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
    if seed == 1 {
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
/// `glb/lod<n>.glb`, with the species' texture sets and the baked atlases
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
    let images: Vec<[Option<Vec<u8>>; 3]> = sets
        .iter()
        .map(|set| {
            [
                read(set, "base_color"),
                read(set, "orm"),
                read(set, "normal"),
            ]
        })
        .collect();
    let textures: Vec<MaterialTextures<'_>> = images
        .iter()
        .map(|[base_color, orm, normal]| MaterialTextures {
            base_color: base_color.as_deref(),
            orm: orm.as_deref(),
            normal: normal.as_deref(),
        })
        .collect();
    let out = dir.join("glb");
    std::fs::create_dir_all(&out)?;
    for level in 0..asset.lods.len() {
        let started = Instant::now();
        let glb = export_lod_glb(&asset, level, &textures)?;
        println!(
            "lod{level}.glb: {} KiB, {} images, written in {} ms",
            glb.bytes.len() / 1024,
            glb.stats.images,
            started.elapsed().as_millis()
        );
        std::fs::write(out.join(format!("lod{level}.glb")), &glb.bytes)?;
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
    let mask = leaf_mask(&foliage.templates[0].shape, 256);
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), mask.width, mask.height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    // Row 0 (`v = 0`, the leaf base) is the top row, as in dapple's PNG output.
    encoder.write_header()?.write_image_data(&mask.coverage)?;
    Ok(())
}

/// Generates the species' bark and leaf sets and writes them per profile.
fn write_textures(
    dir: &std::path::Path,
    species: &Species,
) -> Result<card::Textures, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let bark_set = bark(&BarkRecipe::default())?;
    let mut textures = card::Textures {
        bark: bark_set
            .maps
            .base_color
            .clone()
            .expect("bark sets have colour"),
        leaf: None,
    };
    let mut sets = vec![(format!("{}-bark", species.name), bark_set.maps, None)];
    if let Some(foliage) = &species.foliage {
        let leaf_set = leaf(&LeafRecipe {
            shape: foliage.shape,
            ..LeafRecipe::default()
        })?;
        textures.leaf = leaf_set.maps.base_color.clone();
        sets.push((
            format!("{}-leaf", species.name),
            leaf_set.maps,
            Some(leaf_set.translucency),
        ));
    }
    let generated = started.elapsed();
    for (name, maps, translucency) in &sets {
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
            if let Some(image) = translucency {
                write_srgb_png(&out.join("translucency.png"), image)?;
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

/// Writes a 3-channel linear image as an 8-bit sRGB PNG.
fn write_srgb_png(
    path: &std::path::Path,
    image: &dapple_encode::Image,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), image.width(), image.height());
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    // Row 0 (`v = 0`) is the top row, as in dapple's PNG output.
    let mut data = Vec::with_capacity(image.values().len());
    {
        data.extend(image.values().iter().map(|&v| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a clamped unit value scaled to a byte"
            )]
            let byte = libm_round(dapple_encode::linear_to_srgb(v) * 255.0) as u8;
            byte
        }));
    }
    encoder.write_header()?.write_image_data(&data)?;
    Ok(())
}

fn libm_round(v: f32) -> f32 {
    v.round()
}

#[cfg(test)]
mod tests {
    use sylva_species::Species;

    use super::OAK;

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
