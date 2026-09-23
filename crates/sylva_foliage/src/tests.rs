// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use exedra_mesh::ExtractParams;
use glam::Vec3;
use sylva_skeleton::passes::{FrameParams, PipeModel, compute_frames, pipe_model_radii};
use sylva_skeleton::{Branch, BranchId, Frame, Node, Site, Skeleton};

use crate::{
    FoliageError, FoliageParams, LeafShape, card_mesh, leaf_mask, leaf_mesh, place_leaves,
};

#[test]
fn the_outline_is_a_symmetric_counter_clockwise_polygon() {
    let shape = LeafShape::default();
    let outline = shape.outline();
    let area2: f32 = outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .map(|(a, b)| a.perp_dot(*b))
        .sum();
    assert!(area2 > 0.0, "counter-clockwise from +Z");
    let n = shape.stations as usize;
    for i in 1..n {
        let right = outline[i];
        let left = outline[outline.len() - i];
        assert_eq!((right.x, right.y), (-left.x, left.y), "mirror symmetric");
    }
    // Lobes cut sinuses: the margin is not monotone on the widening side.
    let widths: Vec<f32> = (0..=200)
        .map(|i| shape.half_width(i as f32 / 200.0))
        .collect();
    let dips = widths
        .windows(3)
        .filter(|w| w[1] < w[0] && w[1] < w[2])
        .count();
    assert!(dips >= 4, "lobed margin, {dips} sinuses");
}

#[test]
fn the_mask_covers_the_outline_area() {
    let shape = LeafShape::default();
    let outline = shape.outline();
    let area: f32 = 0.5
        * outline
            .iter()
            .zip(outline.iter().cycle().skip(1))
            .map(|(a, b)| a.perp_dot(*b))
            .sum::<f32>();
    let frame_area = 2.0 * shape.max_half_width() * shape.length;
    let mask = leaf_mask(&shape, 128);
    assert_eq!(mask.coverage.len(), 128 * 128);
    let expected = area / frame_area;
    assert!(
        (mask.coverage_fraction() - expected).abs() < 0.01,
        "{} vs {expected}",
        mask.coverage_fraction()
    );
    assert_eq!(mask, leaf_mask(&shape, 128), "deterministic");
}

#[test]
fn the_blade_mesh_stays_inside_its_own_outline() {
    let shape = LeafShape::default();
    let mesh = leaf_mesh(&shape).expect("mesh");
    let stations = u64::from(shape.stations);
    let triangles: u64 = mesh
        .faces()
        .map(|f| mesh.face_loop(f).count() as u64 - 2)
        .sum();
    assert_eq!(triangles, 4 + 4 * (stations - 2));
    assert!(mesh.validate_fast().is_empty());
    let (tri, _) = mesh.to_trimesh(&ExtractParams::default());
    let half = shape.max_half_width();
    for uv in &tri.uvs {
        assert!((0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]));
        let x = (uv[0] - 0.5) * 2.0 * half;
        assert!(x.abs() <= shape.half_width(uv[1]) + 1e-6, "UV on the blade");
    }
    // Folded halves rise; the tip droops.
    let tip = tri
        .positions
        .iter()
        .max_by(|a, b| a[1].total_cmp(&b[1]))
        .expect("tip");
    assert!(tip[2] < 0.0, "curl droops the tip");
    let card = card_mesh(&shape).expect("card");
    let (quad, _) = card.to_trimesh(&ExtractParams::default());
    let mut corners: Vec<[f32; 2]> = quad.uvs.clone();
    corners.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    assert_eq!(corners, [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0], [1.0, 1.0]]);
}

