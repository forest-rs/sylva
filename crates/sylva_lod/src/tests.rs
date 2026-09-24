// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;
use alloc::vec::Vec;

use glam::Vec3;
use sylva_foliage::{Foliage, FoliageParams, place_leaves};
use sylva_mesh::MeshParams;
use sylva_skeleton::passes::{FrameParams, PipeModel, compute_frames, pipe_model_radii};
use sylva_skeleton::{Attachment, Branch, BranchId, Frame, Node, Site, Skeleton};

use crate::{
    AtlasLayout, AtlasSettings, CardMaterials, ClusterCards, Impostor, ImpostorLayout,
    ImpostorPolicy, LeafDetail, LodError, LodPolicy, bake_clusters, bake_impostor, build_lods,
    hemi_octahedral_decode, hemi_octahedral_encode,
};
use exedra_mesh::{ExtractAttribute, ExtractParams, NormalsSource, TriMesh};
use sylva_bake::BakeMaterial;
use sylva_mesh::{BRANCH_LAYER, mesh_skeleton};

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
            bole_taper: 0.0,
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
    .expect("chain")
    .levels;
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
    for lod in chain.iter().filter(|l| l.report.clustered_leaves == 0) {
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
    .expect("chain")
    .levels;
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
    .expect("again")
    .levels;
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
        impostor: None,
    };
    assert_eq!(
        bad.validate(),
        Err(LodError::Params {
            level: 0,
            name: "leaf_fraction"
        })
    );
}

/// A policy whose coarse level clusters the test tree's twigs (order 2).
fn clustered_policy() -> LodPolicy {
    let mut policy = LodPolicy::default();
    policy.levels[2].clusters = Some(ClusterCards {
        root_order: 2,
        variants: 3,
        planes: 2,
        leaf_facing: 0.5,
    });
    policy.levels[3].clusters = Some(ClusterCards {
        root_order: 1,
        variants: 2,
        planes: 1,
        leaf_facing: 0.5,
    });
    policy
}

fn full_bark(skeleton: &Skeleton) -> TriMesh {
    let bark = mesh_skeleton(skeleton, &MeshParams::default()).expect("bark");
    bark.mesh
        .to_trimesh(&ExtractParams {
            normals: NormalsSource::CustomOnly,
            attributes: vec![ExtractAttribute::new(BRANCH_LAYER, u32::MAX)],
            ..ExtractParams::default()
        })
        .0
}

#[test]
fn clusters_replace_every_leaf_of_their_order() {
    let (skeleton, foliage) = tree();
    let chain = build_lods(
        &skeleton,
        &foliage,
        &MeshParams::default(),
        &clustered_policy(),
    )
    .expect("chain");
    let twig_level = &chain.levels[2];
    let clusters = twig_level.clusters.as_ref().expect("clusters");
    // 6 limbs x 4 twigs, every one carrying leaves.
    assert_eq!(clusters.cards.len(), 24);
    assert_eq!(twig_level.report.cluster_cards, 24);
    assert_eq!(twig_level.report.card_triangles, 24 * 2 * 2);
    assert!(twig_level.leaves.is_empty(), "every leaf is on a twig");
    assert_eq!(twig_level.report.clustered_leaves, foliage.report.leaves);
    let members: usize = clusters.members.iter().map(Vec::len).sum();
    assert_eq!(members as u64, foliage.report.leaves);
    assert_eq!(clusters.variants.len(), 3);
    for card in &clusters.cards {
        assert!((card.right.length() - 1.0).abs() < 1e-4);
        assert!(card.right.dot(card.up).abs() < 1e-4);
        assert!(card.half.x > 0.0 && card.half.y > 0.0);
        assert!((card.variant as usize) < clusters.variants.len());
        // The card faces between the crown's outward direction and its
        // leaves' mean upper surface.
        let members = &clusters.members[clusters.cards.iter().position(|c| c == card).unwrap()];
        let centroid = members
            .iter()
            .map(|&l| foliage.instances[l as usize].position)
            .sum::<Vec3>()
            / members.len() as f32;
        let surface: Vec3 = members
            .iter()
            .map(|&l| foliage.instances[l as usize].rotation * Vec3::Z)
            .sum();
        let blend = (centroid - foliage.report.crown_centroid).normalize_or_zero()
            + surface.normalize_or_zero();
        assert!(card.right.cross(card.up).dot(blend) >= -1e-4);
        // Every member leaf lies inside the card's box.
        for &l in members {
            let d = foliage.instances[l as usize].position - card.center;
            assert!(d.dot(card.right).abs() <= card.half.x + 1e-4);
            assert!(d.dot(card.up).abs() <= card.half.y + 1e-4);
        }
    }
    // Twig bark goes with the cards.
    assert!(twig_level.report.pruned_branches >= 24);
    let geometry = clusters.geometry();
    assert_eq!(geometry.indices.len(), 24 * 2 * 6);
    assert!(
        geometry
            .uvs
            .iter()
            .all(|uv| (0.0..=1.0).contains(&uv[0]) && (0.0..=1.0).contains(&uv[1]))
    );
    // The limb level has six cards, one per limb subtree.
    let limb_level = &chain.levels[3];
    assert_eq!(
        limb_level.clusters.as_ref().expect("clusters").cards.len(),
        6
    );
    assert!(limb_level.report.triangles() < twig_level.report.triangles());
}

