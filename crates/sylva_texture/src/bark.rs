// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tileable bark material sets.

use alloc::vec::Vec;

use dapple_encode::{Image, MaterialMaps};
use dapple_field::program::{Fingerprint, Op, ProgramBuilder};
use dapple_field::{Basis, CellOutput, Domain, FractalParams};
use dapple_raster::{
    AmbientOcclusion, Edge, HeightToNormal, Raster, RasterOp, Realization, realize,
};

use crate::TextureError;

/// A ridged, fissured bark such as oak's, as one repeating tile.
///
/// The tile covers `tile_size` metres around and along a branch, matching
/// `sylva_mesh::BarkMapping::tile_size`, so texels land at their true world
/// size. Its height is a dapple field program: stretched cells whose borders
/// are the fissures (`ridges` across the tile, `plates` along it), their
/// outlines wobbled by noise, flattened into plates, with fine grain on top.
/// Normals, ambient occlusion, colour and roughness all derive from that one
/// height, so they agree.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BarkRecipe {
    /// World size of the tile, in metres.
    pub tile_size: f32,
    /// Texels along each side of the tile.
    pub size: u32,
    /// Bark ridges across the tile (around the branch).
    pub ridges: u32,
    /// Plates each ridge breaks into along the tile.
    pub plates: u32,
    /// Fissure width, as the fraction of a cell's border distance kept
    /// sunken, in `(0, 0.5)`.
    pub fissure: f32,
    /// Fissure depth, in metres.
    pub relief: f32,
    /// Fine grain on the plates, as a fraction of the plate height.
    pub grain: f32,
    /// Linear colour deep in the fissures.
    pub dark: [f32; 3],
    /// Linear colour on the plate tops.
    pub light: [f32; 3],
    /// Roughness on the plate tops and in the fissures.
    pub roughness: [f32; 2],
    /// Seed for every noise in the recipe.
    pub seed: u64,
}

impl Default for BarkRecipe {
    /// Mature pedunculate oak: grey-brown ridges broken into oblong plates
    /// between deep, dark fissures.
    fn default() -> Self {
        Self {
            tile_size: 0.5,
            size: 512,
            ridges: 7,
            plates: 2,
            fissure: 0.32,
            relief: 0.03,
            grain: 0.18,
            dark: [0.028, 0.022, 0.018],
            light: [0.21, 0.19, 0.16],
            roughness: [0.75, 0.95],
            seed: 1,
        }
    }
}

/// A bark material set: its height and the maps ready to pack.
#[derive(Clone, Debug)]
pub struct BarkSet {
    /// Height in `[0, 1]`, one tile, wrapping; 1 is the plate tops.
    pub height: Raster,
    /// Base colour, normals, roughness and occlusion.
    pub maps: MaterialMaps,
    /// Content fingerprint of the height program.
    pub fingerprint: Fingerprint,
}

impl BarkRecipe {
    fn validate(&self) -> Result<(), TextureError> {
        let bad = |ok: bool, name| {
            if ok {
                Ok(())
            } else {
                Err(TextureError::Params { name })
            }
        };
        bad(
            self.tile_size.is_finite() && self.tile_size > 0.0,
            "tile_size",
        )?;
        bad((8..=8192).contains(&self.size), "size")?;
        bad((1..=256).contains(&self.ridges), "ridges")?;
        bad((1..=256).contains(&self.plates), "plates")?;
        bad(self.fissure > 0.0 && self.fissure < 0.5, "fissure")?;
        bad(self.relief.is_finite() && self.relief >= 0.0, "relief")?;
        bad((0.0..1.0).contains(&self.grain), "grain")?;
        bad(
            self.roughness.iter().all(|r| (0.0..=1.0).contains(r)),
            "roughness",
        )
    }
}

