// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! glTF export of [`TreeAsset`]s through `exedra_gltf`.
//!
//! [`export_lod_glb`] writes one level of a tree as a binary glTF: each of
//! the level's meshes becomes a baked `exedra_assembly` part with one
//! instance, compiled with the authored corner normals, MikkTSpace tangents
//! from the primary UVs, and the [`BRANCH_LAYER`] stream exported as the
//! custom attribute `_BRANCH` (float-encoded branch indices, exact below
//! 2^24). Coordinates convert from sylva's Z-up to glTF's Y-up.
//!
//! [`ExportOptions::leaves`] chooses how leaves are written. The default,
//! [`LeafExport::Merged`], writes the level's merged leaf mesh with canopy
//! normals and per-vertex branches. [`LeafExport::Instanced`] writes each
//! leaf template once and places it per leaf with `EXT_mesh_gpu_instancing`:
//! far smaller files, but leaves shade by their template normals instead of
//! canopy normals and carry [`NO_BRANCH`].
//!
//! Materials are projected from OpenPBR to glTF's metallic-roughness core:
//!
//! | glTF | from |
//! |---|---|
//! | `baseColorFactor` | `base_color * base_weight`, alpha 1 |
//! | `metallicFactor` | `base_metalness` |
//! | `roughnessFactor` | `specular_roughness` |
//! | `baseColorTexture` | the material's base colour texture (sRGB, coverage in alpha) |
//! | `metallicRoughnessTexture`, `occlusionTexture` | its ORM texture (occlusion R, roughness G, metalness B) |
//! | `normalTexture` | its normal texture (tangent space, +Y up) |
//! | `alphaMode`, `alphaCutoff` | `MASK` at [`TreeMaterial::alpha_cutoff`], else `OPAQUE` |
//! | `doubleSided` | [`TreeMaterial::double_sided`] |
//! | `KHR_materials_diffuse_transmission` | on a thin-walled material with `subsurface_weight > 0`: `diffuseTransmissionFactor` from `subsurface_weight`, `diffuseTransmissionColorFactor` from `subsurface_color`, and both textures from its diffuse-transmission texture (tint RGB, sRGB; weight A) |
//!
//! glTF multiplies factors by textures, so with an ORM or diffuse-transmission
//! texture bound the matching factors are 1 and the texture carries the
//! values. OpenPBR's thin-walled subsurface is what glTF's diffuse
//! transmission models, so leaf translucency survives; volumetric
//! subsurface, specular transmission, coat and fuzz do not.

use std::fmt;

use exedra_assembly::{
    Assembly, AssemblyError, CompileError, CompilePolicy, NormalsSource, PartCompiler,
};
use exedra_gltf::{
    GlbExport, GltfAttribute, GltfError, GltfExportOptions, GltfInstancing, MaterialResolver,
    Texture, export_glb_with_materials,
};
use exedra_math::Placement3;
use exedra_mesh::{ExtractAttribute, TangentUv};
use serde_json::{Value, json};
use sylva_asset::{AssetLeaves, TreeAsset, TreeMaterial};
use sylva_mesh::BRANCH_LAYER;

/// Encoded PNG textures for one material, as glTF reads them.
#[derive(Copy, Clone, Debug, Default)]
pub struct MaterialTextures<'a> {
    /// Base colour: sRGB, coverage in alpha.
    pub base_color: Option<&'a [u8]>,
    /// Occlusion, roughness and metalness in R, G and B, linear.
    pub orm: Option<&'a [u8]>,
    /// Tangent-space normal, +Y up, linear.
    pub normal: Option<&'a [u8]>,
    /// Diffuse transmission: tint in RGB (sRGB) and weight in A (linear),
    /// as dapple's glTF profile packs a thin-walled subsurface.
    pub diffuse_transmission: Option<&'a [u8]>,
}

/// Why an export failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExportError {
    /// The level index is out of range.
    NoLevel(usize),
    /// `textures` does not have one entry per asset material.
    Textures {
        /// Materials in the asset.
        expected: usize,
        /// Entries supplied.
        found: usize,
    },
    /// Assembling the level failed.
    Assembly(AssemblyError),
    /// Compiling the level failed.
    Compile(CompileError),
    /// Writing glTF failed.
    Gltf(GltfError),
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLevel(level) => write!(f, "the asset has no level {level}"),
            Self::Textures { expected, found } => {
                write!(f, "{found} texture entries for {expected} materials")
            }
            Self::Assembly(error) => write!(f, "assembly: {error}"),
            Self::Compile(error) => write!(f, "compile: {error}"),
            Self::Gltf(error) => write!(f, "glTF: {error}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// The `_BRANCH` value of a vertex with no branch layer, such as an
/// instanced leaf's: 2^24 - 1, the largest index the float encoding carries
/// exactly.
pub const NO_BRANCH: u32 = (1 << 24) - 1;

/// How [`export_lod_glb_with`] writes a level's leaves.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum LeafExport {
    /// The merged leaf mesh: every leaf's vertices, with canopy normals and
    /// per-vertex branch indices.
    #[default]
    Merged,
    /// Each leaf template once, placed per leaf with
    /// `EXT_mesh_gpu_instancing` (which the file then requires). Leaves
    /// shade by their template normals and carry [`NO_BRANCH`].
    Instanced,
}

