// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Meshing parameters.

/// How many segments a branch ring gets.
///
/// A branch's base count is `round(circumference * segments_per_metre)`
/// clamped to `[min_segments, max_segments]`: a level-of-detail setting
/// trades this density for triangles. With `follow_taper`, rings toward the
/// tip halve the count as their own radius allows (keeping it even and at
/// least `min_segments`, one halving per ring), joined to the wider ring by a
/// band of triangles; otherwise every ring keeps the base count.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RingResolution {
    /// Fewest segments around any branch; at least 3.
    pub min_segments: u32,
    /// Most segments around any branch.
    pub max_segments: u32,
    /// Segments per metre of circumference.
    pub segments_per_metre: f32,
    /// Halve segment counts along a tapering branch.
    pub follow_taper: bool,
}

impl Default for RingResolution {
    fn default() -> Self {
        Self {
            min_segments: 4,
            max_segments: 24,
            segments_per_metre: 20.0,
            follow_taper: true,
        }
    }
}

/// Where rings are placed along a branch.
///
/// Rings are placed at skeleton nodes, skipping nodes where the centerline
/// is nearly straight: a node gets a ring once the centerline has turned by
/// `max_bend` radians or run `max_spacing` metres since the previous ring.
/// Collar and root-flare rings are added where the profile changes quickly.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Stations {
    /// Turning angle, in radians, that forces a ring.
    pub max_bend: f32,
    /// Longest centerline run, in metres, between rings.
    pub max_spacing: f32,
}

impl Default for Stations {
    fn default() -> Self {
        Self {
            max_bend: 0.15,
            max_spacing: 1.0,
        }
    }
}

/// Bark texture mapping.
///
/// U runs around a branch with an explicit seam at the frame normal; V runs
/// along it by arc length. A branch wraps the bark tile an integer number of
/// times, `max(1, round(base circumference / tile_size))`, so its seam is
/// continuous, and V advances one unit per `base circumference / repeats`
/// metres, so texels are square at the base and the density matches across
/// branches of every size. Each branch gets a keyed V offset so neighbouring
/// branches do not start the tile at the same place.
///
/// Sylva UVs index textures by row, as glTF and dapple do: `v = 0` is the
/// first texel row. U and V run right-handed about the outward normal, so a
/// dapple normal map (`+X` along U, `+Y` along V) needs no flip.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BarkMapping {
    /// World size of one bark tile, in metres.
    pub tile_size: f32,
    /// Seed for the per-branch V offset.
    pub seed: u64,
}

impl Default for BarkMapping {
    fn default() -> Self {
        Self {
            tile_size: 0.5,
            seed: 0,
        }
    }
}

/// How a child branch meets its parent.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Junction {
    /// The child's base sits inside the parent, as the skeleton places it (on
    /// the parent's centerline). The child flares into a collar near its base
    /// and its normals blend toward the parent's surface normal, hiding the
    /// intersection. This is the real-time standard: cheap, and valid at
    /// every level of detail.
    Embedded(Collar),
}

impl Default for Junction {
    fn default() -> Self {
        Self::Embedded(Collar::default())
    }
}

/// Shape of an embedded child's collar.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Collar {
    /// Collar length, as a multiple of the parent's radius at the attachment
    /// (so the collar reaches past the parent's surface).
    pub length: f32,
    /// Radius multiplier at the child's base, fading to 1 over the collar.
    pub flare: f32,
    /// How far collar normals blend toward the parent's surface normal at
    /// the base, in `[0, 1]`.
    pub normal_blend: f32,
    /// Extra rings placed along the collar.
    pub rings: u32,
}

impl Default for Collar {
    fn default() -> Self {
        Self {
            length: 1.6,
            flare: 1.35,
            normal_blend: 0.6,
            rings: 3,
        }
    }
}

/// Buttressed flare at the base of each root stem.
///
/// The radius grows by `flare` at the base and decays exponentially with
/// height; `lobes` angular buttresses of relative depth `lobe_depth` ride on
/// the flare.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RootFlare {
    /// Decay height, in metres.
    pub height: f32,
    /// Extra radius at the base, as a multiple of the stem radius.
    pub flare: f32,
    /// Number of buttress lobes; 0 for a round flare.
    pub lobes: u32,
    /// Lobe depth relative to the flared radius, in `[0, 1)`.
    pub lobe_depth: f32,
    /// Extra rings placed along the flare.
    pub rings: u32,
}

impl Default for RootFlare {
    fn default() -> Self {
        Self {
            height: 0.6,
            flare: 0.6,
            lobes: 5,
            lobe_depth: 0.25,
            rings: 5,
        }
    }
}

/// Parameters for [`mesh_skeleton`](crate::mesh_skeleton).
///
/// The default flares root stems with [`RootFlare::default`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MeshParams {
    /// Segments around each branch.
    pub rings: RingResolution,
    /// Ring placement along each branch.
    pub stations: Stations,
    /// Bark UV mapping.
    pub bark: BarkMapping,
    /// Child-parent junction strategy.
    pub junction: Junction,
    /// Flare at root stems; `None` for a plain cylinder base.
    pub root_flare: Option<RootFlare>,
}

impl Default for MeshParams {
    fn default() -> Self {
        Self {
            rings: RingResolution::default(),
            stations: Stations::default(),
            bark: BarkMapping::default(),
            junction: Junction::default(),
            root_flare: Some(RootFlare::default()),
        }
    }
}
