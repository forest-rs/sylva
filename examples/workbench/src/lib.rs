// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The browser workbench's narrow bridge to Sylva's native generation and export.
//! Recipes are versioned, bounded inputs. Geometry is the normal detailed GLB
//! realization, including reusable tissue; the browser does not grow another tree.

use dapple_encode::{MaterialMaps, PackSettings, Profile, pack};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sylva_asset::{GeneratedTree, TreeMaterials, build_detailed};
use sylva_gltf::{MaterialTextures, export_detailed_glb};
use sylva_mesh::MeshParams;
use sylva_species::{Growth, Species};
use sylva_texture::{LeafRecipe, bark, bark_module, leaf};
use wasm_bindgen::prelude::*;

type Failure = Box<dyn std::error::Error>;

/// Portable authoring inputs for this workbench version, not a storage schema
/// for `GeneratedTree`. Multipliers modify the bundled species preset.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Recipe interpretation version; currently 1.
    pub version: u32,
    /// One of oak, spruce, beech, birch.
    pub species: String,
    /// Reproducible growth seed (unsigned 32-bit for browser number fidelity).
    pub seed: u32,
    /// Foliage site frequency multiplier, 0.35 through 1.2.
    pub density: f32,
    /// Blade length and width multiplier, 0.7 through 1.4.
    pub leaf_size: f32,
}

impl Recipe {
    fn species(&self) -> Result<Species, Failure> {
        if self.version != 1 {
            return Err("recipe version must be 1".into());
        }
        if !self.density.is_finite() || !(0.35..=1.2).contains(&self.density) {
            return Err("density must be between 0.35 and 1.2".into());
        }
        if !self.leaf_size.is_finite() || !(0.7..=1.4).contains(&self.leaf_size) {
            return Err("leaf_size must be between 0.7 and 1.4".into());
        }
        let source = match self.species.as_str() {
            "oak" => include_str!("../../species_gallery/presets/oak.ron"),
            "spruce" => include_str!("../../species_gallery/presets/spruce.ron"),
            "beech" => include_str!("../../species_gallery/presets/beech.ron"),
            "birch" => include_str!("../../species_gallery/presets/birch.ron"),
            _ => return Err("species must be oak, spruce, beech or birch".into()),
        };
        let mut species: Species = ron::from_str(source)?;
        if let Growth::Hierarchical(h) = &mut species.growth {
            if let Some(sites) = &mut h.trunk.sites {
                sites.per_metre *= self.density;
            }
            for level in &mut h.levels {
                if let Some(sites) = &mut level.sites {
                    sites.per_metre *= self.density;
                }
            }
        }
        if let Some(foliage) = &mut species.foliage {
            foliage.shape.length *= self.leaf_size;
            foliage.shape.width *= self.leaf_size;
        }
        Ok(species)
    }
}

#[derive(Deserialize)]
struct LeafLook {
    green: [f32; 3],
    vein: [f32; 3],
    translucent: [f32; 3],
    translucency: f32,
    mottle: f32,
    roughness: f32,
}

#[derive(Debug)]
struct Images {
    color: Vec<u8>,
    normal: Vec<u8>,
    orm: Vec<u8>,
    transmission: Option<Vec<u8>>,
}

impl Images {
    fn new(maps: &MaterialMaps) -> Result<Self, Failure> {
        let bundle = pack(maps, Profile::Gltf, &PackSettings::default())?;
        let image = |name| -> Result<Vec<u8>, Failure> {
            let texture = bundle.texture(name).ok_or("missing material texture")?;
            Ok(dapple_encode::png::write(texture)?)
        };
        Ok(Self {
            color: image("base_color")?,
            normal: image("normal")?,
            orm: image("orm")?,
            transmission: bundle
                .texture("diffuse_transmission")
                .map(dapple_encode::png::write)
                .transpose()?,
        })
    }

    fn borrowed(&self) -> MaterialTextures<'_> {
        MaterialTextures {
            base_color: Some(&self.color),
            normal: Some(&self.normal),
            orm: Some(&self.orm),
            diffuse_transmission: self.transmission.as_deref(),
        }
    }
}

