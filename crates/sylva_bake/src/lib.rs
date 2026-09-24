// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Deterministic CPU baking of geometry into card textures.
//!
//! Real-time trees replace distant geometry with textured cards: a twig and
//! its leaves become one quad, a whole tree becomes an impostor. [`bake`]
//! renders triangle meshes into such a card with an orthographic view
//! ([`CardView`]) and returns its material maps: base colour, coverage
//! opacity, normals in the card's frame, and depth.
//!
//! - **Frame.** The card spans `[-half.x, half.x] x [-half.y, half.y]` along
//!   `right` and `up`; texel row 0 is the card's bottom edge (`v = 0`, the
//!   sylva and glTF row convention), and normals are expressed with `+X`
//!   along `right`, `+Y` along `up` and `+Z` toward the viewer, the frame of
//!   a dapple normal map.
//! - **Sampling.** Every texel takes `samples x samples` stratified samples.
//!   Each keeps the nearest fragment that passes its material's alpha test;
//!   a texel's opacity is its covered fraction, and its colour, normal and
//!   depth average its covered samples.
//! - **Two-sided.** Faces are drawn from both sides, and a normal facing
//!   away from the viewer is flipped, as foliage is shaded.
//! - **Determinism.** Rasterization uses edge functions in a fixed order and
//!   no fused arithmetic, so equal inputs give equal bytes everywhere.
//!
//! [`Baked::maps`] packs the result as [`dapple_encode::MaterialMaps`], ready
//! for coverage-preserving mips.
//!
//! # Example
//! ```rust
//! use sylva_bake::{BakeMaterial, BakeMesh, BakeSettings, CardView, bake};
//! use sylva_bake::glam::{Affine3A, Vec2, Vec3};
//!
//! // A unit quad facing the viewer, filling the card.
//! let positions = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [-1.0, 1.0, 0.0]];
//! let normals = [[0.0, 0.0, 1.0]; 4];
//! let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
//! let mesh = BakeMesh {
//!     positions: &positions,
//!     normals: &normals,
//!     uvs: &uvs,
//!     indices: &[0, 1, 2, 0, 2, 3],
//!     transform: Affine3A::IDENTITY,
//!     material: BakeMaterial::default(),
//! };
//! let view = CardView::facing(Vec3::ZERO, Vec3::X, Vec3::Y, Vec2::splat(1.0), 1.0);
//! let baked = bake(&[mesh], &view, &BakeSettings { size: [16, 16], samples: 2 })?;
//! assert_eq!(baked.report.covered_texels, 256);
//! # Ok::<(), sylva_bake::BakeError>(())
//! ```

#![no_std]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use dapple_encode::{EncodeError, Image, MaterialMaps};
use dapple_raster::Edge;
pub use glam;
use glam::{Affine3A, Vec2, Vec3};

/// How one mesh is coloured and cut.
#[derive(Copy, Clone, Debug)]
pub struct BakeMaterial<'a> {
    /// Linear base colour, 3 channels, sampled by UV (row 0 at `v = 0`,
    /// nearest texel, clamped); `None` uses `color` alone.
    pub base_color: Option<&'a Image>,
    /// Opacity, 1 channel, sampled like `base_color`; `None` is opaque.
    pub opacity: Option<&'a Image>,
    /// Colour multiplier, or the colour when there is no texture.
    pub color: [f32; 3],
    /// Samples with opacity below this are discarded.
    pub cutoff: f32,
}

impl Default for BakeMaterial<'_> {
    fn default() -> Self {
        Self {
            base_color: None,
            opacity: None,
            color: [1.0; 3],
            cutoff: 0.5,
        }
    }
}

/// One triangle mesh placed in the world.
#[derive(Copy, Clone, Debug)]
pub struct BakeMesh<'a> {
    /// Vertex positions in the mesh's own space.
    pub positions: &'a [[f32; 3]],
    /// Vertex normals, parallel to `positions`.
    pub normals: &'a [[f32; 3]],
    /// Vertex UVs, parallel to `positions`.
    pub uvs: &'a [[f32; 2]],
    /// Triangle indices.
    pub indices: &'a [u32],
    /// Placement of the mesh in the world.
    pub transform: Affine3A,
    /// Its material.
    pub material: BakeMaterial<'a>,
}

/// An orthographic card view.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CardView {
    /// World position of the card's centre.
    pub center: Vec3,
    /// Unit direction of the card's `+X` (texel columns).
    pub right: Vec3,
    /// Unit direction of the card's `+Y` (texel rows), perpendicular to
    /// `right`.
    pub up: Vec3,
    /// Half extent of the card along `right` and `up`, in metres.
    pub half: Vec2,
    /// Half depth of the baked volume along the view axis, in metres;
    /// depth maps to `[0, 1]` from its far side to its near side.
    pub half_depth: f32,
}

