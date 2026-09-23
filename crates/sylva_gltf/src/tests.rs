// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_gltf::GlbDocument;
use exedra_mesh::{Mesh, MeshBuilder, op};
use openpbr::Parameters;
use openpbr::color::{LinearSrgb, OpaqueColor};
use sylva_asset::{
    AssetLeaf, AssetLeaves, AssetLod, AssetMesh, AssetReport, MaterialRole, TreeAsset, TreeMaterial,
};
use sylva_mesh::BRANCH_LAYER;

use crate::{
    ExportError, ExportOptions, LeafExport, MaterialTextures, export_lod_glb, export_lod_glb_with,
};

/// A 1 x 1 RGBA PNG.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0,
    0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 168, 88, 16, 240, 31, 0, 5,
    100, 2, 104, 146, 252, 212, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];

/// A unit quad in the XZ plane (upright in sylva's Z-up frame), with UVs,
/// normals and a branch index.
fn quad(branch: u32) -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0],
    ] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2, 3]).expect("face");
    let built = builder.build().expect("build");
    let mut mesh = built.mesh;
    mesh.define_dense_layer(BRANCH_LAYER, u32::MAX)
        .expect("layer");
    let face = built.face_ids[0];
    let corners: Vec<_> = mesh.face_loop(face).collect();
    let mut edit = mesh.edit();
    for corner in corners {
        let v = edit.mesh().to_vertex(corner).expect("vertex");
        let p = *edit.mesh().vertex_position(v).expect("position");
        op::set_corner_uv(&mut edit, corner, [p[0], p[2]]).expect("uv");
        op::set_corner_normal_override(&mut edit, corner, Some([0.0, -1.0, 0.0])).expect("n");
        op::set_attribute(&mut edit, BRANCH_LAYER, v, branch).expect("branch");
    }
    let _: () = edit.finish();
    mesh
}

/// A leaf template: a quad with UVs and normals but no branch layer.
fn bare_quad() -> Mesh {
    let mut builder = MeshBuilder::new();
    for p in [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ] {
        builder.push_vertex(p);
    }
    builder.add_face(&[0, 1, 2, 3]).expect("face");
    builder.build().expect("build").mesh
}

fn asset() -> TreeAsset {
    let mut leaf = Parameters::<LinearSrgb>::DEFAULT;
    leaf.base_color = OpaqueColor::new([0.2, 0.5, 0.1]);
    leaf.geometry_thin_walled = true;
    leaf.subsurface_weight = 0.4;
    leaf.subsurface_color = OpaqueColor::new([0.2, 0.36, 0.05]);
    TreeAsset {
        lods: vec![AssetLod {
            screen_size: 0.5,
            crossfade: 0.1,
            meshes: vec![
                AssetMesh {
                    name: "bark",
                    material: 0,
                    mesh: quad(0),
                },
                AssetMesh {
                    name: "leaves",
                    material: 1,
                    mesh: quad(3),
                },
            ],
            leaves: None,
        }],
        materials: vec![
            TreeMaterial {
                name: "bark".into(),
                role: MaterialRole::Bark,
                params: Parameters::DEFAULT,
                alpha_cutoff: None,
                double_sided: false,
            },
            TreeMaterial {
                name: "leaf".into(),
                role: MaterialRole::Leaf,
                params: leaf,
                alpha_cutoff: Some(0.5),
                double_sided: true,
            },
        ],
        report: AssetReport::default(),
    }
}

