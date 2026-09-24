// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use sylva_skeleton::glam::Vec3;
use sylva_skeleton::{Branch, Skeleton};

use crate::{
    Arrangement, Count, Curve, Envelope, GrowError, Hierarchy, Level, Radii, Shape, Sites, Trunk,
    grow,
};

fn fixture() -> Hierarchy {
    Hierarchy {
        trunk: Trunk {
            length: 8.0,
            length_jitter: 0.1,
            lean: 0.1,
            shape: Shape {
                gnarl: 0.1,
                gnarl_wavelength: 2.0,
                ..Shape::default()
            },
            sites: None,
        },
        levels: vec![
            Level {
                count: Count::Fixed(8),
                span: [0.3, 1.0],
                roll_jitter: 0.2,
                position_jitter: 0.5,
                angle: Curve::linear(1.2, 0.7),
                angle_jitter: 0.1,
                length: Curve::linear(0.6, 0.3),
                length_jitter: 0.2,
                shape: Shape {
                    curve: 0.3,
                    gnarl: 0.3,
                    gnarl_wavelength: 1.0,
                    light: 0.05,
                    ..Shape::default()
                },
                ..Level::default()
            },
            Level {
                count: Count::PerMetre(2.0),
                span: [0.2, 1.0],
                angle: Curve::constant(0.8),
                length: Curve::linear(0.5, 0.3),
                length_jitter: 0.2,
                shape: Shape {
                    sag: 0.2,
                    ..Shape::default()
                },
                ..Level::default()
            },
            Level {
                count: Count::PerMetre(4.0),
                arrangement: Arrangement::Alternate,
                angle: Curve::constant(0.7),
                length: Curve::constant(0.4),
                sites: Some(Sites {
                    per_metre: 6.0,
                    span: [0.3, 1.0],
                    kind: 1,
                    tip_cluster: 4,
                    ..Sites::default()
                }),
                ..Level::default()
            },
        ],
        envelope: None,
        shade: None,
        radii: Radii::default(),
        segment_length: 0.4,
    }
}

fn positions(branch: &Branch) -> Vec<[u32; 3]> {
    branch
        .nodes
        .iter()
        .map(|n| n.position.to_array().map(f32::to_bits))
        .collect()
}

fn up_to_order(skeleton: &Skeleton, order: u32) -> Vec<(u64, Vec<[u32; 3]>)> {
    skeleton
        .branches()
        .iter()
        .filter(|b| b.order <= order)
        .map(|b| (b.id.bits(), positions(b)))
        .collect()
}

#[test]
fn growth_is_deterministic_and_seeded() {
    let h = fixture();
    let a = grow(&h, 3).expect("grow");
    let b = grow(&h, 3).expect("grow");
    assert_eq!(a.skeleton.branches(), b.skeleton.branches());
    assert_eq!(a.skeleton.sites(), b.skeleton.sites());
    assert_eq!(a.report, b.report);
    let c = grow(&h, 4).expect("grow");
    assert_ne!(up_to_order(&a.skeleton, 3), up_to_order(&c.skeleton, 3));
    // IDs follow the generation path, not the seed.
    assert_eq!(
        a.skeleton.branches()[1].id,
        c.skeleton.branches()[1].id,
        "the first level-1 child has the same ID under any seed"
    );
    assert_eq!(a.report.branches_by_level[..2], [1, 8]);
    assert!(a.report.sites > 0);
    assert_eq!(a.report.sites, a.skeleton.sites().len());
    assert_eq!(a.report.pipe.branches, a.skeleton.branches().len());
}

#[test]
fn editing_a_level_leaves_the_levels_above_bit_identical() {
    let h = fixture();
    let before = grow(&h, 11).expect("grow");
    let mut edited = h.clone();
    edited.levels[2].angle = Curve::constant(1.3);
    edited.levels[2].length = Curve::constant(0.2);
    edited.levels[2].shape.gnarl = 0.5;
    edited.levels[2].shape.gnarl_wavelength = 0.3;
    let after = grow(&edited, 11).expect("grow");
    assert_eq!(
        up_to_order(&before.skeleton, 2),
        up_to_order(&after.skeleton, 2),
        "orders 0-2 keep IDs and positions"
    );
    assert_ne!(
        up_to_order(&before.skeleton, 3),
        up_to_order(&after.skeleton, 3),
        "order 3 did change"
    );
    // Same counts, so level-3 IDs are unchanged too.
    let ids = |s: &Skeleton| s.branches().iter().map(|b| b.id).collect::<Vec<_>>();
    assert_eq!(ids(&before.skeleton), ids(&after.skeleton));
}

