// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;
use core::f32::consts::TAU;

use exedra_mesh::{AttributeBuffer, ExtractAttribute, ExtractParams, NormalsSource, TriMesh};
use glam::Vec3;
use sylva_skeleton::passes::{FrameParams, PipeModel, compute_frames, pipe_model_radii};
use sylva_skeleton::{Attachment, Branch, BranchId, Node, Skeleton};

use crate::{
    BRANCH_LAYER, Collar, Junction, MeshError, MeshParams, RingResolution, RootFlare, Stations,
    Weld, mesh_skeleton,
};

/// A vertical four-metre trunk with nodes every `step` metres, and
/// optionally one child along +X from its middle.
fn tree(with_child: bool, step: f32) -> Skeleton {
    let trunk = BranchId::root(0);
    let mut skeleton = Skeleton::new();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "test lengths are small"
    )]
    let n = (4.0 / step) as usize;
    #[expect(clippy::cast_precision_loss, reason = "test node counts are small")]
    let nodes = (0..=n)
        .map(|i| Node::at(Vec3::new(0.0, 0.0, step * i as f32)))
        .collect();
    skeleton
        .push_branch(Branch {
            id: trunk,
            order: 0,
            parent: None,
            nodes,
        })
        .expect("trunk");
    if with_child {
        skeleton
            .push_branch(Branch {
                id: trunk.child(1, 0),
                order: 1,
                parent: Some(Attachment {
                    parent: trunk,
                    t: 0.5,
                }),
                nodes: vec![
                    Node::at(Vec3::new(0.0, 0.0, 2.0)),
                    Node::at(Vec3::new(1.0, 0.0, 2.0)),
                    Node::at(Vec3::new(2.0, 0.0, 2.0)),
                ],
            })
            .expect("child");
    }
    compute_frames(&mut skeleton, &FrameParams::default());
    pipe_model_radii(
        &mut skeleton,
        &PipeModel {
            tip_radius: 0.05,
            exponent: 2.0,
            ..PipeModel::default()
        },
    )
    .expect("radii");
    skeleton
}

fn plain() -> MeshParams {
    MeshParams {
        root_flare: None,
        ..MeshParams::default()
    }
}

fn extract(mesh: &exedra_mesh::Mesh) -> TriMesh {
    mesh.to_trimesh(&ExtractParams {
        normals: NormalsSource::CustomOnly,
        attributes: vec![ExtractAttribute::new(BRANCH_LAYER, u32::MAX)],
        ..ExtractParams::default()
    })
    .0
}

#[test]
fn a_single_stem_is_a_capped_tube() {
    let skeleton = tree(false, 0.5);
    let bark = mesh_skeleton(&skeleton, &plain()).expect("mesh");
    let r = bark.report;
    let m = u64::from(r.min_segments);
    assert_eq!((r.branches, r.skipped_branches), (1, 0));
    assert_eq!(r.min_segments, r.max_segments);
    assert_eq!(r.quads, (r.rings - 1) * m);
    assert_eq!(r.tip_triangles, m);
    assert_eq!(r.vertices, r.rings * m + 1);
    assert!(bark.mesh.validate_fast().is_empty(), "valid topology");
    let tri = extract(&bark.mesh);
    assert_eq!(tri.indices.len() as u64, 3 * r.triangles());
    for n in &tri.normals {
        let len = Vec3::from_array(*n).length();
        assert!((len - 1.0).abs() < 1e-4, "unit normals, got {len}");
    }
}

#[test]
fn straight_stems_place_rings_by_spacing_and_bends_add_rings() {
    let params = MeshParams {
        stations: Stations {
            max_bend: 0.1,
            max_spacing: 1.0,
        },
        ..plain()
    };
    // Nine nodes over four metres: a ring every metre plus the base.
    let straight = mesh_skeleton(&tree(false, 0.5), &params).expect("straight");
    assert_eq!(straight.report.rings, 5);

    let mut bent = Skeleton::new();
    #[expect(clippy::cast_precision_loss, reason = "test node counts are small")]
    let nodes = (0..=8)
        .map(|i| {
            let a = 0.3 * i as f32;
            Node::at(Vec3::new(libm::sinf(a), 0.0, 1.0 - libm::cosf(a)))
        })
        .collect();
    bent.push_branch(Branch {
        id: BranchId::root(0),
        order: 0,
        parent: None,
        nodes,
    })
    .expect("bent");
    compute_frames(&mut bent, &FrameParams::default());
    pipe_model_radii(&mut bent, &PipeModel::default()).expect("radii");
    let curved = mesh_skeleton(&bent, &params).expect("curved");
    assert_eq!(curved.report.rings, 9, "every turning node keeps a ring");
}

