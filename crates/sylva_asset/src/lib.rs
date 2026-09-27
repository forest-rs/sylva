// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Generated vegetation and its compiled realizations.
//!
//! [`GeneratedTree`] owns vegetation independently of rendering and storage.
//! [`TreeAsset`] is the conventional mesh/card/LOD output. [`build_detailed`]
//! compiles geometric tissue instances directly into a [`DetailedAsset`].
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
//!   under the instance transform, with explicit surface or canopy shading
//!   selected through [`RasterOptions`].
//! - **Cluster cards** and the **impostor** are their crossed quads, each
//!   with its own material, since each samples its own baked atlas.
//!
//! Adapters such as `sylva_gltf` read the asset; nothing here knows a file
//! format.
//!
//! # Example
//! ```rust
//! use sylva_asset::{GeneratedTree, TreeMaterials, RasterOptions, build_asset, build_detailed};
//! use sylva_foliage::FoliageParams;
//! use sylva_lod::{LodPolicy, build_lods};
//! use sylva_mesh::MeshParams;
//! use sylva_skeleton::Skeleton;
//!
//! fn compile(skeleton: Skeleton, foliage: &FoliageParams) -> Result<(), Box<dyn core::error::Error>> {
//!     let tree = GeneratedTree::new(skeleton, Some(foliage), TreeMaterials::default())?;
//!     let mesh_params = MeshParams::default();
//!     let chain = build_lods(tree.skeleton(), tree.foliage(), &mesh_params, &LodPolicy::default())?;
//!     let raster = build_asset(&tree, &chain, &RasterOptions::default())?;
//!     let detailed = build_detailed(&tree, &mesh_params)?;
//!     assert_eq!(detailed.report.instances, tree.foliage().report.leaves);
//!     Ok(())
//! }
//! ```

#![no_std]

extern crate alloc;

mod detailed;
mod source;
pub use detailed::{DetailedAsset, DetailedReport, build_detailed};
pub use source::GeneratedTree;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use exedra_mesh::{BuildError, ExtractParams, Mesh, MeshBuilder, TriMesh, op};
use glam::{Affine3A, Vec3};
use openpbr::Parameters;
use openpbr::color::{LinearSrgb, OpaqueColor};
use sylva_foliage::{Foliage, FoliageError, LeafId, LeafInstance};
use sylva_lod::LodChain;
use sylva_mesh::{BRANCH_LAYER, MeshError};
use sylva_skeleton::{BranchId, SkeletonError};

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
        Self { bark, leaf }
    }
}

/// How merged leaf geometry is shaded. Surface orientation remains available
/// in the source and retained templates for either choice.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum LeafNormals {
    /// Use transformed template normals, matching the instanced surface path.
    Surface,
    /// Use the artistic tree-space canopy normal for every corner of a leaf.
    #[default]
    Canopy,
}

/// Conventional raster compilation choices, independent of source materials.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RasterOptions {
    /// Alpha-test threshold for leaf cards, cluster cards and impostors.
    pub alpha_cutoff: f32,
    /// Normal policy for merged leaves. Retained templates always preserve
    /// surface normals and placements separately carry canopy normals.
    pub leaf_normals: LeafNormals,
}