/// Export options.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportOptions {
    /// How leaves are written.
    pub leaves: LeafExport,
}

impl ExportOptions {
    /// These options with `leaves`.
    #[must_use]
    pub const fn with_leaves(mut self, leaves: LeafExport) -> Self {
        self.leaves = leaves;
        self
    }
}

/// Texture slots per material, in resolver index order.
const SLOTS: u32 = 4;

struct Resolver<'a> {
    asset: &'a TreeAsset,
    textures: &'a [MaterialTextures<'a>],
}

impl Resolver<'_> {
    fn material(&self, index: usize, material: &TreeMaterial) -> Value {
        let p = &material.params;
        let textures = self.textures[index];
        let base = u32::try_from(index).expect("few materials") * SLOTS;
        let [r, g, b] = p.base_color.components;
        let w = p.base_weight;
        let mut pbr = json!({
            "baseColorFactor": [r * w, g * w, b * w, 1.0],
            "metallicFactor": p.base_metalness,
            "roughnessFactor": p.specular_roughness,
        });
        if textures.base_color.is_some() {
            pbr["baseColorTexture"] = json!({ "index": base });
        }
        let mut out = json!({ "name": material.name });
        if textures.orm.is_some() {
            pbr["metallicFactor"] = json!(1.0);
            pbr["roughnessFactor"] = json!(1.0);
            pbr["metallicRoughnessTexture"] = json!({ "index": base + 1 });
            out["occlusionTexture"] = json!({ "index": base + 1 });
        }
        if textures.normal.is_some() {
            out["normalTexture"] = json!({ "index": base + 2 });
        }
        out["pbrMetallicRoughness"] = pbr;
        match material.alpha_cutoff {
            Some(cutoff) => {
                out["alphaMode"] = json!("MASK");
                out["alphaCutoff"] = json!(cutoff);
            }
            None => out["alphaMode"] = json!("OPAQUE"),
        }
        out["doubleSided"] = json!(material.double_sided);
        if p.geometry_thin_walled && p.subsurface_weight > 0.0 {
            let [r, g, b] = p.subsurface_color.components;
            let mut transmission = json!({
                "diffuseTransmissionFactor": p.subsurface_weight,
                "diffuseTransmissionColorFactor": [r, g, b],
            });
            if textures.diffuse_transmission.is_some() {
                transmission["diffuseTransmissionFactor"] = json!(1.0);
                transmission["diffuseTransmissionColorFactor"] = json!([1.0, 1.0, 1.0]);
                transmission["diffuseTransmissionTexture"] = json!({ "index": base + 3 });
                transmission["diffuseTransmissionColorTexture"] = json!({ "index": base + 3 });
            }
            out["extensions"] = json!({ "KHR_materials_diffuse_transmission": transmission });
        }
        out
    }
}

impl MaterialResolver for Resolver<'_> {
    fn resolve(&self, key: &str) -> Option<Value> {
        let (index, material) = self
            .asset
            .materials
            .iter()
            .enumerate()
            .find(|(_, m)| m.name == key)?;
        Some(self.material(index, material))
    }

    fn resolve_texture(&self, index: u32) -> Option<Texture<'_>> {
        let textures = self.textures.get((index / SLOTS) as usize)?;
        let image = match index % SLOTS {
            0 => textures.base_color,
            1 => textures.orm,
            2 => textures.normal,
            _ => textures.diffuse_transmission,
        }?;
        Some(Texture {
            image,
            mime_type: "image/png",
            sampler: None,
        })
    }
}