#[test]
fn bark_uvs_wrap_an_integer_number_of_times_with_square_texels() {
    let skeleton = tree(false, 0.5);
    let params = plain();
    let bark = mesh_skeleton(&skeleton, &params).expect("mesh");
    let tri = extract(&bark.mesh);
    let radius = skeleton.branches()[0].nodes[0].radius;
    let circumference = TAU * radius;
    let repeats = libm::roundf(circumference / params.bark.tile_size).max(1.0);
    let u_max = tri.uvs.iter().map(|uv| uv[0]).fold(0.0, f32::max);
    assert!(
        (u_max - repeats).abs() < 1e-5,
        "U spans the repeats: {u_max}"
    );
    // U per metre around the base equals V per metre along it.
    let per_metre_u = repeats / circumference;
    let v_at = |z: f32| {
        tri.positions
            .iter()
            .zip(&tri.uvs)
            .find(|(p, _)| (p[2] - z).abs() < 1e-5)
            .map(|(_, uv)| uv[1])
            .expect("ring vertex")
    };
    let per_metre_v = v_at(1.0) - v_at(0.0);
    assert!(
        (per_metre_u - per_metre_v).abs() < 1e-3 * per_metre_v,
        "square texels at the base: {per_metre_u} vs {per_metre_v}"
    );
    let seam_edges = bark
        .mesh
        .half_edges()
        .filter(|&e| bark.mesh.edge_seam(e) == Some(true))
        .count();
    assert!(seam_edges > 0, "seam edges are tagged");
    // Each ring's seam vertex appears at both U = 0 and U = repeats.
    let at_zero = tri.uvs.iter().filter(|uv| uv[0] == 0.0).count();
    let at_end = tri.uvs.iter().filter(|uv| uv[0] == repeats).count();
    assert_eq!(at_zero, at_end);
    assert!(at_zero > 0);
}

#[test]
fn rings_follow_the_transported_frame_without_twist() {
    let skeleton = tree(false, 0.5);
    let bark = mesh_skeleton(&skeleton, &plain()).expect("mesh");
    let tri = extract(&bark.mesh);
    let normal = skeleton.branches()[0].nodes[0].frame.normal;
    for (p, uv) in tri.positions.iter().zip(&tri.uvs) {
        if uv[0] == 0.0 {
            let radial = Vec3::new(p[0], p[1], 0.0).normalize();
            assert!(radial.dot(normal) > 0.999, "no twist: {radial:?}");
        }
    }
}

#[test]
fn embedded_children_flare_a_collar_blend_normals_and_record_provenance() {
    let skeleton = tree(true, 0.5);
    let collar = Collar::default();
    let params = MeshParams {
        junction: Junction::Embedded(collar),
        ..plain()
    };
    let bark = mesh_skeleton(&skeleton, &params).expect("mesh");
    assert_eq!(bark.report.branches, 2);
    assert_eq!(bark.report.collar_rings, u64::from(collar.rings));
    let tri = extract(&bark.mesh);
    let Some(AttributeBuffer::U32(branches)) = tri.attribute(BRANCH_LAYER) else {
        panic!("branch stream");
    };
    assert!(
        branches.iter().all(|&b| b <= 1),
        "every vertex names its branch"
    );
    let child_radius = skeleton.branches()[1].nodes[0].radius;
    let mut base_ring = 0;
    let mut diagonal = 0;
    for ((p, n), &b) in tri.positions.iter().zip(&tri.normals).zip(branches) {
        if b != 1 || p[0] != 0.0 {
            continue;
        }
        base_ring += 1;
        let p = Vec3::from_array(*p);
        let n = Vec3::from_array(*n);
        let r = (p - Vec3::new(0.0, 0.0, 2.0)).length();
        assert!(
            (r - child_radius * collar.flare).abs() < 1e-4,
            "the collar flares the base: {r}"
        );
        // The trunk's surface normal here is horizontal; the child's own
        // radial is in the YZ plane. Blending tilts toward the trunk's.
        let child_radial = Vec3::new(0.0, p.y, p.z - 2.0).normalize();
        if p.y.abs() > 0.3 * r && (p.z - 2.0).abs() > 0.3 * r {
            diagonal += 1;
            let trunk_normal = Vec3::new(0.0, p.y, 0.0).normalize();
            assert!(
                n.dot(trunk_normal) > child_radial.dot(trunk_normal) + 1e-3,
                "collar normal leans to the parent: p {p:?} n {n:?} radial {child_radial:?}"
            );
        }
    }
    assert!(base_ring > 0, "child base ring found");
    assert!(diagonal > 0, "diagonal collar vertices checked");
}

