// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leaf template meshes: the full blade and the single-leaf card.

use alloc::vec::Vec;

use exedra_mesh::{CornerId, Mesh, MeshBuilder, op};
use glam::{Vec2, Vec3};

use crate::{FoliageError, LeafShape};

/// Builds a mesh from leaf-space faces, each corner carrying its flat
/// leaf-space position (for UVs) and its shaped position.
fn build(faces: &[Vec<(Vec2, Vec3)>], shape: &LeafShape) -> Result<Mesh, FoliageError> {
    let mut builder = MeshBuilder::new();
    let mut keys: Vec<(Vec2, u32)> = Vec::new();
    let mut loops: Vec<Vec<u32>> = Vec::with_capacity(faces.len());
    for face in faces {
        let mut indices = Vec::with_capacity(face.len());
        for &(flat, shaped) in face {
            // Weld corners that share a flat position.
            let index = if let Some(&(_, i)) = keys.iter().rev().find(|(k, _)| *k == flat) {
                i
            } else {
                let i = builder.push_vertex(shaped.to_array());
                keys.push((flat, i));
                i
            };
            indices.push(index);
        }
        builder.add_face(&indices).map_err(FoliageError::Build)?;
        loops.push(indices);
    }
    let built = builder.build().map_err(FoliageError::Build)?;
    let mut mesh = built.mesh;
    let flat_of = |local: u32| keys.iter().find(|(_, i)| *i == local).map(|(k, _)| *k);
    let mut corners: Vec<(CornerId, [f32; 2])> = Vec::new();
    for (face, edges) in built.face_edge_ids.iter().enumerate() {
        let n = loops[face].len();
        for (i, &local) in loops[face].iter().enumerate() {
            let flat = flat_of(local).expect("every loop vertex was keyed");
            // Edge `i - 1` ends at loop vertex `i`.
            corners.push((edges[(i + n - 1) % n], shape.uv(flat)));
        }
    }
    corners.sort_by_key(|(corner, _)| corner.index());
    let mut edit = mesh.edit();
    for (corner, uv) in corners {
        op::set_corner_uv(&mut edit, corner, uv).map_err(|_| FoliageError::Kernel)?;
    }
    let _: () = edit.finish();
    Ok(mesh)
}

/// Shaped position of a flat leaf-space point: V-folded along the midrib
/// and drooping toward the tip.
fn shaped(shape: &LeafShape, p: Vec2) -> Vec3 {
    let t = p.y / shape.length;
    let z = p.x.abs() * libm::tanf(shape.fold) - shape.curl * shape.length * t * t;
    Vec3::new(p.x, p.y, z)
}

/// Meshes the full leaf blade in leaf space (see [`LeafShape`]).
///
/// The blade is a strip along the midrib: a base vertex, a left, middle and
/// right vertex at every interior station, and a tip vertex, joined by quads
/// and triangle fans. It is folded and curled by the shape, faces `+Z`, and
/// carries corner UVs in the mask frame ([`LeafShape::uv`]). Extract with
/// derived normals and render it two-sided.
///
/// # Errors
///
/// [`FoliageError::Params`] for an invalid shape, or a kernel error.
pub fn leaf_mesh(shape: &LeafShape) -> Result<Mesh, FoliageError> {
    shape.validate()?;
    let ts = shape.station_ts();
    let at = |t: f32, side: f32| {
        let flat = shape.skewed(Vec2::new(side * shape.half_width(t), t * shape.length));
        (flat, shaped(shape, flat))
    };
    let base = at(0.0, 0.0);
    let tip = at(1.0, 0.0);
    let mut faces: Vec<Vec<(Vec2, Vec3)>> = Vec::new();
    let first = ts[1];
    faces.push(vec_of(&[base, at(first, 1.0), at(first, 0.0)]));
    faces.push(vec_of(&[base, at(first, 0.0), at(first, -1.0)]));
    for pair in ts[1..ts.len() - 1].windows(2) {
        let (a, b) = (pair[0], pair[1]);
        faces.push(vec_of(&[at(a, 0.0), at(a, 1.0), at(b, 1.0), at(b, 0.0)]));
        faces.push(vec_of(&[at(a, -1.0), at(a, 0.0), at(b, 0.0), at(b, -1.0)]));
    }
    let last = ts[ts.len() - 2];
    faces.push(vec_of(&[at(last, 0.0), at(last, 1.0), tip]));
    faces.push(vec_of(&[at(last, -1.0), at(last, 0.0), tip]));
    build(&faces, shape)
}

/// Meshes the single-leaf card: one flat quad covering the mask frame,
/// with UVs `(0, 0)` to `(1, 1)`. Pair it with [`leaf_mask`](crate::leaf_mask)
/// for alpha-tested foliage.
///
/// # Errors
///
/// [`FoliageError::Params`] for an invalid shape, or a kernel error.
pub fn card_mesh(shape: &LeafShape) -> Result<Mesh, FoliageError> {
    shape.validate()?;
    let h = shape.max_half_width();
    let l = shape.length;
    let corner = |x: f32, y: f32| {
        let flat = Vec2::new(x, y);
        (flat, Vec3::new(x, y, 0.0))
    };
    build(
        &[vec_of(&[
            corner(-h, 0.0),
            corner(h, 0.0),
            corner(h, l),
            corner(-h, l),
        ])],
        shape,
    )
}

fn vec_of(corners: &[(Vec2, Vec3)]) -> Vec<(Vec2, Vec3)> {
    corners.to_vec()
}
