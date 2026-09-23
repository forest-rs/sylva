// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Ring construction and mesh assembly.

use alloc::vec;
use alloc::vec::Vec;
use core::f32::consts::TAU;

use exedra_mesh::attr;
use exedra_mesh::attributes::{AttrKey, Domain};
use exedra_mesh::{
    ChangeSink, CornerId, EditSession, FaceId, HalfEdgeId, Mesh, MeshBuilder, VertexId, op,
};
use exedra_mesh_ops::junction::{
    JunctionChart, JunctionError, JunctionOutput, JunctionParams, add_junction,
};
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

/// An opening's weld site and the index of the ring below it.
type GapRing = (usize, usize);

/// A corner's UV and authored normal.
type CornerData = ([f32; 2], [f32; 3]);

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
    /// Bands where a ring halves its segment count toward the tip.
    pub transition_bands: u64,
    /// Triangles in those bands, three per coarse segment.
    pub transition_triangles: u64,
    /// Fewest segments around any ring; 0 when none was meshed.
    pub min_segments: u32,
    /// Most segments around any ring.
    pub max_segments: u32,
    /// Forks joined by a welded skin.
    pub welded_junctions: u64,
    /// Major forks whose skin was refused, left embedded instead.
    pub weld_fallbacks: u64,
    /// Quads in welded skins.
    pub skin_quads: u64,
    /// Triangles in welded skins.
    pub skin_triangles: u64,
    /// Crotch center vertices added by welded skins.
    pub skin_vertices: u64,
    /// Mesh builds, one more for each round of refused welds.
    pub builds: u64,
}

impl MeshReport {
    /// Triangles after extraction: two per quad, plus transition bands, tip
    /// fans and welded skins.
    #[must_use]
    pub fn triangles(&self) -> u64 {
        (self.quads + self.skin_quads) * 2
            + self.transition_triangles
            + self.tip_triangles
            + self.skin_triangles
    }
}

/// Bark surfaces of a skeleton, with their report.
#[derive(Clone, Debug)]
pub struct BarkMesh {
    /// The mesh: one open-based, tip-capped tube per branch, joined by a skin
    /// at welded forks, with corner UVs, authored corner normals, UV seams
    /// and [`BRANCH_LAYER`].
    pub mesh: Mesh,
    /// Deterministic counts.
    pub report: MeshReport,
    /// Child branches joined to their parent by a welded skin, as indices in
    /// [`Skeleton::branches`], ascending.
    pub welds: Vec<u32>,
    /// Major forks left embedded because their skin was refused, in the
    /// order they were refused.
    pub weld_refusals: Vec<WeldRefusal>,
}

/// A major fork whose welded skin was refused.
#[derive(Clone, Debug, PartialEq)]
pub struct WeldRefusal {
    /// The child branch, as its index in [`Skeleton::branches`].
    pub branch: u32,
    /// Why the skin was refused.
    pub error: JunctionError,
}

/// A fork chosen for welding.
#[derive(Copy, Clone, Debug)]
struct WeldSite {
    child: usize,
    parent: usize,
    center: Vec3,
    /// Parent arc-length interval left open for the skin.
    gap: (f32, f32),
    /// Child arc length of its first ring.
    child_start: f32,
}

/// Where a branch's tube starts and which parent openings it leaves.
#[derive(Clone, Debug, Default)]
struct Layout {
    /// Arc length of the first ring; nonzero for a welded child.
    start: f32,
    /// Openings as (site, from, to) arc lengths, ascending.
    gaps: Vec<(usize, f32, f32)>,
}

/// A builder face and one of its loop edges.
type FaceEdge = (usize, usize);

