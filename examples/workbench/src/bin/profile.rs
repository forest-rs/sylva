// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native profiling companion to the browser workload.
//! `profile oak generate 100` repeats the exact uncached workbench build.
//! `profile oak export 100` isolates export of one retained detailed specimen,
//! without material images (geometry and placement compilation are unchanged).

use serde_json::json;
use std::{hint::black_box, time::Instant};
use sylva_asset::{GeneratedTree, TreeMaterials, build_detailed};
use sylva_gltf::{MaterialTextures, export_detailed_glb};
use sylva_mesh::MeshParams;
use sylva_species::Species;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(3..=4).contains(&args.len()) || !matches!(args[1].as_str(), "generate" | "export") {
        return Err(
            "usage: profile <birch|oak|beech|spruce> <generate|export> <iterations> [output.glb]"
                .into(),
        );
    }
    let source = match args[0].as_str() {
        "birch" => include_str!("../../../species_gallery/presets/birch.ron"),
        "oak" => include_str!("../../../species_gallery/presets/oak.ron"),
        "beech" => include_str!("../../../species_gallery/presets/beech.ron"),
        "spruce" => include_str!("../../../species_gallery/presets/spruce.ron"),
        _ => return Err("unknown species".into()),
    };
    let iterations: u32 = args[2].parse()?;
    if iterations == 0 {
        return Err("iterations must be positive".into());
    }
    let recipe =
        json!({"version":1,"species":args[0],"seed":1,"density":1.0,"leaf_size":1.0}).to_string();
    let retained = if args[1] == "export" {
        let species: Species = ron::from_str(source)?;
        let grown = species.grow(1)?;
        let tree = GeneratedTree::new(
            grown.skeleton,
            species.foliage.as_ref(),
            TreeMaterials::default(),
        )?;
        Some(build_detailed(&tree, &MeshParams::default())?)
    } else {
        None
    };
    eprintln!(
        "profile pid={} species={} operation={} iterations={iterations}",
        std::process::id(),
        args[0],
        args[1]
    );
    for iteration in 0..iterations {
        let started = Instant::now();
        let (result, bytes) = if let Some(asset) = &retained {
            let glb = export_detailed_glb(black_box(asset), &[MaterialTextures::default(); 2])?;
            (
                json!({"bytes":glb.bytes.len(), "materials":"untextured", "branches":asset.branches.len(), "placements":asset.leaves.instances.len()}),
                glb.bytes,
            )
        } else {
            let mut specimen = sylva_workbench::generate(black_box(&recipe))?;
            let report: serde_json::Value = serde_json::from_str(&specimen.report())?;
            (
                json!({"counts":report["counts"],"timings":report["timings"],"source_revision":report["source_revision"]}),
                specimen.take_glb(),
            )
        };
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        if iteration + 1 == iterations
            && let Some(output) = args.get(3)
        {
            std::fs::write(output, &bytes)?;
        }
        black_box(bytes);
        println!(
            "{}",
            json!({"species":args[0],"operation":args[1],"iteration":iteration,"elapsed_ms":elapsed_ms,"result":result})
        );
    }
    Ok(())
}