#[test]
fn collars_stay_inside_a_parent_barely_thicker_than_the_child() {
    let mut skeleton = tree(true, 0.5);
    // With e = 3 the trunk is only 2^(1/3) times the child, so a full
    // collar flare would push the child's base ring out through the trunk.
    pipe_model_radii(
        &mut skeleton,
        &PipeModel {
            tip_radius: 0.05,
            exponent: 3.0,
            ..PipeModel::default()
        },
    )
    .expect("radii");
    let collar = Collar::default();
    let params = MeshParams {
        junction: Junction::Embedded(collar),
        ..plain()
    };
    let child = skeleton.branches()[1].nodes[0].radius;
    let parent = skeleton.branches()[0].sample(0.5).radius;
    assert!(child * collar.flare > parent, "the case needs capping");
    let tri = extract(&mesh_skeleton(&skeleton, &params).expect("mesh").mesh);
    let Some(AttributeBuffer::U32(branches)) = tri.attribute(BRANCH_LAYER) else {
        panic!("branch stream");
    };
    let mut base_ring = 0;
    for (p, &b) in tri.positions.iter().zip(branches) {
        if b == 1 && p[0] == 0.0 {
            base_ring += 1;
            let r = (Vec3::from_array(*p) - Vec3::new(0.0, 0.0, 2.0)).length();
            assert!(r <= 0.97 * parent + 1e-5, "{r} vs parent {parent}");
            assert!(r >= child - 1e-5, "the cap never thins the child: {r}");
        }
    }
    assert!(base_ring > 0, "child base ring found");
}

#[test]
fn root_flare_widens_the_base_in_lobes() {
    let skeleton = tree(false, 0.5);
    let flare = RootFlare::default();
    let params = MeshParams {
        root_flare: Some(flare),
        rings: RingResolution {
            min_segments: 20,
            max_segments: 20,
            segments_per_metre: 0.0,
            follow_taper: false,
        },
        ..MeshParams::default()
    };
    let bark = mesh_skeleton(&skeleton, &params).expect("mesh");
    assert_eq!(bark.report.flare_rings, u64::from(flare.rings));
    let tri = extract(&bark.mesh);
    let radius = skeleton.branches()[0].nodes[0].radius;
    let base: Vec<f32> = tri
        .positions
        .iter()
        .filter(|p| p[2] == 0.0)
        .map(|p| libm::sqrtf(p[0] * p[0] + p[1] * p[1]))
        .collect();
    let widest = base.iter().copied().fold(0.0, f32::max);
    let narrowest = base.iter().copied().fold(f32::MAX, f32::min);
    // Ring vertices need not land on a ridge crest, and valleys keep
    // `1 - lobe_depth` of the flare.
    assert!(widest <= radius * (1.0 + flare.flare) + 1e-4, "{widest}");
    assert!(
        widest > radius * (1.0 + flare.flare * (1.0 - 0.5 * flare.lobe_depth)),
        "{widest}"
    );
    assert!(
        narrowest >= radius * (1.0 + flare.flare * (1.0 - flare.lobe_depth)) - 1e-4,
        "{narrowest}"
    );
    assert!(narrowest < widest * 0.95, "lobes carve the flare");
}

#[test]
fn meshing_is_deterministic() {
    let skeleton = tree(true, 0.25);
    let a = extract(
        &mesh_skeleton(&skeleton, &MeshParams::default())
            .expect("a")
            .mesh,
    );
    let b = extract(
        &mesh_skeleton(&skeleton, &MeshParams::default())
            .expect("b")
            .mesh,
    );
    assert_eq!(a, b);
}

#[test]
fn invalid_input_is_refused() {
    let skeleton = tree(false, 0.5);
    let params = MeshParams {
        rings: RingResolution {
            min_segments: 2,
            ..RingResolution::default()
        },
        ..MeshParams::default()
    };
    assert_eq!(
        mesh_skeleton(&skeleton, &params).err(),
        Some(MeshError::Params {
            name: "rings.min_segments"
        })
    );
    let mut no_radii = Skeleton::new();
    no_radii
        .push_branch(Branch {
            id: BranchId::root(0),
            order: 0,
            parent: None,
            nodes: vec![Node::at(Vec3::ZERO), Node::at(Vec3::Z)],
        })
        .expect("stem");
    assert!(matches!(
        mesh_skeleton(&no_radii, &MeshParams::default()),
        Err(MeshError::Skeleton(_))
    ));
}

