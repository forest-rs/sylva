// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Ring construction and mesh assembly.

use alloc::vec::Vec;
use core::f32::consts::TAU;

use exedra_mesh::attributes::{AttrKey, Domain};
use exedra_mesh::{CornerId, HalfEdgeId, Mesh, MeshBuilder, VertexId, op};
use glam::Vec3;
use sylva_skeleton::keyed::tag;
use sylva_skeleton::{Branch, Frame, Skeleton};

use crate::{Junction, MeshError, MeshParams, RootFlare};

/// Vertex layer holding each vertex's branch, as its index in
/// [`Skeleton::branches`].
///
/// Branch surfaces are separate components, so every vertex belongs to one
/// branch. The index is the provenance link back to the skeleton, and the
/// key into per-branch tables such as wind pivots.
pub const BRANCH_LAYER: AttrKey<u32> = AttrKey::new(Domain::Vertex, "sylva.branch");

/// Smallest ring radius, in metres; thinner rings would collapse.
const MIN_RADIUS: f32 = 1e-4;

/// Deterministic counts describing one meshing run.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct MeshReport {
    /// Branches meshed.
    pub branches: u64,
    /// Branches skipped because their centerline has no length.
    pub skipped_branches: u64,
    /// Rings emitted, over all branches.
    pub rings: u64,
    /// Rings added for junction collars.
    pub collar_rings: u64,
    /// Rings added for root flares.
    pub flare_rings: u64,
    /// Mesh vertices.
    pub vertices: u64,
    /// Quad faces along branch walls.
    pub quads: u64,
    /// Triangles closing branch tips.
    pub tip_triangles: u64,
    /// Fewest segments around any meshed branch; 0 when none was meshed.
    pub min_segments: u32,
    /// Most segments around any meshed branch.
    pub max_segments: u32,
}

impl MeshReport {
    /// Triangles after extraction: two per quad plus the tip fans.
    #[must_use]
    pub fn triangles(&self) -> u64 {
        self.quads * 2 + self.tip_triangles
    }
}

/// Bark surfaces of a skeleton, with their report.
#[derive(Clone, Debug)]
pub struct BarkMesh {
    /// The mesh: one open-based, tip-capped tube per branch, with corner UVs,
    /// authored corner normals, UV seams and [`BRANCH_LAYER`].
    pub mesh: Mesh,
    /// Deterministic counts.
    pub report: MeshReport,
}

/// One ring position along a branch.
#[derive(Copy, Clone, Debug)]
struct Station {
    s: f32,
    position: Vec3,
    frame: Frame,
    radius: f32,
}

/// Profile modifiers that depend on the branch's role.
#[derive(Copy, Clone, Debug)]
enum Profile {
    Plain,
    Collar {
        length: f32,
        flare: f32,
        normal_blend: f32,
        parent_point: Vec3,
        parent_tangent: Vec3,
    },
    Flare(RootFlare),
}

impl Profile {
    /// Radius multiplier at arc length `s` and ring angle `theta`.
    fn scale(&self, s: f32, theta: f32) -> f32 {
        match *self {
            Self::Plain => 1.0,
            Self::Collar { length, flare, .. } => 1.0 + (flare - 1.0) * fade(s, length),
            Self::Flare(f) => {
                let decay = libm::expf(-s / f.height.max(1e-4));
                #[expect(clippy::cast_precision_loss, reason = "lobe counts are small integers")]
                let lobes = f.lobes as f32;
                let lobe = 1.0 - f.lobe_depth * 0.5 * (1.0 - libm::cosf(lobes * theta));
                1.0 + f.flare * decay * if f.lobes == 0 { 1.0 } else { lobe }
            }
        }
    }
}

/// 1 at `s = 0`, easing to 0 at `s = length`.
fn fade(s: f32, length: f32) -> f32 {
    if length <= 0.0 {
        return 0.0;
    }
    let x = (s / length).clamp(0.0, 1.0);
    1.0 - x * x * (3.0 - 2.0 * x)
}