impl CardView {
    /// A card at `center` with the given axes, extent and depth range.
    #[must_use]
    pub fn facing(center: Vec3, right: Vec3, up: Vec3, half: Vec2, half_depth: f32) -> Self {
        Self {
            center,
            right,
            up,
            half,
            half_depth,
        }
    }

    /// Unit direction toward the viewer, `right x up`.
    #[must_use]
    pub fn toward(&self) -> Vec3 {
        self.right.cross(self.up)
    }

    fn validate(&self) -> Result<(), BakeError> {
        let unit = |v: Vec3| v.is_finite() && (v.length() - 1.0).abs() < 1e-3;
        if !(unit(self.right) && unit(self.up) && self.right.dot(self.up).abs() < 1e-3) {
            return Err(BakeError::View);
        }
        if !(self.center.is_finite()
            && self.half.is_finite()
            && self.half.x > 0.0
            && self.half.y > 0.0
            && self.half_depth.is_finite()
            && self.half_depth > 0.0)
        {
            return Err(BakeError::View);
        }
        Ok(())
    }
}

/// Output size and sampling.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BakeSettings {
    /// Texels across and up the card.
    pub size: [u32; 2],
    /// Samples per texel along each axis.
    pub samples: u32,
}

impl Default for BakeSettings {
    fn default() -> Self {
        Self {
            size: [256, 256],
            samples: 4,
        }
    }
}

/// Deterministic counts describing one bake.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct BakeReport {
    /// Triangles submitted.
    pub triangles: u64,
    /// Samples that received a fragment.
    pub covered_samples: u64,
    /// Texels with any coverage.
    pub covered_texels: u64,
    /// Fragments discarded by alpha tests.
    pub alpha_discards: u64,
}

/// A baked card.
#[derive(Clone, Debug)]
pub struct Baked {
    /// Linear base colour, 3 channels.
    pub base_color: Image,
    /// Covered fraction, 1 channel.
    pub opacity: Image,
    /// Unit normals in the card frame, 3 channels.
    pub normal: Image,
    /// Depth, 1 channel: 0 at the far side of the volume, 1 at the near
    /// side; 0 where uncovered.
    pub depth: Image,
    /// Deterministic counts.
    pub report: BakeReport,
}

impl Baked {
    /// Rescales opacity so an alpha test at `cutoff` keeps as much of the
    /// card as its geometry covers (Castaño, *Computing Alpha Mipmaps*).
    ///
    /// Thin features (needles, twigs, distant leaves) cover a fraction of
    /// each texel they cross, so their opacity sits below a typical cutoff
    /// and an alpha test erases them. This finds the scale `s` for which the
    /// share of texels with `s * opacity >= cutoff` equals the mean
    /// opacity, then stores `min(1, s * opacity)`. Opacity only grows, and
    /// uncovered texels stay 0.
    ///
    /// # Panics
    ///
    /// Never for a baked card, whose opacity is a valid one-channel image.
    pub fn preserve_coverage(&mut self, cutoff: f32) {
        let values = self.opacity.values();
        if values.is_empty() || !(cutoff > 0.0 && cutoff < 1.0) {
            return;
        }
        #[expect(clippy::cast_precision_loss, reason = "a mean over texels")]
        let n = values.len() as f32;
        let mean = values.iter().sum::<f32>() / n;
        #[expect(clippy::cast_precision_loss, reason = "a share of texels")]
        let passing =
            |scale: f32| values.iter().filter(|&&a| a * scale >= cutoff).count() as f32 / n;
        let (mut lo, mut hi) = (1.0_f32, 1.0 / cutoff.max(1e-3) * 16.0);
        if passing(lo) >= mean {
            return;
        }
        for _ in 0..24 {
            let mid = 0.5 * (lo + hi);
            if passing(mid) < mean {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let scaled: Vec<f32> = values.iter().map(|&a| (a * hi).min(1.0)).collect();
        self.opacity = Image::new(
            self.opacity.width(),
            self.opacity.height(),
            1,
            self.opacity.edge(),
            scaled,
        )
        .expect("the same shape as the baked opacity");
    }

    /// The card's maps for [`dapple_encode::pack`].
    #[must_use]
    pub fn maps(&self) -> MaterialMaps {
        MaterialMaps {
            base_color: Some(self.base_color.clone()),
            opacity: Some(self.opacity.clone()),
            normal: Some(self.normal.clone()),
            ..MaterialMaps::default()
        }
    }
}

/// Why a bake failed.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum BakeError {
    /// The view's axes are not orthonormal or its extents are not positive.
    View,
    /// Size or samples out of range.
    Settings,
    /// A mesh's attribute arrays disagree in length or an index is out of
    /// range.
    Mesh {
        /// Index of the offending mesh.
        mesh: usize,
    },
    /// An output image could not be built.
    Encode(EncodeError),
}

impl fmt::Display for BakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::View => f.write_str("card view axes or extents are invalid"),
            Self::Settings => f.write_str("bake size or sample count is out of range"),
            Self::Mesh { mesh } => write!(f, "bake mesh {mesh} is malformed"),
            Self::Encode(error) => write!(f, "bake output: {error}"),
        }
    }
}

