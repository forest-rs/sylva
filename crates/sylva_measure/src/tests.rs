// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use sylva_skeleton::glam::Vec3;
use sylva_skeleton::passes::{FrameParams, compute_frames};
use sylva_skeleton::{Attachment, Branch, BranchId, Frame, Node, Site, Skeleton};

use crate::{GrowthCondition, Range, Reference, measure};

/// A 10 m stem tapering from 0.3 m to 0.1 m radius, with four level
/// branches at 6 m, each carrying sites along its outer half.
fn tree() -> Skeleton {
    let trunk = BranchId::root(0);
    let mut skeleton = Skeleton::new();
    let nodes = (0..=10)
        .map(|i| {
            let mut node = Node::at(Vec3::new(0.0, 0.0, i as f32));
            node.radius = 0.3 - 0.02 * i as f32;
            node
        })
        .collect();
    skeleton
        .push_branch(Branch {
            id: trunk,
            order: 0,
            parent: None,
            nodes,
        })
        .expect("trunk");
    for k in 0..4 {
        let angle = core::f32::consts::FRAC_PI_2 * k as f32;
        let out = Vec3::new(libm::cosf(angle), libm::sinf(angle), 0.0);
        let nodes: Vec<Node> = (0..=4)
            .map(|i| {
                let mut node = Node::at(Vec3::new(0.0, 0.0, 6.0) + out * i as f32);
                node.radius = 0.05;
                node
            })
            .collect();
        skeleton
            .push_branch(Branch {
                id: trunk.child(1, k),
                order: 1,
                parent: Some(Attachment {
                    parent: trunk,
                    t: 0.6,
                }),
                nodes,
            })
            .expect("branch");
    }
    compute_frames(&mut skeleton, &FrameParams::default());
    for k in 0..4 {
        for i in 0..10 {
            skeleton
                .push_site(Site {
                    branch: trunk.child(1, k),
                    ordinal: i,
                    kind: 0,
                    t: 0.5 + 0.05 * i as f32,
                    frame: Frame::from_tangent(Vec3::Z, Vec3::X).expect("frame"),
                    scale: 1.0,
                })
                .expect("site");
        }
    }
    skeleton
}

#[test]
fn allometry_reads_the_skeleton() {
    let m = measure(&tree(), 0.1);
    assert_eq!(m.height, 10.0);
    assert_eq!(m.sites, 40);
    // Sites reach 2 m to 3.8 m out on four arms: across two opposite arms
    // the spread is 7.6 m, across a diagonal less.
    assert!(
        m.crown_width > 5.0 && m.crown_width < 7.7,
        "{}",
        m.crown_width
    );
    assert!((m.crown_base - 6.0).abs() < 1e-4);
    // Radius 0.3 - 0.02 z at 1.3 m.
    assert!((m.dbh - 2.0 * (0.3 - 0.026)).abs() < 1e-4, "{}", m.dbh);
    assert!(m.stem_taper < 1.0);
    let &(order, mean, sd) = m.branch_angles.first().expect("order 1");
    assert_eq!(order, 1);
    assert!((mean - 90.0).abs() < 1e-3 && sd < 1e-3, "{mean} {sd}");
    // Level bark arms fill their rows, but the leaf disks between arms
    // leave sky.
    assert!(
        m.sky_fraction > 0.1 && m.sky_fraction < 1.0,
        "{}",
        m.sky_fraction
    );
    assert!(m.silhouette_aspect > 0.0);
    // 10 cm arms on a 55 cm stem are not major limbs.
    assert_eq!(m.major_limbs, 0);
}

#[test]
fn reports_bound_measurements_by_the_reference() {
    let m = measure(&tree(), 0.1);
    let reference = Reference {
        species: "test".into(),
        ranges: vec![
            Range::new("allometry.height", 8.0, 12.0, "test"),
            Range::new("allometry.slenderness", 1.0, 2.0, "test"),
            Range::new("allometry.no_such", 0.0, 1.0, "test"),
        ],
    };
    let report = m.report("test", &reference);
    let failures: Vec<_> = report.failures().collect();
    assert_eq!(failures.len(), 2, "slenderness and the unknown name fail");
    assert!(!report.passed());
    assert!(report.to_json().contains("\"allometry.height\""));
    let targets = reference.targets("allometry.");
    assert_eq!(targets.len(), 3);
    assert_eq!(targets[0].value, 10.0);
    assert_eq!(targets[0].tolerance, 2.0);
    assert_eq!(m.value("allometry.height"), Some(10.0));
}

#[test]
fn references_select_ranges_by_growth_condition() {
    let reference = Reference {
        species: "test".into(),
        ranges: vec![
            Range::new("allometry.slenderness", 10.0, 20.0, "a").grown_in(GrowthCondition::Open),
            Range::new("allometry.slenderness", 40.0, 80.0, "b").grown_in(GrowthCondition::Stand),
            Range::new("allometry.height", 10.0, 30.0, "c"),
        ],
    };
    let open = reference.for_condition(GrowthCondition::Open);
    assert_eq!(open.ranges.len(), 2);
    assert!(open.ranges.iter().all(|r| r.source != "b"));
    let stand = reference.for_condition(GrowthCondition::Stand);
    assert_eq!(stand.ranges.len(), 2);
    assert!(stand.ranges.iter().any(|r| r.source == "b"));
}
