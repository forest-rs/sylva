// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Render-ready sylva trees.
//!
//! [`build_asset`] turns a grown, foliated tree's [`LodChain`] into a
//! [`TreeAsset`]: per level, one `exedra_mesh` [`Mesh`] per material, with
//! corner UVs, authored corner normals and, on every vertex, the
//! [`BRANCH_LAYER`] index of the skeleton branch it came from (a leaf takes
//! its site's branch). Materials are [`TreeMaterial`]s carrying OpenPBR
//! parameters ([`openpbr::Parameters`] in linear sRGB) and their alpha
//! test; textures stay with the caller, keyed by material, because encoded
//! images belong to an export profile, not to the tree.
//!
//! - **Bark** is the level's bark mesh as meshed.
//! - **Leaves** merge every kept leaf instance into one mesh: its template
//!   under the instance transform, each corner's normal set to the leaf's
//!   canopy normal for soft crown shading.
//! - **Cluster cards** and the **impostor** are their crossed quads, each
//!   with its own material, since each samples its own baked atlas.
//!
//! Adapters such as `sylva_gltf` read the asset; nothing here knows a file
//! format.
//!
//! # Example
//! ```rust,ignore
//! let chain = sylva_lod::build_lods(&skeleton, &foliage, &mesh_params, &policy)?;
//! let asset = sylva_asset::build_asset(&skeleton, &foliage, &chain, &TreeMaterials::default())?;
//! assert_eq!(asset.lods.len(), chain.levels.len() + 1);
//! ```

#![no_std]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use exedra_mesh::{BuildError, ExtractParams, Mesh, MeshBuilder, TriMesh, op};
use glam::Vec3;
use openpbr::Parameters;
use openpbr::color::{LinearSrgb, OpaqueColor};
use sylva_foliage::{Foliage, LeafInstance};
use sylva_lod::LodChain;
use sylva_mesh::BRANCH_LAYER;
use sylva_skeleton::Skeleton;

/// What a material covers, which tells an exporter which textures it takes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MaterialRole {
    /// Branch bark.
    Bark,
    /// Individual leaves.
    Leaf,
    /// Cluster cards of the level with this index.
    Cards {
        /// Index of the level in [`TreeAsset::lods`].
        level: u32,
    },
    /// The impostor's billboards.
    Impostor,
}

/// One material: OpenPBR parameters plus how it is cut.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeMaterial {
    /// Unique name, used as the material key by exporters.
    pub name: String,
    /// What it covers.
    pub role: MaterialRole,
    /// OpenPBR parameters, colours in linear sRGB.
    pub params: Parameters<LinearSrgb>,
    /// Alpha-test threshold on the base colour's coverage, or `None` for
    /// opaque.
    pub alpha_cutoff: Option<f32>,
    /// Draw back faces too, as leaves and cards need.
    pub double_sided: bool,
}

/// OpenPBR parameters for the tree's two authored materials; card and
/// impostor materials derive from the leaf material.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeMaterials {
    /// Bark.
    pub bark: Parameters<LinearSrgb>,
    /// Leaves.
    pub leaf: Parameters<LinearSrgb>,
    /// Alpha-test threshold for leaves, cards and impostors.
    pub alpha_cutoff: f32,
}

impl Default for TreeMaterials {
    /// Rough grey-brown bark and a thin-walled, slightly glossy green leaf
    /// that scatters 40% of the light through its blade with a green tint;
    /// both are multiplied by their textures where exporters bind them.
    fn default() -> Self {
        let mut bark = Parameters::<LinearSrgb>::DEFAULT;
        bark.base_color = OpaqueColor::new([1.0, 1.0, 1.0]);
        bark.specular_roughness = 0.85;
        let mut leaf = Parameters::<LinearSrgb>::DEFAULT;
        leaf.base_color = OpaqueColor::new([1.0, 1.0, 1.0]);
        leaf.specular_roughness = 0.5;
        leaf.geometry_thin_walled = true;
        leaf.subsurface_weight = 0.4;
        leaf.subsurface_color = OpaqueColor::new([0.2, 0.36, 0.05]);
        Self {
            bark,
            leaf,
            alpha_cutoff: 0.5,
        }
    }
}

/// One mesh of a level, drawn with one material.
#[derive(Clone, Debug)]
pub struct AssetMesh {
    /// Name within the level: `bark`, `leaves` or `cards`.
    pub name: &'static str,
    /// Index into [`TreeAsset::materials`].
    pub material: u32,
    /// The mesh: corner UVs, authored corner normals and [`BRANCH_LAYER`].
    pub mesh: Mesh,
}