/// Meshes every branch of `skeleton` as a bark tube.
///
/// Each branch becomes one tube with a constant segment count (see
/// [`RingResolution`](crate::RingResolution)), rings at curvature-adaptive
/// stations plus collar and flare rings, a tip cap, and an open base: a
/// child's base is embedded in its parent, and a root stem's base meets the
/// ground. Ring angle zero follows the skeleton frame's normal, which the
/// skeleton transports without twist, so rings do not twist either.
///
/// Surfaces carry corner UVs (see [`BarkMapping`](crate::BarkMapping)), UV
/// seams tagged on their seam edges, authored corner normals (extract with
/// `NormalsSource::CustomOnly`), and [`BRANCH_LAYER`].
///
/// # Errors
///
/// Returns [`MeshError::Skeleton`] when the skeleton fails validation (for
/// example missing radii or frames), [`MeshError::Params`] for invalid
/// parameters, and [`MeshError::Build`] if the mesh kernel rejects the
/// generated topology.
pub fn mesh_skeleton(skeleton: &Skeleton, params: &MeshParams) -> Result<BarkMesh, MeshError> {
    params.validate()?;
    skeleton.validate().map_err(MeshError::Skeleton)?;
    let mut builder = MeshBuilder::new();
    let mut report = MeshReport::default();
    // Per builder face, the (uv, normal) of each loop corner.
    let mut corner_data: Vec<[([f32; 2], [f32; 3]); 4]> = Vec::new();
    let mut face_sizes: Vec<u8> = Vec::new();
    // Builder-local vertex index -> branch index.
    let mut vertex_branch: Vec<u32> = Vec::new();
    // Builder face index and loop edge index of each seam edge.
    let mut seams: Vec<(usize, usize)> = Vec::new();

    for (index, branch) in skeleton.branches().iter().enumerate() {
        let branch_index = u32::try_from(index).map_err(|_| MeshError::TooLarge)?;
        let profile = profile_for(skeleton, branch, params);
        let Some(stations) = stations(branch, &profile, params, &mut report) else {
            report.skipped_branches += 1;
            continue;
        };
        let segments = segment_count(stations[0].radius, params);
        report.branches += 1;
        report.min_segments = if report.min_segments == 0 {
            segments
        } else {
            report.min_segments.min(segments)
        };
        report.max_segments = report.max_segments.max(segments);
        report.rings += stations.len() as u64;
        emit_tube(
            &mut builder,
            branch,
            branch_index,
            &stations,
            &profile,
            segments,
            params,
            &mut corner_data,
            &mut face_sizes,
            &mut vertex_branch,
            &mut seams,
            &mut report,
        )?;
    }

    let built = builder.build().map_err(MeshError::Build)?;
    let mut mesh = built.mesh;
    report.vertices = built.vertex_ids.len() as u64;
    mesh.define_dense_layer(BRANCH_LAYER, u32::MAX)
        .map_err(|_| MeshError::LayerConflict)?;

    // Sparse corner layers insert fastest in ascending ID order.
    let mut corners: Vec<(CornerId, [f32; 2], [f32; 3])> = Vec::new();
    for (face, edges) in built.face_edge_ids.iter().enumerate() {
        let n = usize::from(face_sizes[face]);
        for (loop_index, data) in corner_data[face][..n].iter().enumerate() {
            // Edge `i` runs from loop vertex `i` to `i + 1`; its destination
            // corner is loop vertex `i + 1`.
            let edge = edges[(loop_index + n - 1) % n];
            corners.push((edge, data.0, data.1));
        }
    }
    corners.sort_by_key(|(corner, ..)| corner.index());
    let mut seam_edges: Vec<HalfEdgeId> = seams
        .iter()
        .map(|&(face, edge)| built.face_edge_ids[face][edge])
        .collect();
    seam_edges.sort_by_key(|edge| edge.index());

    let mut edit = mesh.edit();
    for (corner, uv, normal) in &corners {
        op::set_corner_uv(&mut edit, *corner, *uv).map_err(|_| MeshError::Kernel)?;
        op::set_corner_normal_override(&mut edit, *corner, Some(*normal))
            .map_err(|_| MeshError::Kernel)?;
    }
    for edge in seam_edges {
        op::set_edge_seam(&mut edit, edge, true).map_err(|_| MeshError::Kernel)?;
    }
    for (vertex, &branch) in built.vertex_ids.iter().zip(&vertex_branch) {
        op::set_attribute(&mut edit, BRANCH_LAYER, *vertex, branch)
            .map_err(|_| MeshError::Kernel)?;
    }
    let _: () = edit.finish();
    Ok(BarkMesh { mesh, report })
}

fn profile_for(skeleton: &Skeleton, branch: &Branch, params: &MeshParams) -> Profile {
    match (branch.parent, params.junction) {
        (Some(attachment), Junction::Embedded(collar)) => {
            let Some(parent) = skeleton.branch(attachment.parent) else {
                return Profile::Plain;
            };
            let sample = parent.sample(attachment.t);
            Profile::Collar {
                length: collar.length * sample.radius.max(MIN_RADIUS),
                flare: collar.flare,
                normal_blend: collar.normal_blend,
                parent_point: sample.position,
                parent_tangent: sample.frame.tangent,
            }
        }
        (None, _) => params.root_flare.map_or(Profile::Plain, Profile::Flare),
    }
}