#[test]
fn segment_counts_follow_the_taper_without_cracks() {
    // A long stem tapering from a thick base to a thin tip.
    let mut skeleton = Skeleton::new();
    #[expect(clippy::cast_precision_loss, reason = "test node counts are small")]
    let nodes = (0..=20)
        .map(|i| Node::at(Vec3::new(0.0, 0.0, 0.5 * i as f32)))
        .collect();
    skeleton
        .push_branch(Branch {
            id: BranchId::root(0),
            order: 0,
            parent: None,
            nodes,
        })
        .expect("stem");
    compute_frames(&mut skeleton, &FrameParams::default());
    pipe_model_radii(
        &mut skeleton,
        &PipeModel {
            tip_radius: 0.01,
            exponent: 2.0,
            shoots_per_metre: 40.0,
            bole_taper: 0.0,
        },
    )
    .expect("radii");
    let constant = MeshParams {
        rings: RingResolution {
            follow_taper: false,
            ..RingResolution::default()
        },
        ..plain()
    };
    let flat = mesh_skeleton(&skeleton, &constant).expect("constant");
    let tapered = mesh_skeleton(&skeleton, &plain()).expect("tapered");
    let r = tapered.report;
    assert!(r.transition_bands >= 2, "several halvings: {r:?}");
    assert!(r.min_segments < r.max_segments);
    assert!(r.min_segments >= RingResolution::default().min_segments);
    assert!(r.triangles() < flat.report.triangles(), "fewer triangles");
    assert!(
        tapered.mesh.validate_fast().is_empty(),
        "valid, crack-free topology"
    );
    let tri = extract(&tapered.mesh);
    assert_eq!(tri.indices.len() as u64, 3 * r.triangles());
    // Every transition vertex sits on a ring, and the seam still spans the
    // full U range on every ring.
    let repeats = tri.uvs.iter().map(|uv| uv[0]).fold(0.0, f32::max);
    let rings_at_seam = tri.uvs.iter().filter(|uv| uv[0] == repeats).count();
    let rings_at_zero = tri.uvs.iter().filter(|uv| uv[0] == 0.0).count();
    assert_eq!(rings_at_seam, rings_at_zero, "every ring closes its seam");
}

/// A trunk forking at its middle into a child leaning `angle` radians off
/// the trunk, toward +X.
fn fork(angle: f32) -> Skeleton {
    let trunk = BranchId::root(0);
    let mut skeleton = Skeleton::new();
    #[expect(clippy::cast_precision_loss, reason = "test node counts are small")]
    let nodes = (0..=16)
        .map(|i| Node::at(Vec3::new(0.0, 0.0, 0.25 * i as f32)))
        .collect();
    skeleton
        .push_branch(Branch {
            id: trunk,
            order: 0,
            parent: None,
            nodes,
        })
        .expect("trunk");
    let direction = Vec3::new(libm::sinf(angle), 0.0, libm::cosf(angle));
    #[expect(clippy::cast_precision_loss, reason = "test node counts are small")]
    let nodes = (0..=8)
        .map(|i| Node::at(Vec3::new(0.0, 0.0, 2.0) + direction * (0.25 * i as f32)))
        .collect();
    skeleton
        .push_branch(Branch {
            id: trunk.child(1, 0),
            order: 1,
            parent: Some(Attachment {
                parent: trunk,
                t: 0.5,
            }),
            nodes,
        })
        .expect("child");
    compute_frames(&mut skeleton, &FrameParams::default());
    pipe_model_radii(
        &mut skeleton,
        &PipeModel {
            tip_radius: 0.04,
            exponent: 2.0,
            ..PipeModel::default()
        },
    )
    .expect("radii");
    skeleton
}

fn welded(weld: Weld) -> MeshParams {
    MeshParams {
        junction: Junction::Welded(weld),
        ..plain()
    }
}