/// One level of the asset.
#[derive(Clone, Debug)]
pub struct AssetLod {
    /// Use this level while the tree's projected height is at least this
    /// fraction of the screen height.
    pub screen_size: f32,
    /// Width of the dithered crossfade into the next coarser level, as a
    /// fraction of `screen_size`.
    pub crossfade: f32,
    /// Its meshes; a level may have no meshes of some kind.
    pub meshes: Vec<AssetMesh>,
}

/// Deterministic counts describing an asset.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetReport {
    /// Levels, the impostor included.
    pub levels: u64,
    /// Meshes over all levels.
    pub meshes: u64,
    /// Polygon faces over all levels.
    pub faces: u64,
}

/// A render-ready tree: levels finest first, the impostor last when the
/// chain has one.
#[derive(Clone, Debug)]
pub struct TreeAsset {
    /// Levels, finest first.
    pub lods: Vec<AssetLod>,
    /// Materials the meshes reference.
    pub materials: Vec<TreeMaterial>,
    /// Deterministic counts.
    pub report: AssetReport,
}

/// Why an asset could not be built.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AssetError {
    /// Building a mesh failed.
    Build(BuildError),
    /// Writing a mesh attribute failed.
    Kernel,
    /// A mesh is too large for 32-bit indices.
    TooLarge,
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Build(error) => write!(f, "asset mesh: {error}"),
            Self::Kernel => f.write_str("asset mesh attribute write failed"),
            Self::TooLarge => f.write_str("asset mesh exceeds 32-bit indices"),
        }
    }
}

impl core::error::Error for AssetError {}

/// Builds the asset for a tree's LOD chain.
///
/// # Errors
///
/// [`AssetError`] when a mesh cannot be built.
pub fn build_asset(
    skeleton: &Skeleton,
    foliage: &Foliage,
    chain: &LodChain,
    materials: &TreeMaterials,
) -> Result<TreeAsset, AssetError> {
    let mut out = Vec::new();
    let mut table = alloc::vec![
        TreeMaterial {
            name: String::from("bark"),
            role: MaterialRole::Bark,
            params: materials.bark,
            alpha_cutoff: None,
            double_sided: false,
        },
        TreeMaterial {
            name: String::from("leaf"),
            role: MaterialRole::Leaf,
            params: materials.leaf,
            alpha_cutoff: Some(materials.alpha_cutoff),
            double_sided: true,
        },
    ];
    let card_material = |name: String, role| TreeMaterial {
        name,
        role,
        params: materials.leaf,
        alpha_cutoff: Some(materials.alpha_cutoff),
        double_sided: true,
    };
    let leaf_branches: Vec<u32> = foliage
        .instances
        .iter()
        .map(|leaf| {
            let site = &skeleton.sites()[leaf.site as usize];
            skeleton
                .index_of(site.branch)
                .and_then(|b| u32::try_from(b).ok())
                .unwrap_or(u32::MAX)
        })
        .collect();
    for (index, level) in chain.levels.iter().enumerate() {
        let mut meshes = Vec::new();
        meshes.push(AssetMesh {
            name: "bark",
            material: 0,
            mesh: level.bark.mesh.clone(),
        });
        if !level.leaves.is_empty() {
            meshes.push(AssetMesh {
                name: "leaves",
                material: 1,
                mesh: leaf_mesh(foliage, &level.templates, &level.leaves, &leaf_branches)?,
            });
        }
        if let Some(clusters) = &level.clusters {
            let material = u32::try_from(table.len()).map_err(|_| AssetError::TooLarge)?;
            let level_index = u32::try_from(index).map_err(|_| AssetError::TooLarge)?;
            table.push(card_material(
                format!("lod{index}-cards"),
                MaterialRole::Cards { level: level_index },
            ));
            let branches: Vec<u32> = clusters
                .cards
                .iter()
                .flat_map(|card| {
                    core::iter::repeat_n(card.root, 4 * clusters.params.planes as usize)
                })
                .collect();
            meshes.push(AssetMesh {
                name: "cards",
                material,
                mesh: quad_mesh(&clusters.geometry(), &branches)?,
            });
        }
        out.push(AssetLod {
            screen_size: level.level.screen_size,
            crossfade: level.level.crossfade,
            meshes,
        });
    }
    if let Some(impostor) = &chain.impostor {
        let material = u32::try_from(table.len()).map_err(|_| AssetError::TooLarge)?;
        table.push(card_material(
            String::from("impostor"),
            MaterialRole::Impostor,
        ));
        let geometry = impostor.geometry();
        let trunk = skeleton
            .branches()
            .iter()
            .position(|b| b.parent.is_none())
            .and_then(|b| u32::try_from(b).ok())
            .unwrap_or(u32::MAX);
        let branches = alloc::vec![trunk; geometry.positions.len()];
        out.push(AssetLod {
            screen_size: impostor.policy.screen_size,
            crossfade: impostor.policy.crossfade,
            meshes: alloc::vec![AssetMesh {
                name: "cards",
                material,
                mesh: quad_mesh(&geometry, &branches)?,
            }],
        });
    }
    let report = AssetReport {
        levels: out.len() as u64,
        meshes: out.iter().map(|l| l.meshes.len() as u64).sum(),
        faces: out
            .iter()
            .flat_map(|l| &l.meshes)
            .map(|m| m.mesh.faces().count() as u64)
            .sum(),
    };
    Ok(TreeAsset {
        lods: out,
        materials: table,
        report,
    })
}