fn textures(recipe: &Recipe, species: &Species) -> Result<[Images; 2], Failure> {
    use dapple_library::modules::{Beech, Birch, Spruce};
    // A single species bark tile, matching the gallery's representative bark.
    // Height-dependent bark staging remains a separate, richer realization.
    let bark_set = match recipe.species.as_str() {
        "birch" => bark_module(&Birch, 1.1, 6.0, 256)?,
        "beech" => bark_module(&Beech, 2.5, 1.3, 256)?,
        "spruce" => bark_module(&Spruce, 1.5, 1.3, 256)?,
        _ => {
            let source = include_str!("../../species_gallery/presets/oak_bark.toml")
                .replace("width = 1024", "width = 256")
                .replace("height = 1024", "height = 256");
            bark(&toml::from_str::<dapple_graph::Recipe>(&source)?)?
        }
    };
    let look_source = match recipe.species.as_str() {
        "birch" => include_str!("../../species_gallery/presets/birch_leaf.ron"),
        "beech" => include_str!("../../species_gallery/presets/beech_leaf.ron"),
        "spruce" => include_str!("../../species_gallery/presets/spruce_leaf.ron"),
        _ => {
            "(green:(0.03,0.062,0.02),vein:(0.06,0.11,0.03),translucent:(0.07,0.14,0.02),translucency:0.5,mottle:0.15,roughness:0.55)"
        }
    };
    let look: LeafLook = ron::from_str(look_source)?;
    let foliage = species.foliage.as_ref().ok_or("preset has no foliage")?;
    let mut leaves = leaf(&LeafRecipe {
        shape: foliage.shape,
        size: 128,
        green: look.green,
        vein: look.vein,
        translucent: look.translucent,
        translucency: look.translucency,
        mottle: look.mottle,
        roughness: look.roughness,
        seed: u64::from(recipe.seed),
        ..LeafRecipe::default()
    })?;
    // Tissue owns coverage. Do not bake silhouette alpha into these materials.
    leaves.maps.opacity = None;
    Ok([Images::new(&bark_set.maps)?, Images::new(&leaves.maps)?])
}

/// One generated specimen. Call `take_glb` once and `report` before freeing it.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Specimen {
    glb: Vec<u8>,
    report: String,
}

#[wasm_bindgen]
impl Specimen {
    /// Moves the GLB bytes out. A second call returns an empty array.
    pub fn take_glb(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.glb)
    }

    /// JSON recipe, source identities, and deterministic realization counts.
    pub fn report(&self) -> String {
        self.report.clone()
    }
}

/// Generates a detailed specimen and Dapple textures from a JSON recipe.
///
/// Runs synchronously inside a dedicated browser worker. Inputs are validated
/// before growth. The caller can cancel by terminating that worker.
///
/// # Errors
/// Returns an actionable message for invalid recipes or failed compilation.
#[wasm_bindgen]
pub fn generate(recipe_json: &str) -> Result<Specimen, String> {
    generate_inner(recipe_json).map_err(|error| error.to_string())
}

fn generate_inner(recipe_json: &str) -> Result<Specimen, Failure> {
    let recipe: Recipe = serde_json::from_str(recipe_json)?;
    let species = recipe.species()?;
    let grown = species.grow(u64::from(recipe.seed))?;
    let tree = GeneratedTree::new(
        grown.skeleton,
        species.foliage.as_ref(),
        TreeMaterials::default(),
    )?;
    let detailed = build_detailed(&tree, &MeshParams::default())?;
    let images = textures(&recipe, &species)?;
    let glb = export_detailed_glb(&detailed, &images.each_ref().map(Images::borrowed))?;
    let branches: Vec<_> = tree
        .skeleton()
        .branches()
        .iter()
        .map(|b| {
            json!({
                "id": format!("{:016x}", b.id.bits()),
                "parent": b.parent.map(|a| format!("{:016x}", a.parent.bits())),
                "order": b.order,
            })
        })
        .collect();
    let report = json!({
        "recipe": recipe,
        "source_revision": option_env!("SYLVA_WORKBENCH_REVISION").unwrap_or("unknown"),
        "coordinates": { "up": "Y", "units": "metres" },
        "branches": branches,
        "counts": {
            "branches": detailed.branches.len(),
            "placements": detailed.report.instances,
            "templates": detailed.report.templates,
            "stored_triangles": detailed.report.bark_triangles + detailed.report.template_triangles,
            "expanded_triangles": detailed.report.bark_triangles + detailed.report.instanced_triangles,
            "glb_bytes": glb.bytes.len(),
        },
        "limitations": [
            "Static detailed geometry; no wind or automatic LOD.",
            "One representative bark tile; no height-dependent bark blend.",
            "GLB preserves branch indices; this report maps them to source IDs."
        ]
    }).to_string();
    Ok(Specimen {
        glb: glb.bytes,
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> Recipe {
        Recipe {
            version: 1,
            species: "birch".into(),
            seed: 1,
            density: 1.0,
            leaf_size: 1.0,
        }
    }

    #[test]
    fn rejects_invalid_inputs_before_generation() {
        let mut r = recipe();
        r.version = 2;
        assert!(r.species().unwrap_err().to_string().contains("version"));
        r.version = 1;
        r.species = "unknown".into();
        assert!(r.species().unwrap_err().to_string().contains("species"));
        r.species = "birch".into();
        for value in [f32::NAN, 0.0, 2.0] {
            r.density = value;
            assert!(r.species().unwrap_err().to_string().contains("density"));
        }
        assert!(generate(r#"{"version":1}"#).is_err());
    }

    #[test]
    fn foliage_edit_preserves_branch_structure() {
        let base = recipe();
        let mut edited = base.clone();
        edited.density = 0.5;
        edited.leaf_size = 1.2;
        let a = base.species().unwrap().grow(1).unwrap();
        let b = edited.species().unwrap().grow(1).unwrap();
        assert_eq!(a.skeleton.branches(), b.skeleton.branches());
        assert!(b.skeleton.sites().len() < a.skeleton.sites().len());
    }
}
