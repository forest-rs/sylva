// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Export a preset directly as detailed instanced geometry, without an LOD chain.
//! Run `cargo run -p species_gallery --bin detailed -- spruce /tmp/sylva-detailed`.

use std::path::PathBuf;
use std::time::Instant;

use sylva_asset::{GeneratedTree, TreeMaterials, build_detailed};
use sylva_gltf::{MaterialTextures, export_detailed_glb};
use sylva_mesh::MeshParams;
use sylva_species::Species;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_else(|| "spruce".into());
    let out = PathBuf::from(
        args.next()
            .unwrap_or_else(|| ".local/gallery/detailed".into()),
    );
    let preset = match name.as_str() {
        "spruce" => include_str!("../../presets/spruce.ron"),
        "oak" => include_str!("../../presets/oak.ron"),
        "beech" => include_str!("../../presets/beech.ron"),
        "birch" => include_str!("../../presets/birch.ron"),
        _ => return Err("expected spruce, oak, beech or birch".into()),
    };
    let species: Species = ron::from_str(preset)?;
    let started = Instant::now();
    let grown = species.grow(1)?;
    let mut materials = TreeMaterials::default();
    // Plain colours make geometric gaps visible without relying on coverage
    // textures. This is a geometry inspection asset, not a calibrated render.
    materials.bark.base_color.components = [0.2, 0.12, 0.07];
    materials.leaf.base_color.components = [0.12, 0.28, 0.06];
    let tree = GeneratedTree::new(grown.skeleton, species.foliage.as_ref(), materials)?;
    let source_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let detailed = build_detailed(&tree, &MeshParams::default())?;
    let build_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let glb = export_detailed_glb(&detailed, &[MaterialTextures::default(); 2])?;
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join(format!("{name}-detailed.glb")), &glb.bytes)?;
    println!("{name}: {:?}", detailed.report);
    println!(
        "source {source_ms} ms; realization {build_ms} ms; export {} ms; {} bytes",
        started.elapsed().as_millis(),
        glb.bytes.len()
    );
    Ok(())
}
