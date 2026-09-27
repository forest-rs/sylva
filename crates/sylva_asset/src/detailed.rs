// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Detailed geometry compiled directly from vegetation descriptions.

use alloc::vec::Vec;
use exedra_mesh::Mesh;
use glam::{Affine3A, Vec3};
use sylva_foliage::tissue_mesh;
use sylva_mesh::{MeshParams, mesh_skeleton};

use crate::{AssetError, AssetInstance, AssetInstances, GeneratedTree, MaterialRole, TreeMaterial};
use sylva_skeleton::BranchId;

/// A detailed tree with one bark mesh and reusable tissue templates.
///
/// No whole-tree LOD or merged leaf mesh is constructed. Leaves use two-sided
/// surface geometry with no silhouette alpha test. Keep the authored material's
/// thin-walled transmission; geometric coverage does not make the material
/// optically opaque. Material coordinates match the source shapes and bark UVs.
///
/// Template normals describe physical surface orientation. Every placement
/// separately retains its artistic canopy normal; a renderer may explicitly
/// choose that shading treatment without overwriting the surface information.
#[derive(Clone, Debug)]
pub struct DetailedAsset {
    /// Stable IDs indexed by compiled bark and instance branch provenance.
    pub branches: Vec<BranchId>,
    /// Bark with UVs, authored normals and branch-index provenance.
    pub bark: Mesh,
    /// Geometric tissue templates and identified placements.
    pub leaves: AssetInstances,
    /// Bark then leaf material; neither uses an alpha test.
    pub materials: [TreeMaterial; 2],
    /// Deterministic geometry and reuse counts, without expanding instances.
    pub report: DetailedReport,
}

/// Work and storage counts for a detailed realization.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DetailedReport {
    /// Bark triangles.
    pub bark_triangles: u64,
    /// Unique tissue templates.
    pub templates: u64,
    /// Triangles stored across the tissue templates.
    pub template_triangles: u64,
    /// Leaf placements.
    pub instances: u64,
    /// Leaf triangles if all placements were expanded; no expansion is done.
    pub instanced_triangles: u64,
}

/// Compiles detailed bark and instanced tissue directly from `tree`.
///
/// Each source leaf maps to exactly one placement with the same identity,
/// transform, template index and canopy normal. Tissue is meshed once per
/// template, regardless of the number of placements. No LOD chain is needed.
///
/// # Errors
///
/// [`AssetError::Foliage`] or [`AssetError::Mesh`] for realization failures,
/// or [`AssetError::TooLarge`] when a branch index cannot fit in `u32`.
pub fn build_detailed(
    tree: &GeneratedTree,
    bark: &MeshParams,
) -> Result<DetailedAsset, AssetError> {
    let foliage = tree.foliage();
    let templates: Vec<_> = foliage
        .templates
        .iter()
        .map(|template| tissue_mesh(&template.shape).map_err(AssetError::Foliage))
        .collect::<Result<_, _>>()?;
    let triangles: Vec<u64> = templates
        .iter()
        .map(|mesh| {
            mesh.faces()
                .map(|face| mesh.face_loop(face).count().saturating_sub(2) as u64)
                .sum()
        })
        .collect();
    let instances = foliage
        .instances
        .iter()
        .map(|leaf| {
            let branch = tree
                .skeleton()
                .index_of(leaf.id.branch)
                .expect("source leaf belongs to its skeleton");
            Ok(AssetInstance {
                leaf: Some(leaf.id),
                canopy_normal: leaf.canopy_normal,
                template: leaf.template,
                transform: Affine3A::from_scale_rotation_translation(
                    Vec3::splat(leaf.scale),
                    leaf.rotation,
                    leaf.position,
                ),
                branch: u32::try_from(branch).map_err(|_| AssetError::TooLarge)?,
            })
        })
        .collect::<Result<Vec<_>, AssetError>>()?;
    let bark = mesh_skeleton(tree.skeleton(), bark).map_err(AssetError::Mesh)?;
    let report = DetailedReport {
        bark_triangles: bark.report.triangles(),
        templates: templates.len() as u64,
        template_triangles: triangles.iter().sum(),
        instances: instances.len() as u64,
        instanced_triangles: foliage
            .instances
            .iter()
            .map(|leaf| triangles[leaf.template as usize])
            .sum(),
    };
    Ok(DetailedAsset {
        bark: bark.mesh,
        leaves: AssetInstances {
            templates,
            instances,
        },
        branches: tree
            .skeleton()
            .branches()
            .iter()
            .map(|branch| branch.id)
            .collect(),
        materials: [
            TreeMaterial {
                name: "bark".into(),
                role: MaterialRole::Bark,
                params: tree.materials().bark,
                alpha_cutoff: None,
                double_sided: false,
            },
            TreeMaterial {
                name: "leaf".into(),
                role: MaterialRole::Leaf,
                params: tree.materials().leaf,
                alpha_cutoff: None,
                double_sided: true,
            },
        ],
        report,
    })
}
