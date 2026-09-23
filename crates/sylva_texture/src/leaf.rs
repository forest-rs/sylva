// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leaf material sets, in the frame of a `sylva_foliage` leaf shape.

use alloc::vec::Vec;

use alloc::string::ToString;
use dapple_encode::{Image, MaterialMaps};
use dapple_field::program::{Fingerprint, Op, ProgramBuilder};
use dapple_field::raster::Region;
use dapple_field::{Basis, Domain, FractalParams};
use dapple_imaging::imaging::Painter;
use dapple_imaging::imaging::kurbo::BezPath;
use dapple_imaging::imaging::peniko::Color;
use dapple_imaging::imaging::record::Scene;
use dapple_imaging::rasterize;
use dapple_raster::{Edge, HeightToNormal, Raster, RasterOp, Realization, realize};
use glam::Vec2;
use sylva_foliage::LeafShape;

use crate::TextureError;

/// A broadleaf blade's colour, veins, relief and translucency.
///
/// Textures cover the frame of [`LeafShape::uv`], the same frame as the leaf
/// mesh, card and mask, so every map lines up with the blade. Opacity is the
/// leaf's own coverage mask. Veins follow the shape: a midrib and one
/// secondary vein into each lobe, angled toward the tip.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LeafRecipe {
    /// The blade; its outline and lobes place the veins and the opacity.
    pub shape: LeafShape,
    /// Texels along each side of the square texture.
    pub size: u32,
    /// Linear blade colour (upper surface).
    pub green: [f32; 3],
    /// Linear vein colour.
    pub vein: [f32; 3],
    /// Linear colour of light transmitted through the blade: OpenPBR
    /// `subsurface_color` on a thin-walled material, glTF's
    /// diffuse-transmission colour.
    pub translucent: [f32; 3],
    /// Share of light scattered through the blade, in `[0, 1]`: OpenPBR
    /// `subsurface_weight` on a thin-walled material, glTF's
    /// diffuse-transmission factor. Veins transmit half as much.
    pub translucency: f32,
    /// Midrib width, as a fraction of the blade length; secondary veins are
    /// half as wide.
    pub vein_width: f32,
    /// Vein relief above the blade, in metres.
    pub vein_relief: f32,
    /// Blade colour variation (mottling), as a fraction.
    pub mottle: f32,
    /// Blade roughness.
    pub roughness: f32,
    /// Seed for the mottling.
    pub seed: u64,
}

impl Default for LeafRecipe {
    /// A summer pedunculate-oak leaf.
    fn default() -> Self {
        Self {
            shape: LeafShape::default(),
            size: 256,
            green: [0.045, 0.11, 0.022],
            vein: [0.12, 0.2, 0.06],
            translucent: [0.2, 0.36, 0.05],
            translucency: 0.4,
            vein_width: 0.012,
            vein_relief: 0.0004,
            mottle: 0.15,
            roughness: 0.55,
            seed: 1,
        }
    }
}

/// A leaf material set.
#[derive(Clone, Debug)]
pub struct LeafSet {
    /// Opacity (the blade's coverage), base colour, normals, roughness, and
    /// thin-walled translucency as `subsurface_weight` and
    /// `subsurface_color`.
    pub maps: MaterialMaps,
    /// Content fingerprint of the mottling program.
    pub fingerprint: Fingerprint,
}

impl LeafRecipe {
    fn validate(&self) -> Result<(), TextureError> {
        let bad = |ok: bool, name| {
            if ok {
                Ok(())
            } else {
                Err(TextureError::Params { name })
            }
        };
        self.shape
            .validate()
            .map_err(|_| TextureError::Params { name: "shape" })?;
        bad((8..=8192).contains(&self.size), "size")?;
        bad((0.0..=1.0).contains(&self.translucency), "translucency")?;
        bad(
            self.vein_width.is_finite() && self.vein_width > 0.0 && self.vein_width < 0.2,
            "vein_width",
        )?;
        bad(
            self.vein_relief.is_finite() && self.vein_relief >= 0.0,
            "vein_relief",
        )?;
        bad((0.0..1.0).contains(&self.mottle), "mottle")?;
        bad((0.0..=1.0).contains(&self.roughness), "roughness")
    }

    /// The vein centerlines in leaf space, midrib first.
    fn veins(&self) -> Vec<(Vec2, Vec2, f32)> {
        let shape = &self.shape;
        let l = shape.length;
        let w = self.vein_width * l;
        let mut veins = alloc::vec![(Vec2::ZERO, Vec2::new(0.0, l), w)];
        #[expect(clippy::cast_precision_loss, reason = "lobe counts are small")]
        let lobes = shape.lobes as f32;
        for k in 1..=shape.lobes {
            #[expect(clippy::cast_precision_loss, reason = "lobe counts are small")]
            let t = k as f32 / (lobes + 0.5);
            let start = Vec2::new(0.0, (t - 0.12).max(0.02) * l);
            let reach = 0.85 * shape.half_width(t);
            for side in [-1.0, 1.0] {
                let end = shape.skewed(Vec2::new(side * reach, t * l));
                veins.push((start, end, 0.5 * w));
            }
        }
        veins
    }
}