impl Default for RasterOptions {
    fn default() -> Self {
        Self {
            alpha_cutoff: 0.5,
            leaf_normals: LeafNormals::Canopy,
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
    /// The same leaves as the `leaves` meshes, as templates and placements,
    /// for exporters that instance them; `None` when the level draws no
    /// individual leaves.
    pub leaves: Option<AssetInstances>,
    /// The same cluster cards as the `cards` mesh, as one unit quad per
    /// baked exemplar and one placement per card plane, for exporters that
    /// instance them; `None` when the level has no cluster cards.
    pub cards: Option<AssetInstances>,
}

/// Shared template meshes and per-copy placements.
///
/// Instancing draws each template once per placement: a species' small
/// library of leaves or baked cluster exemplars, repeated across the
/// crown. Templates preserve surface normals; placements separately retain
/// the canopy normal, stable leaf identity and compiled branch index. A
/// consumer chooses its shading treatment explicitly.
#[derive(Clone, Debug)]
pub struct AssetInstances {
    /// Template meshes in their own space, with UVs and normals.
    pub templates: Vec<Mesh>,
    /// One placement per copy.
    pub instances: Vec<AssetInstance>,
}

/// One copy of a template.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AssetInstance {
    /// Stable source leaf identity; `None` for a cluster card.
    pub leaf: Option<LeafId>,
    /// Tree-space artistic canopy normal, separate from template normals.
    pub canopy_normal: Vec3,
    /// Index into [`AssetInstances::templates`].
    pub template: u32,
    /// Template space to tree space: a rotation and a positive, possibly
    /// non-uniform scale along the template's axes, then a translation.
    pub transform: Affine3A,
    /// Index in the skeleton of the branch the copy belongs to (a leaf's
    /// site branch, a card's cluster root), or `u32::MAX`.
    pub branch: u32,
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
    /// Stable IDs indexed by all compiled branch-provenance streams.
    pub branches: Vec<BranchId>,
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
    /// Detailed bark realization failed.
    Mesh(MeshError),
    /// The generated skeleton is invalid.
    Skeleton(SkeletonError),
    /// Foliage description or realization failed.
    Foliage(FoliageError),
    /// The source cannot provide consistent identities or placements.
    Source(&'static str),
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
            Self::Mesh(error) => write!(f, "asset bark: {error}"),
            Self::Skeleton(error) => write!(f, "asset skeleton: {error}"),
            Self::Foliage(error) => write!(f, "asset foliage: {error}"),
            Self::Source(reason) => write!(f, "asset source: {reason}"),
            Self::Build(error) => write!(f, "asset mesh: {error}"),
            Self::Kernel => f.write_str("asset mesh attribute write failed"),
            Self::TooLarge => f.write_str("asset mesh exceeds 32-bit indices"),
        }
    }
}

impl core::error::Error for AssetError {}

