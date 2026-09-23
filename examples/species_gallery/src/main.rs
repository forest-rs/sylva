// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Grows a species preset for several seeds and writes one skeleton dump per
//! seed for Blender review.
//!
//! ```sh
//! cargo run -p species_gallery -- target/species-gallery
//! for d in target/species-gallery/oak-*; do
//!   blender --background --python examples/skeleton_dump/tools/render.py -- "$d"
//! done
//! ```

use std::path::PathBuf;
use std::time::Instant;

use skeleton_dump::{skeleton_json, skeleton_obj};
use sylva_species::Species;

const OAK: &str = include_str!("../presets/oak.ron");
const SEEDS: [u64; 3] = [1, 2, 3];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/species-gallery".to_owned()),
    );
    let species: Species = ron::from_str(OAK)?;
    for seed in SEEDS {
        let started = Instant::now();
        let grown = species.grow(seed)?;
        let elapsed = started.elapsed();
        let dir = out_dir.join(format!("{}-seed{seed}", species.name));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("skeleton.json"), skeleton_json(&grown.skeleton)?)?;
        std::fs::write(dir.join("skeleton.obj"), skeleton_obj(&grown.skeleton)?)?;
        let report = &grown.report;
        let by_level: Vec<String> = report
            .branches_by_level
            .iter()
            .map(ToString::to_string)
            .collect();
        let stats = format!(
            "{{\"species\":\"{}\",\"seed\":{seed},\"branches_by_level\":[{}],\"nodes\":{},\
             \"truncated\":{},\"removed\":{},\"skipped\":{},\"sites\":{},\
             \"max_radius_m\":{},\"grow_us\":{}}}\n",
            species.name,
            by_level.join(","),
            report.nodes,
            report.truncated,
            report.removed,
            report.skipped,
            report.sites,
            report.pipe.max_radius,
            elapsed.as_micros(),
        );
        std::fs::write(dir.join("stats.json"), &stats)?;
        print!("{stats}");
    }
    println!("wrote {}", out_dir.display());
    Ok(())
}