/// Distance from `p` to the segment `a`..`b`.
fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(f32::MIN_POSITIVE)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// The texture region of `shape`: its [`LeafShape::uv`] frame, in leaf-plane
/// metres, on a `size x size` grid.
fn frame(shape: &LeafShape, size: u32) -> Result<Realization, TextureError> {
    let half = shape.max_half_width();
    Ok(Realization::region(
        Region {
            origin: Vec2::new(-half, 0.0),
            size: Vec2::new(2.0 * half, shape.length),
        },
        size,
        size,
    )?)
}

/// Rasterizes `shape`'s coverage mask on a `size x size` grid over its
/// [`LeafShape::uv`] frame, row 0 at the blade base.
///
/// Each texel holds the fraction of its area inside the outline, in steps
/// of 1/255, from `dapple_imaging` on the same grid as the leaf set's other
/// maps. The outline is [`LeafShape::outline_at`] with four stations per
/// texel along the midrib, fine enough that its chords follow the lobes.
///
/// # Errors
///
/// [`TextureError::Params`] for a zero `size`, or a dapple error.
pub fn leaf_mask(shape: &LeafShape, size: u32) -> Result<Raster, TextureError> {
    if size == 0 {
        return Err(TextureError::Params { name: "size" });
    }
    let outline = shape.outline_at(size.saturating_mul(4).max(shape.stations));
    let mut path = BezPath::new();
    for (i, p) in outline.iter().enumerate() {
        let p = (f64::from(p.x), f64::from(p.y));
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.close_path();
    let mut scene = Scene::new();
    Painter::new(&mut scene).fill(&path, Color::WHITE).draw();
    rasterize(&scene, frame(shape, size)?).map_err(|e| TextureError::Mask(e.to_string()))
}

/// Generates a leaf set for `recipe.shape`.
///
/// # Errors
///
/// [`TextureError::Params`] for an invalid recipe, or a dapple error.
pub fn leaf(recipe: &LeafRecipe) -> Result<LeafSet, TextureError> {
    recipe.validate()?;
    let shape = &recipe.shape;
    let n = recipe.size;
    let half = shape.max_half_width();
    let (width, length) = (2.0 * half, shape.length);
    #[expect(clippy::cast_precision_loss, reason = "texture sizes are small")]
    let texel = Vec2::new(width / n as f32, length / n as f32);
    let origin = Vec2::new(-half, 0.0);

    // Mottling: band-limited noise over the blade, in metres.
    let mut b = ProgramBuilder::new();
    let mottle = b.add(Op::Fractal {
        basis: Basis::Gradient,
        domain: Domain::Plane,
        frequency: [60.0, 60.0],
        seed: recipe.seed,
        params: FractalParams {
            octaves: 3,
            ..FractalParams::default()
        },
    })?;
    let program = b.finish(mottle)?;
    let fingerprint = program.fingerprint();
    let mottle = realize(&program, frame(shape, n)?)?;

    let veins = recipe.veins();
    let texels = (n * n) as usize;
    let mut vein = Vec::with_capacity(texels);
    for row in 0..n {
        for col in 0..n {
            #[expect(clippy::cast_precision_loss, reason = "texture sizes are small")]
            let p = origin + Vec2::new((col as f32 + 0.5) * texel.x, (row as f32 + 0.5) * texel.y);
            let v = veins
                .iter()
                .map(|&(a, b, w)| {
                    let d = segment_distance(p, a, b) / (0.5 * w);
                    libm::expf(-d * d)
                })
                .fold(0.0_f32, f32::max);
            vein.push(v);
        }
    }
    let relief = Raster::from_values(n, n, origin, texel, Edge::Clamp, vein.clone())?;
    let normals = HeightToNormal {
        scale: recipe.vein_relief,
    }
    .apply(&relief)?;

    let opacity = leaf_mask(shape, n)?.values().to_vec();
    let mut base_color = Vec::with_capacity(texels * 3);
    let mut subsurface_color = Vec::with_capacity(texels * 3);
    let mut subsurface_weight = Vec::with_capacity(texels);
    for (i, &v) in vein.iter().enumerate() {
        let m = 1.0 + recipe.mottle * mottle.values()[i];
        for c in 0..3 {
            base_color
                .push((recipe.green[c] * m + (recipe.vein[c] - recipe.green[c]) * v).max(0.0));
            subsurface_color.push((recipe.translucent[c] * m).clamp(0.0, 1.0));
        }
        // Veins are thicker and block more transmitted light.
        subsurface_weight.push((recipe.translucency * (1.0 - 0.5 * v)).clamp(0.0, 1.0));
    }
    let image = |channels, values| {
        Image::new(n, n, channels, Edge::Clamp, values).map_err(TextureError::Encode)
    };
    let maps = MaterialMaps {
        base_color: Some(image(3, base_color)?),
        opacity: Some(image(1, opacity)?),
        normal: Some(Image::from(&normals)),
        specular_roughness: Some(image(1, alloc::vec![recipe.roughness; texels])?),
        subsurface_weight: Some(image(1, subsurface_weight)?),
        subsurface_color: Some(image(3, subsurface_color)?),
        ..MaterialMaps::default()
    };
    Ok(LeafSet { maps, fingerprint })
}