impl core::error::Error for BakeError {}

/// Nearest-texel sample of `image` at `uv`, clamped.
fn sample(image: &Image, uv: Vec2) -> &[f32] {
    let clamp = |t: f32, n: u32| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "a clamped unit value scaled to a texel index"
        )]
        let i = (t.clamp(0.0, 1.0) * n as f32) as u32;
        i.min(n - 1)
    };
    image.texel(clamp(uv.x, image.width()), clamp(uv.y, image.height()))
}

/// The nearest accepted fragment of one sample.
#[derive(Copy, Clone)]
struct Fragment {
    depth: f32,
    color: [f32; 3],
    normal: Vec3,
}

/// Bakes `meshes` into a card seen through `view`.
///
/// # Errors
///
/// [`BakeError::View`], [`BakeError::Settings`] or [`BakeError::Mesh`] for
/// invalid input.
pub fn bake(
    meshes: &[BakeMesh<'_>],
    view: &CardView,
    settings: &BakeSettings,
) -> Result<Baked, BakeError> {
    view.validate()?;
    let [w, h] = settings.size;
    let s = settings.samples;
    if !((1..=8192).contains(&w) && (1..=8192).contains(&h) && (1..=16).contains(&s)) {
        return Err(BakeError::Settings);
    }
    for (index, mesh) in meshes.iter().enumerate() {
        let n = mesh.positions.len();
        if mesh.normals.len() != n
            || mesh.uvs.len() != n
            || !mesh.indices.len().is_multiple_of(3)
            || mesh.indices.iter().any(|&i| i as usize >= n)
        {
            return Err(BakeError::Mesh { mesh: index });
        }
    }
    let (sw, sh) = (w * s, h * s);
    let mut samples: Vec<Option<Fragment>> = vec![None; (sw * sh) as usize];
    let toward = view.toward();
    let mut report = BakeReport::default();
    #[expect(clippy::cast_precision_loss, reason = "sample grids are small")]
    let (sample_w, sample_h) = (sw as f32, sh as f32);
    // Card coordinates to sample space: x in [0, sw), y in [0, sh).
    let to_sample = |p: Vec3| {
        let d = p - view.center;
        Vec3::new(
            (d.dot(view.right) / view.half.x * 0.5 + 0.5) * sample_w,
            (d.dot(view.up) / view.half.y * 0.5 + 0.5) * sample_h,
            d.dot(toward),
        )
    };
    for mesh in meshes {
        let normal_matrix = mesh.transform.matrix3.inverse().transpose();
        let world: Vec<Vec3> = mesh
            .positions
            .iter()
            .map(|p| mesh.transform.transform_point3(Vec3::from_array(*p)))
            .collect();
        let projected: Vec<Vec3> = world.iter().map(|&p| to_sample(p)).collect();
        let normals: Vec<Vec3> = mesh
            .normals
            .iter()
            .map(|n| (normal_matrix * Vec3::from_array(*n)).normalize_or_zero())
            .collect();
        for tri in mesh.indices.as_chunks::<3>().0 {
            report.triangles += 1;
            let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            raster_triangle(
                [projected[a], projected[b], projected[c]],
                [a, b, c],
                mesh,
                &normals,
                [sw, sh],
                view.half_depth,
                toward,
                view,
                &mut samples,
                &mut report,
            );
        }
    }
    resolve(&samples, [w, h], s, view.half_depth, &mut report)
}

/// Rasterizes one projected triangle into the sample grid.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "sample coordinates are clamped to the grid"
)]
fn raster_triangle(
    p: [Vec3; 3],
    ids: [usize; 3],
    mesh: &BakeMesh<'_>,
    normals: &[Vec3],
    [sw, sh]: [u32; 2],
    half_depth: f32,
    toward: Vec3,
    view: &CardView,
    samples: &mut [Option<Fragment>],
    report: &mut BakeReport,
) {
    let edge = |a: Vec3, b: Vec3, x: f32, y: f32| (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x);
    let area = edge(p[0], p[1], p[2].x, p[2].y);
    if area == 0.0 || !area.is_finite() {
        return;
    }
    let lo_x = p.iter().map(|v| v.x).fold(f32::INFINITY, f32::min);
    let hi_x = p.iter().map(|v| v.x).fold(f32::NEG_INFINITY, f32::max);
    let lo_y = p.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
    let hi_y = p.iter().map(|v| v.y).fold(f32::NEG_INFINITY, f32::max);
    let lo_x = libm::floorf(lo_x).max(0.0);
    let lo_y = libm::floorf(lo_y).max(0.0);
    let hi_x = libm::ceilf(hi_x).min(sw as f32);
    let hi_y = libm::ceilf(hi_y).min(sh as f32);
    if lo_x >= hi_x || lo_y >= hi_y {
        return;
    }
    let (x0, x1, y0, y1) = (lo_x as u32, hi_x as u32, lo_y as u32, hi_y as u32);
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w = [
                edge(p[1], p[2], px, py) / area,
                edge(p[2], p[0], px, py) / area,
                edge(p[0], p[1], px, py) / area,
            ];
            // Two-sided: accept either winding; a sample on a shared edge
            // belongs to the triangle whose weights are all non-negative.
            if w.iter().any(|&v| v < 0.0) {
                continue;
            }
            let depth = w[0] * p[0].z + w[1] * p[1].z + w[2] * p[2].z;
            if depth.abs() > half_depth {
                continue;
            }
            let slot = &mut samples[(y * sw + x) as usize];
            if slot.is_some_and(|f| f.depth >= depth) {
                continue;
            }
            let uv = w[0] * Vec2::from_array(mesh.uvs[ids[0]])
                + w[1] * Vec2::from_array(mesh.uvs[ids[1]])
                + w[2] * Vec2::from_array(mesh.uvs[ids[2]]);
            let material = &mesh.material;
            if let Some(opacity) = material.opacity
                && sample(opacity, uv)[0] < material.cutoff
            {
                report.alpha_discards += 1;
                continue;
            }
            let mut color = material.color;
            if let Some(base) = material.base_color {
                let texel = sample(base, uv);
                for (c, t) in color.iter_mut().zip(texel) {
                    *c *= t;
                }
            }
            let mut normal =
                (w[0] * normals[ids[0]] + w[1] * normals[ids[1]] + w[2] * normals[ids[2]])
                    .normalize_or(toward);
            if normal.dot(toward) < 0.0 {
                normal = -normal;
            }
            let card = Vec3::new(
                normal.dot(view.right),
                normal.dot(view.up),
                normal.dot(toward),
            );
            *slot = Some(Fragment {
                depth,
                color,
                normal: card,
            });
        }
    }
}