#[test]
fn atlas_layouts_are_squarest_grids() {
    assert_eq!(
        AtlasLayout::for_cells(1),
        AtlasLayout {
            columns: 1,
            rows: 1
        }
    );
    assert_eq!(
        AtlasLayout::for_cells(3),
        AtlasLayout {
            columns: 2,
            rows: 2
        }
    );
    assert_eq!(
        AtlasLayout::for_cells(8),
        AtlasLayout {
            columns: 3,
            rows: 3
        }
    );
    assert_eq!(
        AtlasLayout::for_cells(4).cell(3),
        [0.5, 0.5, 1.0, 1.0],
        "cells fill rows from v = 0"
    );
}

#[test]
fn cluster_and_impostor_atlases_bake_deterministically() {
    let (skeleton, foliage) = tree();
    let chain = build_lods(
        &skeleton,
        &foliage,
        &MeshParams::default(),
        &clustered_policy(),
    )
    .expect("chain");
    let bark = full_bark(&skeleton);
    let materials = CardMaterials {
        bark: BakeMaterial {
            color: [0.3, 0.2, 0.1],
            ..BakeMaterial::default()
        },
        leaf: BakeMaterial {
            color: [0.2, 0.5, 0.1],
            ..BakeMaterial::default()
        },
    };
    let settings = AtlasSettings {
        cell: [24, 32],
        samples: 2,
    };
    let clusters = chain.levels[2].clusters.as_ref().expect("clusters");
    let atlas =
        bake_clusters(&skeleton, &bark, &foliage, clusters, &materials, &settings).expect("atlas");
    assert_eq!(
        atlas.layout,
        AtlasLayout {
            columns: 2,
            rows: 2
        }
    );
    assert_eq!(atlas.baked.opacity.width(), 48);
    assert_eq!(atlas.baked.opacity.height(), 64);
    assert!(atlas.baked.report.covered_texels > 0);
    // The unused fourth cell stays empty.
    for y in 32..64 {
        for x in 24..48 {
            assert_eq!(atlas.baked.opacity.texel(x, y), [0.0]);
        }
    }
    let again =
        bake_clusters(&skeleton, &bark, &foliage, clusters, &materials, &settings).expect("again");
    assert_eq!(atlas.baked.base_color, again.baked.base_color);

    let impostor = chain.impostor.as_ref().expect("impostor");
    assert_eq!(impostor.views.len(), 3);
    assert_eq!(impostor.geometry().indices.len(), 3 * 6);
    let billboard =
        bake_impostor(&bark, &foliage, impostor, &materials, &settings).expect("impostor atlas");
    let coverage = billboard.baked.opacity.values().iter().sum::<f32>() / (3.0 * 24.0 * 32.0);
    assert!(coverage > 0.05, "every plane sees the tree: {coverage}");

    // An octahedral impostor: 3 x 3 frames over the upper hemisphere.
    let octahedral = Impostor::fit(
        &skeleton,
        &foliage,
        ImpostorPolicy {
            layout: ImpostorLayout::Octahedral { frames: 3 },
            ..ImpostorPolicy::default()
        },
    );
    assert_eq!(octahedral.views.len(), 9);
    assert_eq!(
        octahedral.layout(),
        AtlasLayout {
            columns: 3,
            rows: 3
        }
    );
    // Straight down onto the crown is the grid's centre frame; frames face
    // the direction they were baked from.
    assert_eq!(octahedral.frame_for(Vec3::Z), Some(4));
    for (cell, view) in octahedral.views.iter().enumerate() {
        let cell = u32::try_from(cell).expect("small");
        assert_eq!(octahedral.frame_for(view.toward()), Some(cell));
        assert!(view.toward().z >= -1e-6);
    }
    // Below the horizon clamps to a horizon frame.
    assert_eq!(
        octahedral.frame_for(Vec3::new(0.0, -1.0, -0.5)),
        octahedral.frame_for(Vec3::NEG_Y)
    );
    assert_eq!(octahedral.geometry().indices.len(), 6);
    let frames =
        bake_impostor(&bark, &foliage, &octahedral, &materials, &settings).expect("octahedral");
    for cell in 0..9_u32 {
        let [u0, v0, u1, v1] = octahedral.layout().cell(cell);
        let (w, h) = (frames.baked.opacity.width(), frames.baked.opacity.height());
        let mut covered = 0.0;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "texel bounds of a small atlas"
        )]
        for y in (v0 * h as f32) as u32..(v1 * h as f32) as u32 {
            for x in (u0 * w as f32) as u32..(u1 * w as f32) as u32 {
                covered += frames.baked.opacity.texel(x, y)[0];
            }
        }
        assert!(covered > 0.0, "frame {cell} sees the tree");
    }

    let unbranched = TriMesh {
        attributes: Vec::new(),
        ..bark.clone()
    };
    assert_eq!(
        bake_clusters(
            &skeleton,
            &unbranched,
            &foliage,
            clusters,
            &materials,
            &settings
        )
        .map(|_| ()),
        Err(LodError::Bake("bark must carry the branch layer"))
    );
}