/// Open rings and bark chart of one emitted tube.
///
/// Rings are named by an interior edge on them, which names the open ring
/// through its twin.
#[derive(Clone, Debug)]
struct TubeRings {
    /// An edge on the base ring.
    base: FaceEdge,
    /// Per opening: site, the edges on the rings below and above it, and the
    /// lower ring's arc length. The lower edge's twin leaves the ring's
    /// first vertex, which anchors the skin's chart.
    gaps: Vec<(usize, FaceEdge, FaceEdge, f32)>,
    repeats: f32,
    metres_per_v: f32,
    v_offset: f32,
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
        rings: u32,
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
/// With [`Junction::Welded`], each major fork instead opens its parent
/// around the fork, starts the child outside the parent, and closes the
/// three open ends with a junction skin. A refused skin leaves its fork
/// embedded and is recorded in [`BarkMesh::weld_refusals`]; since refusals
/// are found on the built mesh, each round of them costs one rebuild.
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
    let sites = weld_sites(skeleton, params);
    let mut active = vec![true; sites.len()];
    let mut refusals = Vec::new();
    let mut builds = 0;
    loop {
        builds += 1;
        let (mut bark, refused) = assemble(skeleton, params, &sites, &active)?;
        if refused.is_empty() {
            bark.report.builds = builds;
            bark.report.weld_fallbacks = refusals.len() as u64;
            bark.weld_refusals = refusals;
            return Ok(bark);
        }
        // Refusals only reopen embedded forks, which never overlap another
        // site's opening, so each round strictly shrinks the active set.
        for (site, error) in refused {
            active[site] = false;
            let branch = u32::try_from(sites[site].child).map_err(|_| MeshError::TooLarge)?;
            refusals.push(WeldRefusal { branch, error });
        }
    }
}

/// Major forks to weld, in branch order.
///
/// A child is a major fork when its parent is at least `min_radius` thick
/// at the attachment, its base radius is at least `min_ratio` times that,
/// its first ring
/// lies within the first half of its length, the opening leaves tube on both
/// sides of it on the parent, and the opening does not overlap an earlier
/// site's opening on the same parent.
fn weld_sites(skeleton: &Skeleton, params: &MeshParams) -> Vec<WeldSite> {
    let Junction::Welded(weld) = params.junction else {
        return Vec::new();
    };
    let branches = skeleton.branches();
    let mut starts = vec![0.0_f32; branches.len()];
    let mut sites: Vec<WeldSite> = Vec::new();
    for (child, branch) in branches.iter().enumerate() {
        let Some(attachment) = branch.parent else {
            continue;
        };
        let Some(parent) = skeleton.index_of(attachment.parent) else {
            continue;
        };
        let host = &branches[parent];
        let sample = host.sample(attachment.t);
        let radius = sample.radius.max(MIN_RADIUS);
        if radius < weld.min_radius || branch.nodes[0].radius < weld.min_ratio * radius {
            continue;
        }
        let length = host.length();
        let at = attachment.t.clamp(0.0, 1.0) * length;
        let (from, to) = (
            at - weld.parent_reach * radius,
            at + weld.parent_reach * radius,
        );
        let child_start = weld.child_reach * radius;
        let clear = sites
            .iter()
            .filter(|site| site.parent == parent)
            .all(|site| to + radius < site.gap.0 || from - radius > site.gap.1);
        if from <= starts[parent] + radius
            || to >= length - radius
            || child_start >= 0.5 * branch.length()
            || !clear
        {
            continue;
        }
        starts[child] = child_start;
        sites.push(WeldSite {
            child,
            parent,
            center: sample.position,
            gap: (from, to),
            child_start,
        });
    }
    sites
}