fn segment_count(radius: f32, params: &MeshParams) -> u32 {
    let r = params.rings;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to the validated segment range below"
    )]
    let wanted = libm::roundf(TAU * radius * r.segments_per_metre).max(0.0) as u32;
    wanted.clamp(r.min_segments, r.max_segments)
}

/// Ring stations along `branch`, or `None` for a branch without length.
fn stations(
    branch: &Branch,
    profile: &Profile,
    params: &MeshParams,
    report: &mut MeshReport,
) -> Option<Vec<Station>> {
    let lengths = branch.arc_lengths();
    let total = *lengths.last()?;
    if total <= 1e-6 {
        return None;
    }
    let mut at: Vec<f32> = Vec::new();
    at.push(0.0);
    // Profile rings near the base, where the radius changes fastest.
    let (extra, extent) = match *profile {
        Profile::Plain => (0, 0.0),
        Profile::Collar { length, .. } => match params.junction {
            Junction::Embedded(collar) => (collar.rings, length),
        },
        Profile::Flare(f) => (f.rings, 3.0 * f.height),
    };
    let extent = extent.min(0.5 * total);
    for k in 1..=extra {
        #[expect(clippy::cast_precision_loss, reason = "ring counts are small")]
        at.push(extent * k as f32 / (extra + 1) as f32);
    }
    let added_before = at.len();
    // Curvature- and spacing-driven rings at skeleton nodes.
    let mut last_s = 0.0_f32;
    let mut last_tangent = branch.nodes[0].frame.tangent;
    for (i, node) in branch.nodes.iter().enumerate().skip(1) {
        let s = lengths[i];
        let is_tip = i + 1 == branch.nodes.len();
        let bend = libm::acosf(last_tangent.dot(node.frame.tangent).clamp(-1.0, 1.0));
        if is_tip || bend >= params.stations.max_bend || s - last_s >= params.stations.max_spacing {
            at.push(s);
            last_s = s;
            last_tangent = node.frame.tangent;
        }
    }
    match *profile {
        Profile::Plain => {}
        Profile::Collar { .. } => report.collar_rings += (added_before - 1) as u64,
        Profile::Flare(_) => report.flare_rings += (added_before - 1) as u64,
    }
    at.sort_by(f32::total_cmp);
    at.dedup_by(|a, b| (*a - *b).abs() < 1e-5 * total.max(1.0));
    Some(
        at.iter()
            .map(|&s| station_at(branch, &lengths, s))
            .collect(),
    )
}