#[test]
fn opposite_and_whorled_children_share_their_node() {
    let mut h = fixture();
    h.levels.truncate(1);
    h.levels[0].count = Count::Fixed(9);
    h.levels[0].roll_jitter = 0.0;
    h.levels[0].arrangement = Arrangement::Whorled { per_node: 3 };
    let grown = grow(&h, 5).expect("grow");
    let ts: Vec<f32> = grown.skeleton.branches()[1..]
        .iter()
        .map(|b| b.parent.expect("child").t)
        .collect();
    assert_eq!(ts.len(), 9);
    for whorl in ts.chunks(3) {
        assert!(whorl.iter().all(|t| *t == whorl[0]), "whorl {whorl:?}");
    }
    assert!(ts[0] < ts[3] && ts[3] < ts[6]);

    h.levels[0].arrangement = Arrangement::Opposite;
    h.levels[0].count = Count::Fixed(4);
    let grown = grow(&h, 5).expect("grow");
    let children = &grown.skeleton.branches()[1..];
    assert_eq!(
        children[0].parent, children[1].parent,
        "a pair shares its node"
    );
    // Opposite children leave in opposite horizontal directions.
    let out = |b: &Branch| {
        let d = b.nodes[1].position - b.nodes[0].position;
        Vec3::new(d.x, d.y, 0.0).normalize()
    };
    assert!(out(&children[0]).dot(out(&children[1])) < -0.9);
}

#[test]
fn envelope_prunes_and_counts() {
    let mut h = fixture();
    h.envelope = Some(Envelope {
        base: 2.0,
        height: 7.0,
        radius: 2.0,
        profile: Curve::new(vec![[0.0, 0.5], [0.5, 1.0], [1.0, 0.2]]).expect("profile"),
        from_level: 0,
        min_fraction: 0.2,
        lumps: 0.0,
        lump_size: 3.0,
    });
    let grown = grow(&h, 9).expect("grow");
    let envelope = h.envelope.as_ref().expect("envelope");
    assert!(grown.report.truncated > 0);
    for branch in grown.skeleton.branches().iter().filter(|b| b.order > 0) {
        for node in &branch.nodes[1..] {
            let p = node.position;
            let u = (p.z - envelope.base) / envelope.height;
            let d = Vec3::new(p.x, p.y, 0.0).length();
            assert!(
                (0.0..=1.0).contains(&u) && d <= envelope.radius * envelope.profile.eval(u),
                "node {p} outside the envelope"
            );
        }
    }
    let unpruned = grow(&fixture(), 9).expect("grow");
    let total = |g: &crate::Grown| g.report.branches_by_level.iter().sum::<usize>();
    assert!(total(&grown) < total(&unpruned));
}

#[test]
fn shapes_bend_the_way_they_say() {
    let mut h = fixture();
    h.levels.truncate(1);
    h.levels[0].shape = Shape {
        curve: 0.8,
        curve_back: 0.8,
        ..Shape::default()
    };
    h.levels[0].angle = Curve::constant(core::f32::consts::FRAC_PI_2);
    h.levels[0].angle_jitter = 0.0;
    let up = grow(&h, 1).expect("grow");
    h.levels[0].shape.curve = -0.8;
    h.levels[0].shape.curve_back = -0.8;
    let down = grow(&h, 1).expect("grow");
    let rise = |g: &crate::Grown| {
        let b = &g.skeleton.branches()[1];
        b.nodes.last().expect("nodes").position.z - b.nodes[0].position.z
    };
    assert!(rise(&up) > 0.5, "positive curve bends up: {}", rise(&up));
    assert!(
        rise(&down) < -0.5,
        "negative curve bends down: {}",
        rise(&down)
    );
}

#[test]
fn invalid_parameters_are_reported_before_growth() {
    let mut h = fixture();
    h.levels[1].span = [0.8, 0.2];
    assert_eq!(
        grow(&h, 0).err(),
        Some(GrowError::InvalidParameter {
            level: Some(1),
            name: "span"
        })
    );
    let mut h = fixture();
    h.segment_length = 0.0;
    assert_eq!(
        grow(&h, 0).err(),
        Some(GrowError::InvalidParameter {
            level: None,
            name: "segment_length"
        })
    );
}

