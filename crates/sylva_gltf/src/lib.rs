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
//!
//! glTF multiplies factors by textures, so with an ORM texture bound the
//! metallic and roughness factors are 1 and the texture carries the values.
//! The projection is lossy: OpenPBR's thin-walled transmission, subsurface,
//! coat and fuzz have no core glTF equivalent. Leaf translucency waits for
//! `KHR_materials_diffuse_transmission` support in `exedra_gltf`.

use std::fmt;

use exedra_assembly::{
    Assembly, AssemblyError, CompileError, CompilePolicy, NormalsSource, PartCompiler,
};
use exedra_gltf::{
    GlbExport, GltfAttribute, GltfError, GltfExportOptions, MaterialResolver, Texture,
    export_glb_with_materials,
};
use exedra_math::Placement3;
use exedra_mesh::{ExtractAttribute, TangentUv};
use serde_json::{Value, json};
use sylva_asset::{TreeAsset, TreeMaterial};
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

/// Texture slots per material, in resolver index order.
const SLOTS: u32 = 3;

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
            _ => textures.normal,
        }?;
        Some(Texture {
            image,
            mime_type: "image/png",
            sampler: None,
        })
    }
}

/// Writes level `level` of `asset` as a binary glTF.
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
    let lod = asset.lods.get(level).ok_or(ExportError::NoLevel(level))?;
    if textures.len() != asset.materials.len() {
        return Err(ExportError::Textures {
            expected: asset.materials.len(),
            found: textures.len(),
        });
    }
    let mut assembly = Assembly::new();
    for mesh in &lod.meshes {
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
        attributes: vec![ExtractAttribute::new(BRANCH_LAYER, u32::MAX)],
        tangents: Some(TangentUv::Primary),
        ..CompilePolicy::default()
    };
    let compiled = PartCompiler::new()
        .compile_parts(&assembly, &policy)
        .map_err(ExportError::Compile)?;
    let mappings = [GltfAttribute::custom(BRANCH_LAYER, "_BRANCH")];
    let options = GltfExportOptions::z_up_to_y_up().with_attributes(&mappings);
    let resolver = Resolver { asset, textures };
    export_glb_with_materials(&assembly, &compiled, &resolver, options).map_err(ExportError::Gltf)
}

#[cfg(test)]
mod tests;