/// Writes level `level` of `asset` as a binary glTF with default options.
///
/// `textures` holds one entry per asset material, in material order; an
/// entry's missing textures leave that slot unbound.
///
/// # Errors
///
/// [`ExportError`] for a missing level, a texture table of the wrong
/// length, or an assembly, compile or glTF failure.
pub fn export_lod_glb(
    asset: &TreeAsset,
    level: usize,
    textures: &[MaterialTextures<'_>],
) -> Result<GlbExport, ExportError> {
    export_lod_glb_with(asset, level, textures, ExportOptions::default())
}

/// Writes level `level` of `asset` as a binary glTF.
///
/// As [`export_lod_glb`], with `options`.
///
/// # Errors
///
/// As [`export_lod_glb`].
pub fn export_lod_glb_with(
    asset: &TreeAsset,
    level: usize,
    textures: &[MaterialTextures<'_>],
    options: ExportOptions,
) -> Result<GlbExport, ExportError> {
    let lod = asset.lods.get(level).ok_or(ExportError::NoLevel(level))?;
    if textures.len() != asset.materials.len() {
        return Err(ExportError::Textures {
            expected: asset.materials.len(),
            found: textures.len(),
        });
    }
    let instanced = match (options.leaves, &lod.leaves) {
        (LeafExport::Instanced, Some(leaves)) => Some(leaves),
        _ => None,
    };
    let mut assembly = Assembly::new();
    for mesh in &lod.meshes {
        if instanced.is_some() && mesh.name == "leaves" {
            add_leaf_instances(&mut assembly, level, mesh.material, instanced, asset)?;
            continue;
        }
        let key = format!("lod{level}/{}", mesh.name);
        let material = &asset.materials[mesh.material as usize];
        let part = assembly
            .add_baked_part(&key, mesh.mesh.clone(), &["surface"])
            .map_err(ExportError::Assembly)?;
        assembly
            .set_default_slot(part, "surface")
            .map_err(ExportError::Assembly)?;
        assembly
            .set_part_material(part, "surface", &material.name)
            .map_err(ExportError::Assembly)?;
        assembly
            .add_instance(None, mesh.name, part, Placement3::IDENTITY)
            .map_err(ExportError::Assembly)?;
    }
    let policy = CompilePolicy {
        normals: NormalsSource::CustomOrDerived,
        attributes: vec![ExtractAttribute::new(BRANCH_LAYER, NO_BRANCH)],
        tangents: Some(TangentUv::Primary),
        ..CompilePolicy::default()
    };
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &policy)
        .map_err(ExportError::Compile)?;
    let mappings = [GltfAttribute::custom(BRANCH_LAYER, "_BRANCH")];
    let mut gltf = GltfExportOptions::z_up_to_y_up().with_attributes(&mappings);
    if instanced.is_some() {
        gltf = gltf.with_instancing(GltfInstancing::GpuInstancing);
    }
    let resolver = Resolver { asset, textures };
    export_glb_with_materials(&assembly, &compiled, &resolver, gltf).map_err(ExportError::Gltf)
}

/// Adds one part per leaf template and one instance per leaf, all at the
/// root with the leaf material, so glTF export batches them.
fn add_leaf_instances(
    assembly: &mut Assembly,
    level: usize,
    material: u32,
    leaves: Option<&AssetLeaves>,
    asset: &TreeAsset,
) -> Result<(), ExportError> {
    let Some(leaves) = leaves else {
        return Ok(());
    };
    let material = &asset.materials[material as usize];
    let mut parts = Vec::with_capacity(leaves.templates.len());
    for (t, template) in leaves.templates.iter().enumerate() {
        let part = assembly
            .add_baked_part(
                &format!("lod{level}/leaf{t}"),
                template.clone(),
                &["surface"],
            )
            .map_err(ExportError::Assembly)?;
        assembly
            .set_default_slot(part, "surface")
            .map_err(ExportError::Assembly)?;
        assembly
            .set_part_material(part, "surface", &material.name)
            .map_err(ExportError::Assembly)?;
        parts.push(part);
    }
    for (i, leaf) in leaves.instances.iter().enumerate() {
        let q = leaf.rotation;
        let (x, y, z, w) = (
            f64::from(q.x),
            f64::from(q.y),
            f64::from(q.z),
            f64::from(q.w),
        );
        let s = f64::from(leaf.scale);
        let p = leaf.position;
        let rows = [
            [
                s * (1.0 - 2.0 * (y * y + z * z)),
                s * 2.0 * (x * y - w * z),
                s * 2.0 * (x * z + w * y),
                f64::from(p.x),
            ],
            [
                s * 2.0 * (x * y + w * z),
                s * (1.0 - 2.0 * (x * x + z * z)),
                s * 2.0 * (y * z - w * x),
                f64::from(p.y),
            ],
            [
                s * 2.0 * (x * z - w * y),
                s * 2.0 * (y * z + w * x),
                s * (1.0 - 2.0 * (x * x + y * y)),
                f64::from(p.z),
            ],
        ];
        assembly
            .add_instance(
                None,
                &format!("leaf{i}"),
                parts[leaf.template as usize],
                Placement3 { rows },
            )
            .map_err(ExportError::Assembly)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