/// Generates a tileable bark set.
///
/// # Errors
///
/// [`TextureError::Params`] for an invalid recipe, or a dapple error.
pub fn bark(recipe: &BarkRecipe) -> Result<BarkSet, TextureError> {
    recipe.validate()?;
    let torus = Domain::periodic(1, 1).ok_or(TextureError::Params { name: "domain" })?;
    let seed = recipe.seed;
    #[expect(clippy::cast_precision_loss, reason = "cell counts are small")]
    let cells = [recipe.ridges as f32, recipe.plates as f32];
    let mut b = ProgramBuilder::new();
    // Border distance of stretched cells: fissures run along the branch.
    let border = b.add(Op::Cellular {
        domain: torus,
        frequency: cells,
        jitter: 1.0,
        seed,
        output: CellOutput::Border,
    })?;
    // A repeating domain needs whole cycles per tile.
    #[expect(clippy::cast_precision_loss, reason = "cell counts are small")]
    let wobble_frequency = [
        recipe.ridges.div_ceil(2).max(1) as f32,
        (recipe.plates * 3) as f32,
    ];
    let wobble = |b: &mut ProgramBuilder, salt: u64| {
        b.add(Op::Fractal {
            basis: Basis::Gradient,
            domain: torus,
            frequency: wobble_frequency,
            seed: seed.wrapping_add(salt),
            params: FractalParams::default(),
        })
    };
    let dx = wobble(&mut b, 1)?;
    let dy = wobble(&mut b, 2)?;
    let wavy = b.add(Op::Warp {
        input: border,
        dx,
        dy,
        amount: 0.35 / cells[0],
    })?;
    // Border distance is in cell units: sunken within `fissure` of a
    // border, flat plate tops beyond.
    let plates = b.add(Op::Clamp {
        input: wavy,
        min: 0.0,
        max: recipe.fissure,
    })?;
    let plates = b.add(Op::Remap {
        input: plates,
        from: [0.0, recipe.fissure],
        to: [0.0, 1.0 - recipe.grain],
    })?;
    let grain = b.add(Op::Fractal {
        basis: Basis::Gradient,
        domain: torus,
        frequency: [cells[0] * 4.0, cells[1] * 4.0],
        seed: seed.wrapping_add(3),
        params: FractalParams {
            octaves: 4,
            ..FractalParams::default()
        },
    })?;
    let grain = b.add(Op::Remap {
        input: grain,
        from: [-1.0, 1.0],
        to: [0.0, recipe.grain],
    })?;
    let height = b.add(Op::Add {
        a: plates,
        b: grain,
    })?;
    let program = b.finish(height)?;
    let fingerprint = program.fingerprint();
    let height = realize(
        &program,
        Realization::period(torus, recipe.size, recipe.size)?,
    )?;

    // One domain unit is one tile; heights of 1 are `relief` metres deep.
    let scale = recipe.relief / recipe.tile_size;
    let normals = HeightToNormal { scale }.apply(&height)?;
    let ao = AmbientOcclusion {
        radius: 0.06 / cells[0],
        directions: 12,
        scale,
    }
    .apply(&height)?;
    let (dark, light) = (recipe.dark, recipe.light);
    let (smooth, rough) = (recipe.roughness[0], recipe.roughness[1]);
    let values = height.values();
    let base_color: Vec<f32> = values
        .iter()
        .flat_map(|&h| {
            let t = h.clamp(0.0, 1.0);
            [0, 1, 2].map(|c| dark[c] + (light[c] - dark[c]) * t)
        })
        .collect();
    let roughness: Vec<f32> = values
        .iter()
        .map(|&h| rough + (smooth - rough) * h.clamp(0.0, 1.0))
        .collect();
    let image = |channels, values| {
        Image::new(recipe.size, recipe.size, channels, Edge::Wrap, values)
            .map_err(TextureError::Encode)
    };
    let maps = MaterialMaps {
        base_color: Some(image(3, base_color)?),
        normal: Some(Image::from(&normals)),
        specular_roughness: Some(image(1, roughness)?),
        occlusion: Some(Image::from(&ao)),
        ..MaterialMaps::default()
    };
    Ok(BarkSet {
        height,
        maps,
        fingerprint,
    })
}