/// Meshes the skeleton with the `active` sites welded; returns the bark and
/// the sites whose skin was refused.
fn assemble(
    skeleton: &Skeleton,
    params: &MeshParams,
    sites: &[WeldSite],
    active: &[bool],
) -> Result<(BarkMesh, Vec<(usize, JunctionError)>), MeshError> {
    let branches = skeleton.branches();
    let mut layouts = vec![Layout::default(); branches.len()];
    for (index, site) in sites.iter().enumerate() {
        if active[index] {
            layouts[site.child].start = site.child_start;
            layouts[site.parent]
                .gaps
                .push((index, site.gap.0, site.gap.1));
        }
    }
    let mut builder = MeshBuilder::new();
    let mut report = MeshReport::default();
    // Per builder face, the (uv, normal) of each loop corner.
    let mut corner_data: Vec<[CornerData; 4]> = Vec::new();
    let mut face_sizes: Vec<u8> = Vec::new();
    // Builder-local vertex index -> branch index and authored normal.
    let mut vertex_branch: Vec<u32> = Vec::new();
    let mut vertex_normals: Vec<Vec3> = Vec::new();
    // Builder face index and loop edge index of each seam edge.
    let mut seams: Vec<(usize, usize)> = Vec::new();
    let mut tubes: Vec<Option<TubeRings>> = vec![None; branches.len()];

    for (index, branch) in branches.iter().enumerate() {
        let branch_index = u32::try_from(index).map_err(|_| MeshError::TooLarge)?;
        let layout = &layouts[index];
        let profile = profile_for(skeleton, branch, params, layout.start > 0.0);
        let Some((stations, gap_rings)) = stations(branch, &profile, layout, params, &mut report)
        else {
            report.skipped_branches += 1;
            continue;
        };
        let counts = ring_segments(&stations, segment_count(stations[0].radius, params), params);
        report.branches += 1;
        for &segments in &counts {
            report.min_segments = if report.min_segments == 0 {
                segments
            } else {
                report.min_segments.min(segments)
            };
            report.max_segments = report.max_segments.max(segments);
        }
        report.rings += stations.len() as u64;
        tubes[index] = Some(emit_tube(
            &mut builder,
            branch,
            branch_index,
            &stations,
            &gap_rings,
            &profile,
            &counts,
            params,
            &mut corner_data,
            &mut face_sizes,
            &mut vertex_branch,
            &mut vertex_normals,
            &mut seams,
            &mut report,
        )?);
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

    // Authored normals by vertex slot, for the skin's ring corners.
    let slots = built
        .vertex_ids
        .iter()
        .map(|vertex| vertex.index() as usize + 1)
        .max()
        .unwrap_or(0);
    let mut slot_normals = vec![Vec3::ZERO; slots];
    for (vertex, normal) in built.vertex_ids.iter().zip(&vertex_normals) {
        slot_normals[vertex.index() as usize] = *normal;
    }
    let mut refused = Vec::new();
    let mut welds = Vec::new();
    for (index, site) in sites.iter().enumerate() {
        if !active[index] {
            continue;
        }
        let (Some(parent), Some(child)) = (&tubes[site.parent], &tubes[site.child]) else {
            return Err(MeshError::Kernel);
        };
        let &(_, below, above, below_s) = parent
            .gaps
            .iter()
            .find(|gap| gap.0 == index)
            .ok_or(MeshError::Kernel)?;
        let seed = |(face, edge): FaceEdge| built.face_edge_ids[face][edge];
        let junction = JunctionParams {
            center: site.center.to_array().map(f64::from),
            rings: vec![seed(below), seed(above), seed(child.base)],
            chart: Some(JunctionChart {
                parent: 0,
                u_origin: 0.0,
                u_per_turn: f64::from(parent.repeats),
                v_origin: f64::from(parent.v_offset + below_s / parent.metres_per_v),
                v_per_unit: f64::from(1.0 / parent.metres_per_v),
            }),
            region: None,
        };
        match add_junction(&mut edit, &junction) {
            Ok(skin) => {
                let parent_index = u32::try_from(site.parent).map_err(|_| MeshError::TooLarge)?;
                finish_skin(&mut edit, &skin, &slot_normals, parent_index)?;
                report.welded_junctions += 1;
                welds.push(u32::try_from(site.child).map_err(|_| MeshError::TooLarge)?);
                report.skin_quads += skin.stats.bridge_quads + skin.stats.crotch_quads;
                report.skin_triangles += skin.stats.bridge_triangles;
                report.skin_vertices += skin.center_vertices.len() as u64;
                report.vertices += skin.center_vertices.len() as u64;
            }
            Err(error) => refused.push((index, error)),
        }
    }
    let _: () = edit.finish();
    let bark = BarkMesh {
        mesh,
        report,
        welds,
        weld_refusals: Vec::new(),
    };
    Ok((bark, refused))
}

/// Authors normals, provenance and seams on a fresh junction skin.
///
/// Ring corners take the tube's authored normal at their vertex, so shading
/// is continuous across the weld; crotch centers take the area-weighted
/// normal of their skin faces. Skin edges where the chart's U jumps, and
/// where the skin meets the rings of the upper parent and the child, are
/// tagged as seams.
fn finish_skin<S: ChangeSink>(
    edit: &mut EditSession<'_, S>,
    skin: &JunctionOutput,
    slot_normals: &[Vec3],
    parent: u32,
) -> Result<(), MeshError> {
    let mut center_normals = vec![Vec3::ZERO; skin.center_vertices.len()];
    let mut corners: Vec<(HalfEdgeId, VertexId)> = Vec::new();
    let mut edges: Vec<HalfEdgeId> = Vec::new();
    {
        let mesh = edit.mesh();
        for &face in &skin.faces {
            let loop_edges: Vec<HalfEdgeId> = mesh.face_loop(face).collect();
            let mut points: Vec<Vec3> = Vec::with_capacity(loop_edges.len());
            for &edge in &loop_edges {
                let vertex = mesh.to_vertex(edge).ok_or(MeshError::Kernel)?;
                let position = mesh.vertex_position(vertex).ok_or(MeshError::Kernel)?;
                points.push(Vec3::from_array(*position));
                corners.push((edge, vertex));
                edges.push(edge);
            }
            // Newell's normal: twice the area vector of the face.
            let mut area = Vec3::ZERO;
            for (i, a) in points.iter().enumerate() {
                area += a.cross(points[(i + 1) % points.len()]);
            }
            for &edge in &loop_edges {
                let vertex = mesh.to_vertex(edge).ok_or(MeshError::Kernel)?;
                if let Some(center) = skin.center_vertices.iter().position(|&c| c == vertex) {
                    center_normals[center] += area;
                }
            }
        }
    }
    for (corner, vertex) in corners {
        let normal = match skin.center_vertices.iter().position(|&c| c == vertex) {
            Some(center) => center_normals[center].normalize_or_zero(),
            None => slot_normals
                .get(vertex.index() as usize)
                .copied()
                .unwrap_or(Vec3::ZERO),
        };
        op::set_corner_normal_override(edit, corner, Some(normal.to_array()))
            .map_err(|_| MeshError::Kernel)?;
    }
    for &center in &skin.center_vertices {
        op::set_attribute(edit, BRANCH_LAYER, center, parent).map_err(|_| MeshError::Kernel)?;
    }
    // The chart reproduces the parent tube's UVs below the fork up to
    // rounding; snap those corners to the tube's exact values so the weld
    // is not a UV seam.
    let mut snaps: Vec<(HalfEdgeId, [f32; 2])> = Vec::new();
    {
        let mesh = edit.mesh();
        let layer = mesh.attrs().sparse(attr::CORNER_UV);
        let uv = |corner: HalfEdgeId| layer.and_then(|l| l.get(corner.as_id()).copied());
        for &edge in &edges {
            let (Some(twin), Some(before)) = (mesh.twin(edge), mesh.prev(edge)) else {
                continue;
            };
            let outer = mesh.face(twin);
            if outer.is_none_or(|face| face == FaceId::OUTSIDE || skin.faces.contains(&face)) {
                continue;
            }
            let Some(twin_before) = mesh.prev(twin) else {
                continue;
            };
            for (skin_corner, tube_corner) in [(edge, twin_before), (before, twin)] {
                if let (Some(a), Some(b)) = (uv(skin_corner), uv(tube_corner))
                    && a != b
                    && (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) < 1e-4
                {
                    snaps.push((skin_corner, b));
                }
            }
        }
    }
    for (corner, value) in snaps {
        op::set_corner_uv(edit, corner, value).map_err(|_| MeshError::Kernel)?;
    }
    for edge in edges {
        if edit.mesh().is_uv_discontinuous(edge) == Some(true) {
            op::set_edge_seam(edit, edge, true).map_err(|_| MeshError::Kernel)?;
        }
    }
    Ok(())
}

fn profile_for(skeleton: &Skeleton, branch: &Branch, params: &MeshParams, welded: bool) -> Profile {
    let collar = match params.junction {
        Junction::Embedded(collar) => collar,
        Junction::Welded(weld) => weld.collar,
    };
    match branch.parent {
        Some(_) if welded => Profile::Plain,
        Some(attachment) => {
            let Some(parent) = skeleton.branch(attachment.parent) else {
                return Profile::Plain;
            };
            let sample = parent.sample(attachment.t);
            Profile::Collar {
                length: collar.length * sample.radius.max(MIN_RADIUS),
                rings: collar.rings,
                flare: collar.flare,
                normal_blend: collar.normal_blend,
                parent_point: sample.position,
                parent_tangent: sample.frame.tangent,
            }
        }
        None => params.root_flare.map_or(Profile::Plain, Profile::Flare),
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

/// Ring stations along `branch` from the layout's start, and each opening's
/// site with the index of the ring below it; `None` for a branch without length.
///
/// Each opening gets a ring at both ends and none between, so the band
/// between those two rings is the opening.
fn stations(
    branch: &Branch,
    profile: &Profile,
    layout: &Layout,
    params: &MeshParams,
    report: &mut MeshReport,
) -> Option<(Vec<Station>, Vec<GapRing>)> {
    let lengths = branch.arc_lengths();
    let total = *lengths.last()?;
    if total <= 1e-6 {
        return None;
    }
    let mut at: Vec<f32> = Vec::new();
    at.push(layout.start);
    // Profile rings near the base, where the radius changes fastest.
    let (extra, extent) = match *profile {
        Profile::Plain => (0, 0.0),
        Profile::Collar { length, rings, .. } => (rings, length),
        Profile::Flare(f) => (f.rings, 3.0 * f.height),
    };
    let extent = extent.min(0.5 * total);
    for k in 1..=extra {
        #[expect(clippy::cast_precision_loss, reason = "ring counts are small")]
        at.push(extent * k as f32 / (extra + 1) as f32);
    }
    let added_before = at.len();
    // Curvature- and spacing-driven rings at skeleton nodes.
    let mut last_s = layout.start;
    let mut last_tangent = branch.nodes[0].frame.tangent;
    for (i, node) in branch.nodes.iter().enumerate().skip(1) {
        let s = lengths[i];
        let is_tip = i + 1 == branch.nodes.len();
        if s <= layout.start && !is_tip {
            continue;
        }
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
    let tolerance = 1e-5 * total.max(1.0);
    for &(_, from, to) in &layout.gaps {
        at.retain(|&s| s <= from || s >= to);
        at.push(from);
        at.push(to);
    }
    at.retain(|&s| s >= layout.start);
    at.sort_by(f32::total_cmp);
    at.dedup_by(|a, b| (*a - *b).abs() < tolerance);
    let gap_rings = layout
        .gaps
        .iter()
        .filter_map(|&(site, from, _)| {
            let ring = at.iter().position(|&s| (s - from).abs() < tolerance)?;
            Some((site, ring))
        })
        .collect();
    Some((
        at.iter()
            .map(|&s| station_at(branch, &lengths, s))
            .collect(),
        gap_rings,
    ))
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

/// Segment count of every ring: the branch's base count, halved each time
/// the ring's radius alone would warrant half as many, while the count stays
/// even and at least `min_segments`. Counts never rise toward the tip and
/// drop at most one level per ring, so every band is quads or a clean
/// two-to-one transition.
fn ring_segments(stations: &[Station], base: u32, params: &MeshParams) -> Vec<u32> {
    let mut counts = Vec::with_capacity(stations.len());
    let mut current = base;
    for station in stations {
        if params.rings.follow_taper {
            let wanted = segment_count(station.radius, params);
            let half = current / 2;
            if current.is_multiple_of(2) && half >= params.rings.min_segments && wanted <= half {
                current = half;
            }
        }
        counts.push(current);
    }
    counts
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
    gap_rings: &[GapRing],
    profile: &Profile,
    counts: &[u32],
    params: &MeshParams,
    corner_data: &mut Vec<[CornerData; 4]>,
    face_sizes: &mut Vec<u8>,
    vertex_branch: &mut Vec<u32>,
    vertex_normals: &mut Vec<Vec3>,
    seams: &mut Vec<(usize, usize)>,
    report: &mut MeshReport,
) -> Result<TubeRings, MeshError> {
    let base = &stations[0];
    let circumference = TAU * base.radius;
    let repeats = libm::roundf(circumference / params.bark.tile_size).max(1.0);
    let metres_per_v = circumference / repeats;
    let v_offset = branch
        .id
        .key(params.bark.seed)
        .with(tag("bark.v_offset"))
        .unit_f32();

    // Positions and normals per ring vertex; `starts[ring]` indexes a ring's
    // first vertex.
    let mut ids: Vec<u32> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut starts: Vec<usize> = Vec::with_capacity(stations.len());
    for (i, station) in stations.iter().enumerate() {
        let prev = i.checked_sub(1).map(|p| &stations[p]);
        let next = stations.get(i + 1);
        let m = counts[i] as usize;
        starts.push(ids.len());
        for j in 0..m {
            let theta = TAU * j as f32 / m as f32;
            let (position, normal) = ring_point(station, next, prev, profile, theta);
            ids.push(builder.push_vertex(position.to_array()));
            normals.push(normal);
            vertex_branch.push(branch_index);
            vertex_normals.push(normal);
        }
    }
    // Corner `j` of `ring`, where `j == m` is the seam's U = repeats side.
    let corner = |ring: usize, j: usize| -> (usize, CornerData) {
        let m = counts[ring] as usize;
        let index = starts[ring] + j % m;
        let uv = [
            repeats * j as f32 / m as f32,
            v_offset + stations[ring].s / metres_per_v,
        ];
        (index, (uv, normals[index].to_array()))
    };
    // First builder face of each band, for naming rings: a band is `m`
    // quads, or three triangles per coarse segment.
    let skipped = |ring: usize| gap_rings.iter().any(|&(_, below)| below == ring);
    let mut band_faces: Vec<usize> = Vec::with_capacity(stations.len());
    let mut next_face = face_sizes.len();
    for ring in 0..stations.len() - 1 {
        band_faces.push(next_face);
        if !skipped(ring) {
            let (m, n) = (counts[ring] as usize, counts[ring + 1] as usize);
            next_face += if m == n { m } else { 3 * n };
        }
    }
    let mut face =
        |corners: &[(usize, CornerData)], seam_edge: Option<usize>| -> Result<(), MeshError> {
            let loop_ids: Vec<u32> = corners.iter().map(|(i, _)| ids[*i]).collect();
            builder.add_face(&loop_ids).map_err(MeshError::Build)?;
            let mut data = [([0.0; 2], [0.0; 3]); 4];
            for (slot, (_, d)) in data.iter_mut().zip(corners) {
                *slot = *d;
            }
            corner_data.push(data);
            face_sizes.push(u8::try_from(corners.len()).expect("faces have at most four corners"));
            if let Some(edge) = seam_edge {
                seams.push((face_sizes.len() - 1, edge));
            }
            Ok(())
        };
    for ring in 0..stations.len() - 1 {
        if skipped(ring) {
            continue;
        }
        let (m, n) = (counts[ring] as usize, counts[ring + 1] as usize);
        if m == n {
            for j in 0..m {
                // The last column closes the ring; loop edge 1 (b -> c) is
                // the seam.
                let seam = (j + 1 == m).then_some(1);
                face(
                    &[
                        corner(ring, j),
                        corner(ring, j + 1),
                        corner(ring + 1, j + 1),
                        corner(ring + 1, j),
                    ],
                    seam,
                )?;
                report.quads += 1;
            }
        } else {
            // Two fine segments below each coarse one: three triangles.
            debug_assert_eq!(m, 2 * n, "rings halve at most once per band");
            report.transition_bands += 1;
            for j in 0..n {
                let (f0, f1, f2) = (2 * j, 2 * j + 1, 2 * j + 2);
                face(
                    &[corner(ring, f0), corner(ring, f1), corner(ring + 1, j)],
                    None,
                )?;
                // Loop edge 1 (fine 2j + 2 -> coarse j + 1) is the seam on
                // the last segment.
                let seam = (j + 1 == n).then_some(1);
                face(
                    &[corner(ring, f1), corner(ring, f2), corner(ring + 1, j + 1)],
                    seam,
                )?;
                face(
                    &[
                        corner(ring, f1),
                        corner(ring + 1, j + 1),
                        corner(ring + 1, j),
                    ],
                    None,
                )?;
                report.transition_triangles += 3;
            }
        }
    }

    // Tip cap: a fan to a point one tip radius beyond the last ring.
    let ring = stations.len() - 1;
    let m = counts[ring] as usize;
    let tip = stations.last().expect("stations are non-empty");
    let tip_position = tip.position + tip.frame.tangent * tip.radius;
    let apex = builder.push_vertex(tip_position.to_array());
    vertex_branch.push(branch_index);
    vertex_normals.push(tip.frame.tangent);
    let apex_uv_v = v_offset + (tip.s + tip.radius) / metres_per_v;
    for j in 0..m {
        let (a, a_data) = corner(ring, j);
        let (b, b_data) = corner(ring, j + 1);
        builder
            .add_face(&[ids[a], ids[b], apex])
            .map_err(MeshError::Build)?;
        let u_mid = repeats * (j as f32 + 0.5) / m as f32;
        corner_data.push([
            a_data,
            b_data,
            ([u_mid, apex_uv_v], tip.frame.tangent.to_array()),
            ([0.0, 0.0], [0.0, 0.0, 0.0]),
        ]);
        face_sizes.push(3);
        report.tip_triangles += 1;
    }
    // Openings leave rings on both sides, so the bands around them exist.
    // Band `r`'s first face starts with the edge from vertex 0 to 1 of ring
    // `r`; its edge from vertex 1 to 0 of ring `r + 1` is loop edge 2 of that
    // quad, or loop edge 1 of the third triangle of a transition band.
    let below = |ring: usize| {
        let band = ring - 1;
        if counts[band] == counts[ring] {
            (band_faces[band], 2)
        } else {
            (band_faces[band] + 2, 1)
        }
    };
    Ok(TubeRings {
        base: (band_faces[0], 0),
        gaps: gap_rings
            .iter()
            .map(|&(site, ring)| {
                (
                    site,
                    below(ring),
                    (band_faces[ring + 1], 0),
                    stations[ring].s,
                )
            })
            .collect(),
        repeats,
        metres_per_v,
        v_offset,
    })
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