/// Merges leaf instances of `templates` into one mesh.
fn leaf_mesh(
    foliage: &Foliage,
    templates: &[Mesh],
    leaves: &[LeafInstance],
    leaf_branches: &[u32],
) -> Result<Mesh, AssetError> {
    let templates: Vec<TriMesh> = templates
        .iter()
        .map(|t| t.to_trimesh(&ExtractParams::default()).0)
        .collect();
    // Leaf instances at a level are rescaled copies of the foliage's; find
    // each one's branch through its site.
    let branch_of_site = |site: u32| {
        foliage
            .instances
            .binary_search_by_key(&site, |l| l.site)
            .ok()
            .map_or(u32::MAX, |i| leaf_branches[i])
    };
    let mut merged = Merged::default();
    for leaf in leaves {
        let t = &templates[leaf.template as usize];
        let branch = branch_of_site(leaf.site);
        let base = merged.positions.len();
        for p in &t.positions {
            let world = leaf.position + leaf.rotation * (Vec3::from_array(*p) * leaf.scale);
            merged.positions.push(world.to_array());
            merged.branches.push(branch);
        }
        merged.uvs.extend_from_slice(&t.uvs);
        merged.normals.extend(core::iter::repeat_n(
            leaf.canopy_normal.to_array(),
            t.positions.len(),
        ));
        for tri in t.indices.as_chunks::<3>().0 {
            merged.triangles.push(tri.map(|i| base + i as usize));
        }
    }
    merged.build()
}

/// Card quads as a mesh, one branch index per vertex.
fn quad_mesh(geometry: &TriMesh, branches: &[u32]) -> Result<Mesh, AssetError> {
    let merged = Merged {
        positions: geometry.positions.clone(),
        uvs: geometry.uvs.clone(),
        normals: geometry.normals.clone(),
        branches: branches.to_vec(),
        triangles: geometry
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| t.map(|i| i as usize))
            .collect(),
    };
    merged.build()
}

/// Triangle soup with per-vertex UVs, normals and branches, turned into a
/// mesh whose corners carry them.
#[derive(Default)]
struct Merged {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    normals: Vec<[f32; 3]>,
    branches: Vec<u32>,
    triangles: Vec<[usize; 3]>,
}

impl Merged {
    fn build(self) -> Result<Mesh, AssetError> {
        let mut builder = MeshBuilder::new();
        for p in &self.positions {
            builder.push_vertex(*p);
        }
        for tri in &self.triangles {
            let loop_indices = tri.map(|i| u32::try_from(i).unwrap_or(u32::MAX));
            builder.add_face(&loop_indices).map_err(AssetError::Build)?;
        }
        let built = builder.build().map_err(AssetError::Build)?;
        let mut mesh = built.mesh;
        mesh.define_dense_layer(BRANCH_LAYER, u32::MAX)
            .map_err(|_| AssetError::Kernel)?;
        // Sparse corner layers insert fastest in ascending ID order.
        let mut corners: Vec<(exedra_mesh::CornerId, usize)> = Vec::new();
        for (face, edges) in built.face_edge_ids.iter().enumerate() {
            for (i, &vertex) in self.triangles[face].iter().enumerate() {
                // Edge `i - 1` ends at loop vertex `i`.
                corners.push((edges[(i + 2) % 3], vertex));
            }
        }
        corners.sort_by_key(|(corner, _)| corner.index());
        let mut edit = mesh.edit();
        for &(corner, vertex) in &corners {
            op::set_corner_uv(&mut edit, corner, self.uvs[vertex])
                .map_err(|_| AssetError::Kernel)?;
            op::set_corner_normal_override(&mut edit, corner, Some(self.normals[vertex]))
                .map_err(|_| AssetError::Kernel)?;
        }
        for (vertex, &branch) in built.vertex_ids.iter().zip(&self.branches) {
            op::set_attribute(&mut edit, BRANCH_LAYER, *vertex, branch)
                .map_err(|_| AssetError::Kernel)?;
        }
        let _: () = edit.finish();
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests;