/// A horizontal twig along +X with leaf sites on alternate sides.
fn twig() -> Skeleton {
    let mut skeleton = Skeleton::new();
    let id = BranchId::root(0);
    skeleton
        .push_branch(Branch {
            id,
            order: 0,
            parent: None,
            nodes: vec![
                Node::at(Vec3::new(0.0, 0.0, 1.0)),
                Node::at(Vec3::new(0.5, 0.0, 1.0)),
                Node::at(Vec3::new(1.0, 0.0, 1.0)),
            ],
        })
        .expect("twig");
    compute_frames(&mut skeleton, &FrameParams::default());
    pipe_model_radii(&mut skeleton, &PipeModel::default()).expect("radii");
    for ordinal in 0..8_u32 {
        let side = if ordinal % 2 == 0 { Vec3::Y } else { -Vec3::Y };
        let frame = Frame::from_tangent(side, Vec3::Z).expect("frame");
        skeleton
            .push_site(Site {
                branch: id,
                ordinal,
                kind: if ordinal == 7 { 1 } else { 0 },
                #[expect(clippy::cast_precision_loss, reason = "small ordinals")]
                t: 0.1 + 0.1 * ordinal as f32,
                frame,
                scale: 1.0,
            })
            .expect("site");
    }
    skeleton
}

#[test]
fn leaves_droop_face_the_sky_and_skip_other_kinds() {
    let skeleton = twig();
    let params = FoliageParams {
        light: 1.0,
        roll_jitter: 0.0,
        droop: 0.3,
        ..FoliageParams::default()
    };
    let foliage = place_leaves(&skeleton, &params).expect("leaves");
    assert_eq!(foliage.report.leaves, 7);
    assert_eq!(foliage.report.other_sites, 1);
    assert_eq!(foliage.templates.len(), params.variants as usize);
    for leaf in &foliage.instances {
        let site = &skeleton.sites()[leaf.site as usize];
        let direction = leaf.rotation * Vec3::Y;
        let upper = leaf.rotation * Vec3::Z;
        let tilt = libm::acosf(direction.dot(site.frame.tangent).clamp(-1.0, 1.0));
        assert!((tilt - 0.3).abs() < 1e-4, "drooped by 0.3: {tilt}");
        assert!(direction.z < 0.0, "droops down");
        assert!(upper.z > 0.9, "faces the sky: {upper:?}");
        assert!((leaf.rotation.length() - 1.0).abs() < 1e-5);
        assert!(leaf.position.distance(Vec3::new(leaf.position.x, 0.0, 1.0)) > 0.0);
        assert!((leaf.canopy_normal.length() - 1.0).abs() < 1e-5);
    }
    assert_eq!(
        foliage.report.instanced_triangles,
        foliage
            .instances
            .iter()
            .map(|l| foliage.templates[l.template as usize].triangles)
            .sum::<u64>()
    );
}

#[test]
fn leaf_choices_are_keyed_per_site() {
    let skeleton = twig();
    let a = place_leaves(&skeleton, &FoliageParams::default()).expect("a");
    let b = place_leaves(
        &skeleton,
        &FoliageParams {
            roll_jitter: 0.1,
            ..FoliageParams::default()
        },
    )
    .expect("b");
    for (x, y) in a.instances.iter().zip(&b.instances) {
        assert_eq!(
            x.template, y.template,
            "rolling does not reshuffle templates"
        );
        assert_eq!(x.position, y.position);
    }
    let again = place_leaves(&skeleton, &FoliageParams::default()).expect("again");
    assert_eq!(a.instances, again.instances, "deterministic");
    let templates: Vec<u32> = a.instances.iter().map(|l| l.template).collect();
    assert!(
        templates.iter().any(|&t| t != templates[0]),
        "variants are used"
    );
}

#[test]
fn invalid_foliage_is_refused() {
    let params = FoliageParams {
        variants: 0,
        ..FoliageParams::default()
    };
    assert_eq!(
        place_leaves(&twig(), &params).err(),
        Some(FoliageError::Params { name: "variants" })
    );
    let shape = LeafShape {
        widest_at: 1.0,
        ..LeafShape::default()
    };
    assert_eq!(
        leaf_mesh(&shape).err(),
        Some(FoliageError::Params {
            name: "shape.widest_at"
        })
    );
}