#[test]
fn kinks_zig_zag_at_every_internode() {
    let hierarchy = Hierarchy {
        trunk: Trunk {
            length: 2.0,
            shape: Shape {
                kink: 0.3,
                kink_interval: 0.25,
                ..Shape::default()
            },
            ..Trunk::default()
        },
        levels: Vec::new(),
        envelope: None,
        shade: None,
        radii: Radii::default(),
        segment_length: 0.5,
    };
    let grown = grow(&hierarchy, 5).expect("grow");
    let nodes = &grown.skeleton.branches()[0].nodes;
    assert_eq!(nodes.len(), 9, "a node at every internode");
    let dirs: Vec<Vec3> = nodes
        .windows(2)
        .map(|w| (w[1].position - w[0].position).normalize())
        .collect();
    // Each kink turns by the full angle, and consecutive turns bend to
    // opposite sides of the same plane.
    let turns: Vec<Vec3> = dirs.windows(2).map(|w| w[0].cross(w[1])).collect();
    for (w, turn) in dirs.windows(2).zip(&turns) {
        let angle = libm::acosf(w[0].dot(w[1]).clamp(-1.0, 1.0));
        assert!((angle - 0.3).abs() < 1e-3, "kink angle {angle}");
        assert!(turn.length() > 0.1);
    }
    for pair in turns.windows(2) {
        assert!(
            pair[0].normalize().dot(pair[1].normalize()) < -0.9,
            "turns alternate sides"
        );
    }
}

#[test]
fn kink_parameters_are_validated() {
    let mut hierarchy = fixture();
    hierarchy.trunk.shape.kink = 0.2;
    assert!(matches!(
        grow(&hierarchy, 1),
        Err(GrowError::InvalidParameter { .. })
    ));
    hierarchy.trunk.shape.kink_interval = 0.3;
    hierarchy.trunk.shape.kink_jitter = 1.5;
    assert!(matches!(
        grow(&hierarchy, 1),
        Err(GrowError::InvalidParameter { .. })
    ));
}

#[test]
fn balance_evens_the_crown_without_changing_structure() {
    let crown = |balance: f32| {
        let mut hierarchy = fixture();
        hierarchy.levels.truncate(1);
        hierarchy.levels[0].length_jitter = 0.5;
        hierarchy.levels[0].balance = balance;
        let grown = grow(&hierarchy, 3).expect("grow");
        let limbs: Vec<Branch> = grown.skeleton.branches()[1..].to_vec();
        let net: Vec3 = limbs
            .iter()
            .map(|b| {
                let d = b.nodes.last().unwrap().position - b.nodes[0].position;
                Vec3::new(d.x, d.y, 0.0)
            })
            .sum();
        (net.length(), limbs.iter().map(|b| b.id).collect::<Vec<_>>())
    };
    let (lopsided, ids) = crown(0.0);
    let (balanced, balanced_ids) = crown(1.0);
    assert_eq!(ids, balanced_ids, "balance changes lengths, not identities");
    assert!(balanced < lopsided * 0.8, "{balanced} vs {lopsided}");
}

#[test]
fn kinks_zig_zag_around_the_trend_without_steering_it() {
    let mut hierarchy = Hierarchy {
        trunk: Trunk {
            length: 6.0,
            shape: Shape {
                kink: 0.5,
                kink_interval: 0.2,
                kink_jitter: 0.8,
                ..Shape::default()
            },
            ..Trunk::default()
        },
        levels: Vec::new(),
        envelope: None,
        shade: None,
        radii: Radii::default(),
        segment_length: 0.5,
    };
    for seed in 0..16 {
        let grown = grow(&hierarchy, seed).expect("grow");
        let nodes = &grown.skeleton.branches()[0].nodes;
        let chord = nodes.last().unwrap().position - nodes[0].position;
        let drift = libm::acosf(chord.normalize().dot(Vec3::Z));
        assert!(
            drift < 0.2,
            "seed {seed}: kinks drifted the trunk by {drift}"
        );
    }
    hierarchy.trunk.shape.kink = 0.0;
    let straight = grow(&hierarchy, 0).expect("straight");
    let nodes = &straight.skeleton.branches()[0].nodes;
    assert!((nodes.last().unwrap().position - Vec3::new(0.0, 0.0, 6.0)).length() < 1e-4);
}

#[test]
fn sites_point_away_from_their_branch_and_crowd_the_tip() {
    let grown = grow(&fixture(), 11).expect("grow");
    let skeleton = &grown.skeleton;
    let sites = Sites::default();
    let mut whorls = 0;
    for site in skeleton.sites() {
        let branch = skeleton.branch(site.branch).expect("branch");
        let axis = branch.sample(site.t).frame.tangent;
        let angle = libm::acosf(site.frame.tangent.dot(axis).clamp(-1.0, 1.0));
        assert!(
            (angle - sites.angle).abs() < 1e-3,
            "outward at the insertion angle: {angle}"
        );
        assert!(site.frame.is_orthonormal(1e-4));
        if site.t > 1.0 - sites.cluster_span {
            whorls += 1;
        }
    }
    let twigs = grown.report.branches_by_level[3] as u64;
    assert!(
        whorls >= 4 * twigs,
        "every twig carries its whorl: {whorls} tip sites on {twigs} twigs"
    );
}

