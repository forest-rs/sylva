// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec::Vec;

use exedra_mesh::{AttributeBuffer, ExtractAttribute, ExtractParams, NormalsSource};
use glam::Vec3;
use sylva_foliage::{Foliage, FoliageParams, place_leaves};
use sylva_lod::{ClusterCards, LodPolicy, build_lods};
use sylva_mesh::{BRANCH_LAYER, MeshParams};
use sylva_skeleton::passes::{FrameParams, PipeModel, compute_frames, pipe_model_radii};
use sylva_skeleton::{Attachment, Branch, BranchId, Frame, Node, Site, Skeleton};

use crate::{LEAVES_PER_MESH, MaterialRole, TreeMaterials, build_asset};

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
        let dir = glam::Vec2::from_angle(a).extend(0.3);
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
            let out = glam::Vec2::from_angle(roll).extend(0.2);
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
fn every_level_becomes_meshes_with_materials_and_provenance() {
    let (skeleton, foliage) = tree();
    let mut policy = LodPolicy::default();
    policy.levels[2].clusters = Some(ClusterCards {
        root_order: 2,
        variants: 2,
        planes: 2,
        leaf_facing: 0.5,
    });
    policy.levels[3].clusters = None;
    let chain = build_lods(&skeleton, &foliage, &MeshParams::default(), &policy).expect("chain");
    let asset = build_asset(&skeleton, &foliage, &chain, &TreeMaterials::default()).expect("asset");
    assert_eq!(
        asset.lods.len(),
        chain.levels.len() + 1,
        "levels then impostor"
    );
    let roles: Vec<MaterialRole> = asset.materials.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        [
            MaterialRole::Bark,
            MaterialRole::Leaf,
            MaterialRole::Cards { level: 2 },
            MaterialRole::Impostor
        ]
    );
    assert!(asset.materials[1].alpha_cutoff.is_some() && asset.materials[1].double_sided);
    assert!(asset.materials[0].alpha_cutoff.is_none());

    let names =
        |level: usize| -> Vec<&str> { asset.lods[level].meshes.iter().map(|m| m.name).collect() };
    let chunks = chain.levels[0].leaves.len().div_ceil(LEAVES_PER_MESH);
    assert!(chunks > 1, "the fixture spans several leaf meshes");
    assert_eq!(names(0)[0], "bark");
    assert_eq!(names(0).len(), 1 + chunks);
    assert!(names(0)[1..].iter().all(|n| *n == "leaves"));
    assert_eq!(names(2), ["bark", "cards"], "clustered leaves become cards");
    assert_eq!(names(4), ["cards"]);
    let leaf_faces: usize = asset.lods[0].meshes[1..]
        .iter()
        .map(|m| m.mesh.faces().count())
        .sum();
    assert_eq!(
        leaf_faces as u64, chain.levels[0].report.leaf_triangles,
        "one triangle per template triangle per leaf"
    );
    let instanced = asset.lods[0].leaves.as_ref().expect("instanced leaves");
    assert_eq!(instanced.instances.len(), chain.levels[0].leaves.len());
    assert!(
        instanced
            .instances
            .iter()
            .all(|l| (l.template as usize) < instanced.templates.len())
    );
    assert!(asset.lods[2].leaves.is_none(), "cards draw no leaves");
    // Instanced cards: one quad per exemplar, one placement per card plane,
    // each landing on the merged card geometry's corners.
    let clusters = chain.levels[2].clusters.as_ref().expect("clusters");
    let cards = asset.lods[2].cards.as_ref().expect("instanced cards");
    assert_eq!(cards.templates.len(), clusters.variants.len());
    assert_eq!(
        cards.instances.len(),
        clusters.cards.len() * clusters.params.planes as usize
    );
    let merged = clusters.geometry();
    for (plane, copy) in cards.instances.iter().enumerate() {
        assert_eq!(
            copy.branch,
            clusters.cards[plane / clusters.params.planes as usize].root
        );
        let corner = copy.transform.transform_point3(Vec3::new(1.0, 1.0, 0.0));
        let expected = Vec3::from_array(merged.positions[4 * plane + 2]);
        assert!(corner.distance(expected) < 1e-4, "{corner} vs {expected}");
    }
    let branches = u32::try_from(skeleton.branches().len()).expect("few branches");
    for lod in &asset.lods {
        for mesh in &lod.meshes {
            assert!(
                mesh.mesh.validate_fast().is_empty(),
                "{} validates",
                mesh.name
            );
            let (tri, stats) = mesh.mesh.to_trimesh(&ExtractParams {
                normals: NormalsSource::CustomOnly,
                attributes: alloc::vec![ExtractAttribute::new(BRANCH_LAYER, u32::MAX)],
                ..ExtractParams::default()
            });
            assert_eq!(stats.missing_attribute_layers, 0);
            let Some(AttributeBuffer::U32(owner)) = tri.attribute(BRANCH_LAYER) else {
                panic!("branch stream");
            };
            assert!(
                owner.iter().all(|&b| b < branches),
                "{} keeps provenance",
                mesh.name
            );
            assert!(
                tri.normals
                    .iter()
                    .all(|n| (Vec3::from_array(*n).length() - 1.0).abs() < 1e-3)
            );
        }
    }
    assert_eq!(asset.report.levels, 5);
}
