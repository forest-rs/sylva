// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use exedra_gltf::GlbDocument;
use exedra_mesh::{Mesh, MeshBuilder, op};
use openpbr::Parameters;
use openpbr::color::{LinearSrgb, OpaqueColor};
use sylva_asset::{AssetLod, AssetMesh, AssetReport, MaterialRole, TreeAsset, TreeMaterial};
use sylva_mesh::BRANCH_LAYER;

use crate::{ExportError, MaterialTextures, export_lod_glb};

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

fn asset() -> TreeAsset {
    let mut leaf = Parameters::<LinearSrgb>::DEFAULT;
    leaf.base_color = OpaqueColor::new([0.2, 0.5, 0.1]);
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
        },
        MaterialTextures {
            base_color: Some(PNG),
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