#[test]
fn ranged_counts_vary_by_seed_within_bounds() {
    let mut h = fixture();
    h.levels.truncate(1);
    h.levels[0].count = Count::Range { min: 3, max: 6 };
    let counts: Vec<usize> = (0..24)
        .map(|seed| grow(&h, seed).expect("grow").skeleton.branches().len() - 1)
        .collect();
    assert!(counts.iter().all(|c| (3..=6).contains(c)), "{counts:?}");
    assert!(counts.contains(&3) && counts.contains(&6), "{counts:?}");
    h.levels[0].count = Count::Range { min: 4, max: 2 };
    assert!(grow(&h, 1).is_err());
}

#[test]
fn lumpy_envelopes_stay_within_their_bulge_and_vary_the_trim() {
    let mut h = fixture();
    let smooth = Envelope {
        base: 2.0,
        height: 7.0,
        radius: 2.0,
        profile: Curve::new(vec![[0.0, 0.5], [0.5, 1.0], [1.0, 0.2]]).expect("profile"),
        from_level: 0,
        min_fraction: 0.2,
        lumps: 0.0,
        lump_size: 1.5,
    };
    h.envelope = Some(smooth.clone());
    let plain = grow(&h, 9).expect("grow");
    let lumpy = Envelope {
        lumps: 0.3,
        ..smooth
    };
    h.envelope = Some(lumpy.clone());
    let grown = grow(&h, 9).expect("grow");
    let middle = Vec3::new(0.0, 0.0, lumpy.base + 0.5 * lumpy.height);
    for branch in grown.skeleton.branches().iter().filter(|b| b.order > 0) {
        for node in &branch.nodes[1..] {
            // Shrunk by the largest bulge, every node is inside the smooth
            // envelope.
            let p = middle + (node.position - middle) / (1.0 + lumpy.lumps);
            let u = (p.z - lumpy.base) / lumpy.height;
            let d = Vec3::new(p.x, p.y, 0.0).length();
            assert!(
                (0.0..=1.0).contains(&u) && d <= lumpy.radius * lumpy.profile.eval(u),
                "node {} outside the bulge",
                node.position
            );
        }
    }
    let nodes = |g: &crate::Grown| g.report.nodes;
    assert_ne!(nodes(&plain), nodes(&grown), "lumps change the trim");
}

#[test]
fn trunk_sites_clothe_the_leader_top() {
    let mut h = fixture();
    h.trunk.sites = Some(Sites {
        per_metre: 10.0,
        span: [0.8, 1.0],
        tip_cluster: 4,
        ..Sites::default()
    });
    let grown = grow(&h, 3).expect("grow");
    let trunk = grown.skeleton.branches()[0].id;
    let on_trunk: Vec<_> = grown
        .skeleton
        .sites()
        .iter()
        .filter(|s| s.branch == trunk)
        .collect();
    assert!(on_trunk.len() >= 4, "{} trunk sites", on_trunk.len());
    assert!(on_trunk.iter().all(|s| s.t >= 0.75), "sites on the top");
}

#[test]
fn shade_bares_inner_branches_and_keeps_the_shell() {
    let mut h = fixture();
    let open = grow(&h, 5).expect("grow");
    h.shade = Some(crate::Shade {
        shell: 0.25,
        interior: 0.0,
    });
    let shaded = grow(&h, 5).expect("grow");
    assert!(
        shaded.report.sites < open.report.sites,
        "{} of {} sites",
        shaded.report.sites,
        open.report.sites
    );
    // Sites that survive are the same sites, and none are new.
    let key = |s: &sylva_skeleton::Site| (s.branch, s.ordinal);
    let before: Vec<_> = open.skeleton.sites().iter().map(key).collect();
    assert!(
        shaded
            .skeleton
            .sites()
            .iter()
            .all(|s| before.contains(&key(s)))
    );
    // The branches reaching farthest keep their foliage.
    let far = open
        .skeleton
        .sites()
        .iter()
        .max_by(|a, b| {
            let r = |s: &sylva_skeleton::Site| {
                let p = open
                    .skeleton
                    .branch(s.branch)
                    .expect("branch")
                    .sample(s.t)
                    .position;
                Vec3::new(p.x, p.y, 0.0).length()
            };
            r(a).total_cmp(&r(b))
        })
        .expect("sites");
    assert!(
        shaded
            .skeleton
            .sites()
            .iter()
            .any(|s| s.branch == far.branch)
    );
}