fn station_at(branch: &Branch, lengths: &[f32], s: f32) -> Station {
    let segment = lengths
        .windows(2)
        .position(|pair| s <= pair[1])
        .unwrap_or(lengths.len().saturating_sub(2));
    let (a, b) = (&branch.nodes[segment], &branch.nodes[segment + 1]);
    let span = lengths[segment + 1] - lengths[segment];
    let t = if span > 0.0 {
        ((s - lengths[segment]) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let tangent = a.frame.tangent.lerp(b.frame.tangent, t);
    let normal = a.frame.normal.lerp(b.frame.normal, t);
    Station {
        s,
        position: a.position.lerp(b.position, t),
        frame: Frame::from_tangent(tangent, normal).unwrap_or(a.frame),
        radius: (a.radius + (b.radius - a.radius) * t).max(MIN_RADIUS),
    }
}

/// Ring vertex position and outward normal at a station and angle.
fn ring_point(
    station: &Station,
    next: Option<&Station>,
    prev: Option<&Station>,
    profile: &Profile,
    theta: f32,
) -> (Vec3, Vec3) {
    let frame = station.frame;
    let radial = frame.normal * libm::cosf(theta) + frame.binormal() * libm::sinf(theta);
    let radius_at = |st: &Station| st.radius * profile.scale(st.s, theta);
    let r = radius_at(station);
    let position = station.position + radial * r;
    // Slope of the modulated radius along the centerline tilts the normal
    // toward the base on a taper and away from it on a flare.
    let (lo, hi) = (prev.unwrap_or(station), next.unwrap_or(station));
    let ds = hi.s - lo.s;
    let slope = if ds > 0.0 {
        (radius_at(hi) - radius_at(lo)) / ds
    } else {
        0.0
    };
    let mut normal = (radial - frame.tangent * slope).normalize_or(radial);
    if let Profile::Collar {
        length,
        normal_blend,
        parent_point,
        parent_tangent,
        ..
    } = *profile
    {
        let w = normal_blend * fade(station.s, length);
        if w > 0.0 {
            let away = position - parent_point;
            let parent_normal = (away - parent_tangent * away.dot(parent_tangent))
                .try_normalize()
                .unwrap_or(normal);
            normal = normal.lerp(parent_normal, w).normalize_or(normal);
        }
    }
    (position, normal)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "segment counts and repeats are small integers"
)]
fn emit_tube(
    builder: &mut MeshBuilder,
    branch: &Branch,
    branch_index: u32,
    stations: &[Station],
    profile: &Profile,
    segments: u32,
    params: &MeshParams,
    corner_data: &mut Vec<[([f32; 2], [f32; 3]); 4]>,
    face_sizes: &mut Vec<u8>,
    vertex_branch: &mut Vec<u32>,
    seams: &mut Vec<(usize, usize)>,
    report: &mut MeshReport,
) -> Result<(), MeshError> {
    let m = segments as usize;
    let base = &stations[0];
    let circumference = TAU * base.radius;
    let repeats = libm::roundf(circumference / params.bark.tile_size).max(1.0);
    let metres_per_v = circumference / repeats;
    let v_offset = branch
        .id
        .key(params.bark.seed)
        .with(tag("bark.v_offset"))
        .unit_f32();

    // Positions and normals per ring vertex.
    let mut ids: Vec<u32> = Vec::with_capacity(stations.len() * m + 1);
    let mut normals: Vec<Vec3> = Vec::with_capacity(stations.len() * m + 1);
    for (i, station) in stations.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| &stations[p]);
        let next = stations.get(i + 1);
        for j in 0..m {
            let theta = TAU * j as f32 / m as f32;
            let (position, normal) = ring_point(station, next, prev, profile, theta);
            ids.push(builder.push_vertex(position.to_array()));
            normals.push(normal);
            vertex_branch.push(branch_index);
        }
    }
    let uv = |ring: usize, j: usize| -> [f32; 2] {
        [
            repeats * j as f32 / m as f32,
            v_offset + stations[ring].s / metres_per_v,
        ]
    };
    for ring in 0..stations.len() - 1 {
        for j in 0..m {
            let j1 = (j + 1) % m;
            let a = ring * m + j;
            let b = ring * m + j1;
            let c = (ring + 1) * m + j1;
            let d = (ring + 1) * m + j;
            builder
                .add_face(&[ids[a], ids[b], ids[c], ids[d]])
                .map_err(MeshError::Build)?;
            // The last column closes the ring: its `j + 1` side is U = repeats.
            let u_next = if j1 == 0 { m } else { j1 };
            corner_data.push([
                (uv(ring, j), normals[a].to_array()),
                (uv(ring, u_next), normals[b].to_array()),
                (uv(ring + 1, u_next), normals[c].to_array()),
                (uv(ring + 1, j), normals[d].to_array()),
            ]);
            face_sizes.push(4);
            if j1 == 0 {
                // Loop edge 1 runs b -> c along the seam.
                seams.push((face_sizes.len() - 1, 1));
            }
            report.quads += 1;
        }
    }

    // Tip cap: a fan to a point one tip radius beyond the last ring.
    let tip = stations.last().expect("stations are non-empty");
    let tip_position = tip.position + tip.frame.tangent * tip.radius;
    let apex = builder.push_vertex(tip_position.to_array());
    vertex_branch.push(branch_index);
    let ring = stations.len() - 1;
    let apex_uv_v = v_offset + (tip.s + tip.radius) / metres_per_v;
    for j in 0..m {
        let j1 = (j + 1) % m;
        let a = ring * m + j;
        let b = ring * m + j1;
        builder
            .add_face(&[ids[a], ids[b], apex])
            .map_err(MeshError::Build)?;
        let u_next = if j1 == 0 { m } else { j1 };
        let u_mid = repeats * (j as f32 + 0.5) / m as f32;
        corner_data.push([
            (uv(ring, j), normals[a].to_array()),
            (uv(ring, u_next), normals[b].to_array()),
            ([u_mid, apex_uv_v], tip.frame.tangent.to_array()),
            ([0.0, 0.0], [0.0, 0.0, 0.0]),
        ]);
        face_sizes.push(3);
        report.tip_triangles += 1;
    }
    Ok(())
}

/// Returns the branch index stored for `vertex`, if any.
#[must_use]
pub fn branch_of(mesh: &Mesh, vertex: VertexId) -> Option<u32> {
    mesh.attrs()
        .dense(BRANCH_LAYER)
        .and_then(|layer| layer.get(vertex.as_id()))
        .copied()
        .filter(|&index| index != u32::MAX)
}