/// Compiles the conventional raster asset from a generated tree and its LOD chain.
///
/// Build `chain` from `tree.skeleton()` and `tree.foliage()`. The chain must
/// belong to this source snapshot. `options` chooses merged-leaf shading;
/// instances preserve surface normals and separately retain the canopy direction.
///
/// # Errors
///
/// [`AssetError`] when a mesh cannot be built or the alpha cutoff is invalid.
pub fn build_asset(
    tree: &GeneratedTree,
    chain: &LodChain,
    options: &RasterOptions,
) -> Result<TreeAsset, AssetError> {
    if !(0.0..=1.0).contains(&options.alpha_cutoff) {
        return Err(AssetError::Source("alpha cutoff must be in [0, 1]"));
    }
    let skeleton = tree.skeleton();
    let foliage = tree.foliage();
    let materials = tree.materials();
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
            alpha_cutoff: Some(options.alpha_cutoff),
            double_sided: true,
        },
    ];
    let card_material = |name: String, role| TreeMaterial {
        name,
        role,
        params: materials.leaf,
        alpha_cutoff: Some(options.alpha_cutoff),
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
        let mut leaves = None;
        if !level.leaves.is_empty() {
            meshes.push(AssetMesh {
                name: "leaves",
                material: 1,
                mesh: leaf_mesh(
                    foliage,
                    &level.templates,
                    &level.leaves,
                    &leaf_branches,
                    options.leaf_normals,
                )?,
            });
            leaves = Some(AssetInstances {
                templates: level.templates.clone(),
                instances: level
                    .leaves
                    .iter()
                    .map(|leaf| AssetInstance {
                        leaf: Some(leaf.id),
                        canopy_normal: leaf.canopy_normal,
                        template: leaf.template,
                        transform: Affine3A::from_scale_rotation_translation(
                            Vec3::splat(leaf.scale),
                            leaf.rotation,
                            leaf.position,
                        ),
                        branch: branch_of_site(foliage, &leaf_branches, leaf.site),
                    })
                    .collect(),
            });
        }
        let mut cards = None;
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
            cards = Some(card_instances(clusters)?);
        }
        out.push(AssetLod {
            screen_size: level.level.screen_size,
            crossfade: level.level.crossfade,
            meshes,
            leaves,
            cards,
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
            leaves: None,
            cards: None,
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
        branches: skeleton.branches().iter().map(|branch| branch.id).collect(),
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
    normals: LeafNormals,
) -> Result<Mesh, AssetError> {
    let templates: Vec<TriMesh> = templates
        .iter()
        .map(|t| t.to_trimesh(&ExtractParams::default()).0)
        .collect();
    let mut merged = Merged::default();
    for leaf in leaves {
        let t = &templates[leaf.template as usize];
        let branch = branch_of_site(foliage, leaf_branches, leaf.site);
        let base = merged.positions.len();
        for p in &t.positions {
            let world = leaf.position + leaf.rotation * (Vec3::from_array(*p) * leaf.scale);
            merged.positions.push(world.to_array());
            merged.branches.push(branch);
        }
        merged.uvs.extend_from_slice(&t.uvs);
        match normals {
            LeafNormals::Canopy => merged.normals.extend(core::iter::repeat_n(
                leaf.canopy_normal.to_array(),
                t.positions.len(),
            )),
            LeafNormals::Surface => merged.normals.extend(
                t.normals
                    .iter()
                    .map(|normal| (leaf.rotation * Vec3::from_array(*normal)).to_array()),
            ),
        }
        for tri in t.indices.as_chunks::<3>().0 {
            merged.triangles.push(tri.map(|i| base + i as usize));
        }
    }
    merged.build()
}

/// Leaf instances at a level are rescaled copies of the foliage's; finds
/// one's branch through its site.
fn branch_of_site(foliage: &Foliage, leaf_branches: &[u32], site: u32) -> u32 {
    foliage
        .instances
        .binary_search_by_key(&site, |l| l.site)
        .ok()
        .map_or(u32::MAX, |i| leaf_branches[i])
}

/// Cluster cards as instances: per baked exemplar, a unit quad (`[-1, 1]`
/// in `X` and `Y`, facing `+Z`) with UVs over its atlas cell; per card plane,
/// a placement stretching it to the plane's half extents.
fn card_instances(clusters: &sylva_lod::Clusters) -> Result<AssetInstances, AssetError> {
    let layout = clusters.layout();
    let templates = (0..clusters.variants.len())
        .map(|v| {
            let mut quad = TriMesh::default();
            sylva_lod::push_unit_quad(
                &mut quad,
                layout.cell(u32::try_from(v).map_err(|_| AssetError::TooLarge)?),
            );
            // Templates carry no branch layer; each placement keeps its own.
            quad_mesh(&quad, &[])
        })
        .collect::<Result<Vec<_>, _>>()?;
    let instances = clusters
        .planes()
        .map(|(card, right, up)| {
            let toward = right.cross(up);
            AssetInstance {
                leaf: None,
                canopy_normal: card.canopy_normal,
                template: card.variant,
                transform: Affine3A::from_cols(
                    (right * card.half.x).into(),
                    (up * card.half.y).into(),
                    toward.into(),
                    card.center.into(),
                ),
                branch: card.root,
            }
        })
        .collect();
    Ok(AssetInstances {
        templates,
        instances,
    })
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
/// mesh whose corners carry them; with no branches, the mesh has no
/// branch layer.
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
        if !self.branches.is_empty() {
            mesh.define_dense_layer(BRANCH_LAYER, u32::MAX)
                .map_err(|_| AssetError::Kernel)?;
        }
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