#[test]
fn impostors_must_come_last() {
    let policy = LodPolicy {
        impostor: Some(ImpostorPolicy {
            screen_size: 0.2,
            ..ImpostorPolicy::default()
        }),
        ..LodPolicy::default()
    };
    assert_eq!(
        policy.validate(),
        Err(LodError::Impostor("screen_size order"))
    );
    let planes = LodPolicy {
        impostor: Some(ImpostorPolicy {
            layout: ImpostorLayout::Crossed { planes: 0 },
            ..ImpostorPolicy::default()
        }),
        ..LodPolicy::default()
    };
    assert_eq!(planes.validate(), Err(LodError::Impostor("planes")));
    let frames = LodPolicy {
        impostor: Some(ImpostorPolicy {
            layout: ImpostorLayout::Octahedral { frames: 1 },
            ..ImpostorPolicy::default()
        }),
        ..LodPolicy::default()
    };
    assert_eq!(frames.validate(), Err(LodError::Impostor("frames")));
}

#[test]
fn hemi_octahedral_map_round_trips() {
    for &d in &[
        Vec3::Z,
        Vec3::X,
        Vec3::NEG_Y,
        Vec3::new(1.0, 2.0, 3.0).normalize(),
        Vec3::new(-0.3, 0.8, 0.1).normalize(),
        Vec3::new(0.6, -0.6, 0.0).normalize(),
    ] {
        let uv = hemi_octahedral_encode(d);
        assert!(
            (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y),
            "{uv}"
        );
        let back = hemi_octahedral_decode(uv);
        assert!(back.distance(d) < 1e-5, "{d} -> {uv} -> {back}");
    }
    // Straight up is the square's centre.
    assert!(hemi_octahedral_encode(Vec3::Z).distance(glam::Vec2::splat(0.5)) < 1e-6);
}

#[test]
fn cluster_leaf_facing_is_a_fraction() {
    let mut policy = LodPolicy::default();
    if let Some(clusters) = &mut policy.levels[2].clusters {
        clusters.leaf_facing = 1.5;
    }
    assert!(policy.validate().is_err());
}
