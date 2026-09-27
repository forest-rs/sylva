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

use crate::{GeneratedTree, MaterialRole, TreeMaterials, build_asset};

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
    let source = GeneratedTree::new(
        skeleton,
        Some(&FoliageParams::default()),
        TreeMaterials::default(),
    )
    .expect("source");
    let asset = build_asset(&source, &chain, &crate::RasterOptions::default()).expect("asset");
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
    assert_eq!(names(0), ["bark", "leaves"], "one merged leaf mesh");
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
    let branches = u32::try_from(source.skeleton().branches().len()).expect("few branches");
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

#[test]
fn both_realizations_retain_the_same_identified_leaves() {
    let (skeleton, _) = tree();
    let params = FoliageParams {
        whorl: 3,
        shape: sylva_foliage::LeafShape {
            leaflets: 12,
            lobes: 0,
            lobe_depth: 0.0,
            auricle: 0.0,
            ..sylva_foliage::LeafShape::default()
        },
        ..FoliageParams::default()
    };
    let source =
        GeneratedTree::new(skeleton, Some(&params), TreeMaterials::default()).expect("source");
    let mut policy = LodPolicy::default();
    policy.levels.truncate(2);
    policy.impostor = None;
    let chain = build_lods(
        source.skeleton(),
        source.foliage(),
        &MeshParams::default(),
        &policy,
    )
    .expect("chain");
    let raster = build_asset(&source, &chain, &crate::RasterOptions::default()).expect("raster");
    let detailed = crate::build_detailed(&source, &MeshParams::default()).expect("detailed");
    let full = raster.lods[0].leaves.as_ref().expect("leaves");
    assert_eq!(
        full.instances, detailed.leaves.instances,
        "same identity, placement and shading data"
    );
    assert_eq!(
        detailed.report.instances,
        source.foliage().instances.len() as u64
    );
    assert_eq!(detailed.leaves.templates.len(), params.variants as usize);
    assert!(detailed.report.template_triangles < detailed.report.instanced_triangles);
    for leaf in &raster.lods[1]
        .leaves
        .as_ref()
        .expect("reduced leaves")
        .instances
    {
        let original = detailed
            .leaves
            .instances
            .iter()
            .find(|other| other.leaf == leaf.leaf)
            .expect("retained identity");
        assert_eq!(leaf.transform.translation, original.transform.translation);
        assert_eq!(leaf.canopy_normal, original.canopy_normal);
        assert!(
            leaf.transform.matrix3.x_axis.length() > original.transform.matrix3.x_axis.length(),
            "legacy area compensation remains"
        );
    }
    for mesh in &detailed.leaves.templates {
        assert!(mesh.validate_fast().is_empty(), "valid tissue");
    }
}

#[test]
fn source_rejects_duplicate_site_identity() {
    let (mut skeleton, _) = tree();
    let site = skeleton.sites()[0];
    skeleton
        .push_site(site)
        .expect("skeleton permits duplicate sites");
    assert!(matches!(
        GeneratedTree::new(
            skeleton,
            Some(&FoliageParams::default()),
            TreeMaterials::default()
        ),
        Err(crate::AssetError::Source("duplicate site identity"))
    ));
}

#[test]
fn bare_tree_has_no_foliage_templates_or_instances() {
    let (skeleton, _) = tree();
    let source = GeneratedTree::new(skeleton, None, TreeMaterials::default()).expect("bare source");
    let detailed = crate::build_detailed(&source, &MeshParams::default()).expect("bare geometry");
    assert!(detailed.leaves.templates.is_empty());
    assert!(detailed.leaves.instances.is_empty());
    assert_eq!(detailed.report.instanced_triangles, 0);
    assert!(detailed.report.bark_triangles > 0);
}

#[test]
fn merged_normal_policy_preserves_surface_geometry_and_source_shading_data() {
    let (skeleton, _) = tree();
    let source = GeneratedTree::new(
        skeleton,
        Some(&FoliageParams::default()),
        TreeMaterials::default(),
    )
    .expect("source");
    let mut policy = LodPolicy::default();
    policy.levels.truncate(1);
    policy.levels[0].leaf_detail = sylva_lod::LeafDetail::Mesh { stations: 8 };
    policy.impostor = None;
    let chain = build_lods(
        source.skeleton(),
        source.foliage(),
        &MeshParams::default(),
        &policy,
    )
    .expect("chain");
    let canopy = build_asset(&source, &chain, &crate::RasterOptions::default()).expect("canopy");
    let surface = build_asset(
        &source,
        &chain,
        &crate::RasterOptions {
            leaf_normals: crate::LeafNormals::Surface,
            ..crate::RasterOptions::default()
        },
    )
    .expect("surface");
    let extract = |asset: &crate::TreeAsset| {
        asset.lods[0].meshes[1]
            .mesh
            .to_trimesh(&ExtractParams {
                normals: NormalsSource::CustomOnly,
                ..ExtractParams::default()
            })
            .0
    };
    let a = extract(&canopy);
    let b = extract(&surface);
    // Corner normals can split render vertices differently; compare the
    // triangle streams rather than requiring the same render-buffer layout.
    let expanded = |mesh: &exedra_mesh::TriMesh| {
        mesh.indices
            .iter()
            .map(|&i| mesh.positions[i as usize])
            .collect::<Vec<_>>()
    };
    assert_eq!(expanded(&a), expanded(&b));
    assert_ne!(a.normals, b.normals);
    let first = &source.foliage().instances[0];
    assert_eq!(a.normals[0], first.canopy_normal.to_array());
    let template = chain.levels[0].templates[first.template as usize]
        .to_trimesh(&ExtractParams::default())
        .0;
    let position = Vec3::from_array(b.positions[0]);
    let normal = Vec3::from_array(b.normals[0]);
    assert!(
        template
            .positions
            .iter()
            .zip(&template.normals)
            .any(|(p, n)| {
                let expected_position =
                    first.position + first.rotation * (first.scale * Vec3::from_array(*p));
                let expected_normal = first.rotation * Vec3::from_array(*n);
                expected_position.distance(position) < 1e-5
                    && expected_normal.distance(normal) < 1e-5
            }),
        "surface normal follows the corresponding template vertex"
    );
    assert_eq!(
        canopy.lods[0].leaves.as_ref().expect("leaves").instances,
        surface.lods[0].leaves.as_ref().expect("leaves").instances
    );
}
