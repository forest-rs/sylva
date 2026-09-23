// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use glam::Vec3;
use sylva_foliage::{Foliage, FoliageParams, place_leaves};
use sylva_mesh::MeshParams;
use sylva_skeleton::passes::{FrameParams, PipeModel, compute_frames, pipe_model_radii};
use sylva_skeleton::{Attachment, Branch, BranchId, Frame, Node, Site, Skeleton};

use crate::{LeafDetail, LodError, LodPolicy, build_lods};

/// A trunk with a fan of side branches, each carrying twigs full of leaf
/// sites.
fn tree() -> (Skeleton, Foliage) {
    let mut skeleton = Skeleton::new();
    let trunk = BranchId::root(0);
    let line = |from: Vec3, to: Vec3, n: usize| -> Vec<Node> {
        (0..=n)
            .map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "small counts")]
                let t = i as f32 / n as f32;
                Node::at(from.lerp(to, t))
            })
            .collect()
    };
    skeleton
        .push_branch(Branch {
            id: trunk,
            order: 0,
            parent: None,
            nodes: line(Vec3::ZERO, Vec3::new(0.0, 0.0, 6.0), 12),
        })
        .expect("trunk");
    let mut twigs = Vec::new();
    for i in 0..6_u64 {
        #[expect(clippy::cast_precision_loss, reason = "small counts")]
        let (a, z) = (i as f32 * 1.05, 3.0 + 0.5 * i as f32);
        let start = Vec3::new(0.0, 0.0, z);
        let dir = Vec3::new(libm::cosf(a), libm::sinf(a), 0.3);
        let limb = trunk.child(1, i);
        skeleton
            .push_branch(Branch {
                id: limb,
                order: 1,
                parent: Some(Attachment {
                    parent: trunk,
                    t: z / 6.0,
                }),
                nodes: line(start, start + dir * 2.5, 6),
            })
            .expect("limb");
        for j in 0..4_u64 {
            #[expect(clippy::cast_precision_loss, reason = "small counts")]
            let t = 0.3 + 0.17 * j as f32;
            let at = start + dir * 2.5 * t;
            let twig = limb.child(2, j);
            skeleton
                .push_branch(Branch {
                    id: twig,
                    order: 2,
                    parent: Some(Attachment { parent: limb, t }),
                    nodes: line(at, at + Vec3::new(0.0, 0.0, 0.5) + dir * 0.2, 3),
                })
                .expect("twig");
            twigs.push(twig);
        }
    }
    compute_frames(&mut skeleton, &FrameParams::default());
    pipe_model_radii(
        &mut skeleton,
        &PipeModel {
            tip_radius: 0.004,
            exponent: 2.3,
            shoots_per_metre: 20.0,
        },
    )
    .expect("radii");
    for twig in twigs {
        for ordinal in 0..20_u32 {
            #[expect(clippy::cast_precision_loss, reason = "small counts")]
            let roll = ordinal as f32 * 2.4;
            let out = Vec3::new(libm::cosf(roll), libm::sinf(roll), 0.2);
            skeleton
                .push_site(Site {
                    branch: twig,
                    ordinal,
                    kind: 0,
                    t: 0.5 + 0.025 * ordinal as f32,
                    frame: Frame::from_tangent(out, Vec3::Z).expect("frame"),
                    scale: 1.0,
                })
                .expect("site");
        }
    }
    let foliage = place_leaves(&skeleton, &FoliageParams::default()).expect("leaves");
    (skeleton, foliage)
}

#[test]
fn levels_shrink_while_leaf_area_holds() {
    let (skeleton, foliage) = tree();
    let chain = build_lods(
        &skeleton,
        &foliage,
        &MeshParams::default(),
        &LodPolicy::default(),
    )
    .expect("chain");
    assert_eq!(chain.len(), 4);
    let full = &chain[0];
    assert_eq!(full.report.leaves, foliage.report.leaves);
    assert_eq!(full.report.pruned_branches, 0);
    for pair in chain.windows(2) {
        assert!(
            pair[1].report.triangles() < pair[0].report.triangles(),
            "each level is cheaper: {:?} then {:?}",
            pair[0].report,
            pair[1].report
        );
        assert!(pair[1].report.branches <= pair[0].report.branches);
    }
    assert!(chain[3].report.pruned_branches > 0, "twigs lose their bark");
    // Leaf area: survivors scale by 1 / sqrt(fraction), so the summed
    // squared scale stays near the full count.
    for lod in &chain {
        let area: f32 = lod.leaves.iter().map(|l| l.scale * l.scale).sum();
        #[expect(clippy::cast_precision_loss, reason = "leaf counts")]
        let full_area = foliage.report.leaves as f32;
        assert!(
            (area / full_area - 1.0).abs() < 0.25,
            "leaf area {area} vs {full_area} at {:?}",
            lod.level.leaf_fraction
        );
        if lod.level.leaf_detail == LeafDetail::Card {
            assert_eq!(lod.report.leaf_triangles, 2 * lod.report.leaves);
        }
    }
}

#[test]
fn leaf_subsets_nest_across_levels() {
    let (skeleton, foliage) = tree();
    let chain = build_lods(
        &skeleton,
        &foliage,
        &MeshParams::default(),
        &LodPolicy::default(),
    )
    .expect("chain");
    for pair in chain.windows(2) {
        let finer: Vec<u32> = pair[0].leaves.iter().map(|l| l.site).collect();
        for leaf in &pair[1].leaves {
            assert!(finer.contains(&leaf.site), "coarser leaves are a subset");
        }
    }
    let again = build_lods(
        &skeleton,
        &foliage,
        &MeshParams::default(),
        &LodPolicy::default(),
    )
    .expect("again");
    for (a, b) in chain.iter().zip(&again) {
        assert_eq!(a.leaves, b.leaves, "deterministic");
        assert_eq!(a.report, b.report);
    }
}

#[test]
fn policies_must_run_finest_first() {
    let (skeleton, foliage) = tree();
    let mut policy = LodPolicy::default();
    policy.levels.swap(1, 2);
    assert!(matches!(
        build_lods(&skeleton, &foliage, &MeshParams::default(), &policy),
        Err(LodError::Params { level: 2, .. })
    ));
    let bad = LodPolicy {
        levels: vec![crate::LodLevel {
            leaf_fraction: 0.0,
            ..LodPolicy::default().levels[0]
        }],
        seed: 0,
    };
    assert_eq!(
        bad.validate(),
        Err(LodError::Params {
            level: 0,
            name: "leaf_fraction"
        })
    );
}
