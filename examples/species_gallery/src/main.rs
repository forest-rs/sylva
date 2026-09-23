// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Grows a species preset for several seeds and writes, per seed, a skeleton
//! dump and the bark mesh for Blender review.
//!
//! ```sh
//! cargo run --release -p species_gallery -- target/species-gallery
//! for d in target/species-gallery/oak-*; do
//!   blender --background --python examples/skeleton_dump/tools/render.py -- "$d"
//!   blender --background --python examples/species_gallery/tools/render_bark.py -- "$d"
//! done
//! ```
//!
//! `bark.obj` carries positions, bark UVs and the authored normals;
//! `render_bark.py` shows it with a UV grid so seams and texel density are
//! visible.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use exedra_mesh::{ExtractAttribute, ExtractParams, NormalsSource, TriMesh};
use sylva_mesh::{BRANCH_LAYER, MeshParams, mesh_skeleton};

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
             \"mesh_us\":{}}}}}\n",
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
    for uv in &tri.uvs {
        writeln!(out, "vt {} {}", uv[0], uv[1])?;
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
