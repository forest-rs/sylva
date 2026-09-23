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
//!   blender --background --python examples/species_gallery/tools/render_tree.py -- "$d"
//! done
//! ```
//!
//! `bark.obj` carries positions, bark UVs and the authored normals;
//! `render_bark.py` shows it with a UV grid so seams and texel density are
//! visible. `leaves.obj` expands every leaf instance at full detail,
//! `leaf-mask.png` is the first template's coverage mask, and
//! `render_tree.py` renders bark and leaves together.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use exedra_mesh::{ExtractAttribute, ExtractParams, NormalsSource, TriMesh};
use sylva_foliage::{Foliage, leaf_mask, place_leaves};
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
        let leaves = match &species.foliage {
            Some(params) => {
                let placed = Instant::now();
                let foliage = place_leaves(&grown.skeleton, params)?;
                let place_us = placed.elapsed().as_micros();
                std::fs::write(dir.join("leaves.obj"), leaves_obj(&foliage)?)?;
                write_mask(&dir.join("leaf-mask.png"), &foliage)?;
                let r = &foliage.report;
                format!(
                    ",\"foliage\":{{\"leaves\":{},\"templates\":{},\"triangles\":{},\"place_us\":{place_us}}}",
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

/// Writes every leaf instance at full detail as one OBJ with UVs.
fn leaves_obj(foliage: &Foliage) -> Result<String, std::fmt::Error> {
    let templates: Vec<TriMesh> = foliage
        .templates
        .iter()
        .map(|t| t.mesh.to_trimesh(&ExtractParams::default()).0)
        .collect();
    let mut out = String::from("# sylva leaves\n");
    let mut faces = String::new();
    let mut base = 1_u32;
    for leaf in &foliage.instances {
        let tri = &templates[leaf.template as usize];
        for (p, uv) in tri.positions.iter().zip(&tri.uvs) {
            let world = leaf.position
                + leaf.rotation * (sylva_skeleton::glam::Vec3::from_array(*p) * leaf.scale);
            writeln!(out, "v {} {} {}", world.x, world.y, world.z)?;
            writeln!(out, "vt {} {}", uv[0], uv[1])?;
        }
        for face in tri.indices.as_chunks::<3>().0 {
            let [a, b, c] = face.map(|i| i + base);
            writeln!(faces, "f {a}/{a} {b}/{b} {c}/{c}")?;
        }
        base += u32::try_from(tri.positions.len()).expect("small templates");
    }
    out.push_str(&faces);
    Ok(out)
}

/// Writes the first template's coverage mask as an 8-bit grayscale PNG.
fn write_mask(path: &std::path::Path, foliage: &Foliage) -> Result<(), Box<dyn std::error::Error>> {
    let mask = leaf_mask(&foliage.templates[0].shape, 256);
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), mask.width, mask.height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    // PNG rows run top-down; the mask runs from v = 0 (the leaf base).
    let mut rows = Vec::with_capacity(mask.coverage.len());
    for row in mask.coverage.chunks_exact(mask.width as usize).rev() {
        rows.extend_from_slice(row);
    }
    encoder.write_header()?.write_image_data(&rows)?;
    Ok(())
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