/// Averages each texel's covered samples.
fn resolve(
    samples: &[Option<Fragment>],
    [w, h]: [u32; 2],
    s: u32,
    half_depth: f32,
    report: &mut BakeReport,
) -> Result<Baked, BakeError> {
    let texels = (w * h) as usize;
    let mut color = Vec::with_capacity(texels * 3);
    let mut opacity = Vec::with_capacity(texels);
    let mut normal = Vec::with_capacity(texels * 3);
    let mut depth = Vec::with_capacity(texels);
    #[expect(clippy::cast_precision_loss, reason = "sample counts are small")]
    let per_texel = (s * s) as f32;
    for ty in 0..h {
        for tx in 0..w {
            let mut n = 0_u32;
            let mut c = [0.0_f32; 3];
            let mut nn = Vec3::ZERO;
            let mut d = 0.0_f32;
            for sy in 0..s {
                for sx in 0..s {
                    let index = ((ty * s + sy) * w * s + tx * s + sx) as usize;
                    if let Some(f) = samples[index] {
                        n += 1;
                        for (acc, v) in c.iter_mut().zip(f.color) {
                            *acc += v;
                        }
                        nn += f.normal;
                        d += f.depth;
                    }
                }
            }
            report.covered_samples += u64::from(n);
            if n == 0 {
                color.extend_from_slice(&[0.0; 3]);
                opacity.push(0.0);
                normal.extend_from_slice(&[0.0, 0.0, 1.0]);
                depth.push(0.0);
                continue;
            }
            report.covered_texels += 1;
            #[expect(clippy::cast_precision_loss, reason = "sample counts are small")]
            let count = n as f32;
            color.extend(c.map(|v| v / count));
            opacity.push(count / per_texel);
            normal.extend_from_slice(&nn.normalize_or(Vec3::Z).to_array());
            depth.push((0.5 + 0.5 * d / count / half_depth).clamp(0.0, 1.0));
        }
    }
    let image = |channels, values| {
        Image::new(w, h, channels, Edge::Clamp, values).map_err(BakeError::Encode)
    };
    Ok(Baked {
        base_color: image(3, color)?,
        opacity: image(1, opacity)?,
        normal: image(3, normal)?,
        depth: image(1, depth)?,
        report: *report,
    })
}

#[cfg(test)]
mod tests;