#[test]
fn levels_export_with_tangents_branches_and_projected_materials() {
    let asset = asset();
    let textures = [
        MaterialTextures {
            base_color: Some(PNG),
            orm: Some(PNG),
            normal: Some(PNG),
            ..MaterialTextures::default()
        },
        MaterialTextures {
            base_color: Some(PNG),
            diffuse_transmission: Some(PNG),
            ..MaterialTextures::default()
        },
    ];
    let glb = export_lod_glb(&asset, 0, &textures).expect("export");
    let doc = GlbDocument::parse(&glb.bytes).expect("parse");
    assert_eq!(doc.triangle_count(), 4);
    let semantics = doc.attribute_semantics();
    for semantic in ["POSITION", "NORMAL", "TEXCOORD_0", "TANGENT", "_BRANCH"] {
        assert!(semantics.contains(&semantic), "{semantic} in {semantics:?}");
    }
    let branches = doc.attribute_components("_BRANCH").expect("branches");
    assert!(branches.iter().all(|&b| b == 0.0 || b == 3.0));
    let json = doc.json();
    let materials = json["materials"].as_array().expect("materials");
    let bark = materials
        .iter()
        .find(|m| m["name"] == "bark")
        .expect("bark");
    assert_eq!(bark["alphaMode"], "OPAQUE");
    assert_eq!(bark["pbrMetallicRoughness"]["roughnessFactor"], 1.0);
    assert!(bark["normalTexture"].is_object());
    assert!(bark["occlusionTexture"].is_object());
    let leaf = materials
        .iter()
        .find(|m| m["name"] == "leaf")
        .expect("leaf");
    assert_eq!(leaf["alphaMode"], "MASK");
    assert_eq!(leaf["doubleSided"], true);
    let factor = leaf["pbrMetallicRoughness"]["baseColorFactor"][1]
        .as_f64()
        .expect("factor");
    assert!((factor - 0.5).abs() < 1e-6, "base colour times weight");
    assert!(
        leaf.get("normalTexture").is_none(),
        "unbound slots stay unbound"
    );
    // The thin-walled leaf's translucency: the texture carries weight and
    // tint, so the factors are 1.
    let transmission = &leaf["extensions"]["KHR_materials_diffuse_transmission"];
    assert_eq!(transmission["diffuseTransmissionFactor"], 1.0);
    assert!(transmission["diffuseTransmissionTexture"].is_object());
    assert!(transmission["diffuseTransmissionColorTexture"].is_object());
    assert!(
        bark.get("extensions").is_none(),
        "opaque bark transmits nothing"
    );
    let used = json["extensionsUsed"].as_array().expect("extensions used");
    assert!(
        used.iter()
            .any(|e| e == "KHR_materials_diffuse_transmission")
    );

    // Without a texture, the factors carry the OpenPBR values.
    let untextured = [MaterialTextures::default(); 2];
    let bare = export_lod_glb(&asset, 0, &untextured).expect("export");
    let bare = GlbDocument::parse(&bare.bytes)
        .expect("parse")
        .json()
        .clone();
    let transmission = &bare["materials"][1]["extensions"]["KHR_materials_diffuse_transmission"];
    let factor = transmission["diffuseTransmissionFactor"]
        .as_f64()
        .expect("factor");
    assert!((factor - 0.4).abs() < 1e-6);
    assert!(transmission.get("diffuseTransmissionTexture").is_none());
    // One shared image serves every slot that uses it.
    assert_eq!(glb.stats.images, 1);
}

#[test]
fn exports_check_their_inputs() {
    let asset = asset();
    assert!(matches!(
        export_lod_glb(&asset, 3, &[MaterialTextures::default(); 2]),
        Err(ExportError::NoLevel(3))
    ));
    assert!(matches!(
        export_lod_glb(&asset, 0, &[MaterialTextures::default()]),
        Err(ExportError::Textures {
            expected: 2,
            found: 1
        })
    ));
}

#[test]
fn instanced_leaves_draw_each_template_once() {
    let mut asset = asset();
    let leaf = |x: f32| AssetLeaf {
        template: 0,
        position: glam::Vec3::new(x, 0.0, 2.0),
        rotation: glam::Quat::from_rotation_z(x),
        scale: 0.5,
        branch: 3,
    };
    asset.lods[0].leaves = Some(AssetLeaves {
        templates: vec![bare_quad()],
        instances: vec![leaf(0.0), leaf(1.0), leaf(2.0)],
    });
    let textures = [MaterialTextures::default(); 2];
    let options = ExportOptions::default().with_leaves(LeafExport::Instanced);
    // The templates have no branch layer; they export `NO_BRANCH`, which
    // the float encoding represents.
    let glb = export_lod_glb_with(&asset, 0, &textures, options).expect("export");
    let json = GlbDocument::parse(&glb.bytes)
        .expect("parse")
        .json()
        .clone();
    let required = json["extensionsRequired"].as_array().expect("required");
    assert!(required.iter().any(|e| e == "EXT_mesh_gpu_instancing"));
    let nodes = json["nodes"].as_array().expect("nodes");
    let batched: Vec<_> = nodes
        .iter()
        .filter_map(|n| n["extensions"].get("EXT_mesh_gpu_instancing"))
        .collect();
    assert_eq!(batched.len(), 1, "one batch for the one template");
    let translation = batched[0]["attributes"]["TRANSLATION"]
        .as_u64()
        .expect("translation accessor");
    assert_eq!(
        json["accessors"][usize::try_from(translation).expect("index")]["count"],
        3
    );

    // The default still merges.
    let merged = export_lod_glb(&asset, 0, &textures).expect("export");
    let merged = GlbDocument::parse(&merged.bytes)
        .expect("parse")
        .json()
        .clone();
    assert!(merged.get("extensionsRequired").is_none());
}
