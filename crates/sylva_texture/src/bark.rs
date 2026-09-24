// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tileable bark material sets from dapple recipes.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use dapple_encode::{Image, MaterialMaps};
use dapple_field::program::Fingerprint;
use dapple_graph::{RasterData, Recipe, RecipeError};
use dapple_material::Grid;
use dapple_material::module::{Bind, Context, Module};
use dapple_material::resource::NoResources;
use dapple_raster::Edge;
use dapple_raster::typed::Storage;

use crate::TextureError;

/// Texels per tile of the material graph a bark recipe runs in. Tile size
/// only partitions the work; the maps do not depend on it.
const GRAPH_TILE: u32 = 64;

/// A bark material set: the maps ready to pack.
#[derive(Clone, Debug)]
pub struct BarkSet {
    /// The maps the recipe's outputs fill: base colour, normals, roughness,
    /// occlusion.
    pub maps: MaterialMaps,
    /// The recipe's content fingerprint; `None` for a module bark.
    pub fingerprint: Option<Fingerprint>,
}

/// Runs a dapple bark recipe and collects its outputs as material maps.
///
/// Bark is a [`dapple_graph::Recipe`], data rather than code, so sylva and
/// dapple share one oak bark instead of two that drift apart. A recipe's
/// outputs name `dapple_encode` material roles (`base_color`, `normal`,
/// `specular_roughness`, `occlusion`, `base_metalness`, `opacity`,
/// `subsurface_weight`, `subsurface_color`) and the realize or raster nodes
/// that fill them, one node per channel group.
///
/// The recipe's repeating tile is one tile of `sylva_mesh`'s bark UVs, so its
/// world size must equal `sylva_mesh::BarkMapping::tile_size`; a recipe
/// states that size and scales its normal and occlusion relief to it.
///
/// # Errors
///
/// [`TextureError::Recipe`] when the recipe cannot be fingerprinted, built
/// or run; [`TextureError::Output`] when an output names an unknown role,
/// its channels do not fit it, or no output fills `base_color`.
pub fn bark(recipe: &Recipe) -> Result<BarkSet, TextureError> {
    let failed = |error: RecipeError| TextureError::Recipe(error.to_string());
    let fingerprint = recipe.fingerprint().map_err(failed)?;
    let (mut graph, nodes) = recipe.build(GRAPH_TILE).map_err(failed)?;
    graph.run().map_err(|e| failed(RecipeError::Material(e)))?;
    let mut maps = MaterialMaps::default();
    for output in &recipe.outputs {
        let wrong = || TextureError::Output {
            role: output.role.clone(),
        };
        let (slot, expected) = match output.role.as_str() {
            "base_color" => (&mut maps.base_color, 3),
            "normal" => (&mut maps.normal, 3),
            "specular_roughness" => (&mut maps.specular_roughness, 1),
            "occlusion" => (&mut maps.occlusion, 1),
            "base_metalness" => (&mut maps.base_metalness, 1),
            "opacity" => (&mut maps.opacity, 1),
            "subsurface_weight" => (&mut maps.subsurface_weight, 1),
            "subsurface_color" => (&mut maps.subsurface_color, 3),
            _ => return Err(wrong()),
        };
        let sources: Vec<Image> = output
            .channels
            .iter()
            .map(|label| {
                let node = nodes.get(label).ok_or_else(wrong)?;
                let value = graph.raster_value(*node).ok_or_else(wrong)?;
                match &value.data {
                    RasterData::Scalar(r) => Ok(Image::from(r)),
                    RasterData::Vector3(r) => Ok(Image::from(r)),
                    RasterData::Typed(typed) => match typed.storage() {
                        Storage::F32(r) => Ok(Image::from(r)),
                        Storage::F32x2(r) => Ok(Image::from(r)),
                        Storage::F32x3(r) => Ok(Image::from(r)),
                        Storage::U32(_) => Err(wrong()),
                    },
                }
            })
            .collect::<Result<_, TextureError>>()?;
        let first = sources.first().ok_or_else(wrong)?;
        let grid = |i: &Image| (i.width(), i.height(), i.edge());
        let channels: usize = sources.iter().map(Image::channels).sum();
        if channels != expected || sources.iter().any(|s| grid(s) != grid(first)) {
            return Err(wrong());
        }
        // Interleave the sources' channels texel by texel.
        let texels = first.width() as usize * first.height() as usize;
        let mut values = Vec::with_capacity(texels * channels);
        for i in 0..texels {
            for source in &sources {
                let c = source.channels();
                values.extend_from_slice(&source.values()[i * c..(i + 1) * c]);
            }
        }
        *slot = Some(
            Image::new(
                first.width(),
                first.height(),
                channels,
                first.edge(),
                values,
            )
            .map_err(TextureError::Encode)?,
        );
    }
    if maps.base_color.is_none() {
        return Err(TextureError::Output {
            role: String::from("base_color"),
        });
    }
    Ok(BarkSet {
        maps,
        fingerprint: Some(fingerprint),
    })
}

/// Runs a dapple bark module at a stem's `girth` (circumference, metres)
/// and `height` on the trunk (metres) over one tile, `size` texels a side.
///
/// Dapple's bark modules (`dapple_library::modules`: beech, silver birch,
/// Scots pine, Norway spruce) are calibrated against measured bark
/// reflectance and follow a stem's girth and height, where a recipe is one
/// fixed stage. Their tile is one metre of bark, as `sylva_mesh`'s bark UVs
/// map by default. The module's material is lowered to maps with
/// `dapple_material::lower::maps`, which derives normals from its height.
///
/// # Errors
///
/// [`TextureError::Params`] for a zero `size`, or
/// [`TextureError::Recipe`] when the module fails or yields no material.
pub fn bark_module(
    module: &dyn Module,
    girth: f32,
    height: f32,
    size: u32,
) -> Result<BarkSet, TextureError> {
    if size == 0 {
        return Err(TextureError::Params { name: "size" });
    }
    #[expect(clippy::cast_precision_loss, reason = "texture sizes are small")]
    let grid = Grid {
        width: size,
        height: size,
        origin: glam::Vec2::ZERO,
        texel: glam::Vec2::splat(1.0 / size as f32),
        edge: Edge::Wrap,
    };
    let failed = |e: &dyn core::fmt::Display| TextureError::Recipe(e.to_string());
    let mut context = Context::new(grid, &NoResources);
    let mut outputs = context
        .instantiate(
            module,
            "bark",
            Bind::new().scalar("girth", girth).scalar("height", height),
        )
        .map_err(|e| failed(&e))?;
    let material = outputs
        .take_material("material")
        .ok_or_else(|| TextureError::Recipe(String::from("the module yields no material")))?;
    let (maps, _) = dapple_material::lower::maps(&material).map_err(|e| failed(&e))?;
    if maps.base_color.is_none() {
        return Err(TextureError::Output {
            role: String::from("base_color"),
        });
    }
    Ok(BarkSet {
        maps,
        fingerprint: None,
    })
}