#[test]
fn welded_forks_close_the_parent_opening_with_an_authored_skin() {
    let skeleton = fork(1.0);
    let bark = mesh_skeleton(
        &skeleton,
        &welded(Weld {
            min_ratio: 0.1,
            ..Weld::default()
        }),
    )
    .expect("mesh");
    let r = bark.report;
    assert_eq!(bark.weld_refusals, vec![]);
    assert_eq!(bark.welds, vec![1]);
    assert_eq!((r.welded_junctions, r.weld_fallbacks, r.builds), (1, 0, 1));
    assert!(r.skin_quads > 0);
    assert!(bark.mesh.validate_fast().is_empty(), "valid topology");
    // Only the trunk's base stays open: the child and both trunk pieces are
    // joined by the skin.
    let loops = bark.mesh.boundary_loops().expect("boundary loops");
    assert_eq!(loops.len(), 1, "only the ground ring is open");
    let tri = extract(&bark.mesh);
    assert_eq!(tri.indices.len() as u64, 3 * r.triangles());
    for n in &tri.normals {
        let len = Vec3::from_array(*n).length();
        assert!((len - 1.0).abs() < 1e-4, "every corner is authored: {len}");
    }
    let Some(AttributeBuffer::U32(branches)) = tri.attribute(BRANCH_LAYER) else {
        panic!("branch provenance is extracted");
    };
    assert!(branches.iter().all(|&b| b < 2), "every vertex has a branch");
    // The skin continues the trunk's chart across the ring below the fork:
    // corners meeting there agree exactly, or differ by whole tiles where the
    // skin's face straddles the chart's U seam.
    let below = 2.0 - Weld::default().parent_reach * skeleton.branches()[0].sample(0.5).radius;
    let mesh = &bark.mesh;
    let at_ring = |v: Option<exedra_mesh::VertexId>| {
        v.and_then(|v| mesh.vertex_position(v))
            .is_some_and(|p| (p[2] - below).abs() < 1e-4)
    };
    let ring: Vec<_> = mesh
        .half_edges()
        .filter(|&e| at_ring(mesh.from_vertex(e)) && at_ring(mesh.to_vertex(e)))
        .collect();
    assert!(!ring.is_empty(), "the ring below the fork is found");
    let layer = mesh
        .attrs()
        .sparse(exedra_mesh::attr::CORNER_UV)
        .expect("uvs");
    let uv = |corner: exedra_mesh::HalfEdgeId| *layer.get(corner.as_id()).expect("corner uv");
    let mut whole_tile_steps = 0;
    for &edge in &ring {
        let twin = mesh.twin(edge).expect("twin");
        for (a, b) in [
            (edge, mesh.prev(twin).expect("prev")),
            (twin, mesh.prev(edge).expect("prev")),
        ] {
            let (a, b) = (uv(a), uv(b));
            assert_eq!(a[1], b[1], "V agrees exactly");
            let du = a[0] - b[0];
            assert_eq!(du, libm::roundf(du), "U agrees up to whole tiles");
            whole_tile_steps += usize::from(du != 0.0);
        }
    }
    assert!(whole_tile_steps <= 4, "only the seam-straddling face steps");
}

#[test]
fn refused_welds_fall_back_to_the_embedded_collar() {
    // A child hugging its parent cannot be cleared at short reaches.
    let skeleton = fork(0.15);
    let weld = Weld {
        min_ratio: 0.1,
        parent_reach: 1.0,
        child_reach: 1.0,
        ..Weld::default()
    };
    let bark = mesh_skeleton(&skeleton, &welded(weld)).expect("mesh");
    let r = bark.report;
    assert_eq!((r.welded_junctions, r.weld_fallbacks, r.builds), (0, 1, 2));
    assert_eq!(bark.weld_refusals.len(), 1);
    assert_eq!(bark.weld_refusals[0].branch, 1);
    assert_eq!(r.skin_quads + r.skin_triangles, 0);
    let embedded = mesh_skeleton(
        &skeleton,
        &MeshParams {
            junction: Junction::Embedded(weld.collar),
            ..plain()
        },
    )
    .expect("embedded");
    assert_eq!(r.triangles(), embedded.report.triangles());
    assert_eq!(bark.mesh.boundary_loops().expect("loops").len(), 2);
}

#[test]
fn minor_children_stay_embedded_under_welding() {
    let skeleton = fork(1.0);
    let bark = mesh_skeleton(
        &skeleton,
        &welded(Weld {
            min_ratio: 10.0,
            ..Weld::default()
        }),
    )
    .expect("mesh");
    assert_eq!(bark.report.welded_junctions + bark.report.weld_fallbacks, 0);
    assert!(bark.report.collar_rings > 0);
}
