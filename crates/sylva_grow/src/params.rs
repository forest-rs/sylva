// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Parameters of the hierarchical generator.

use alloc::vec::Vec;

use crate::Curve;

/// A complete hierarchical growth description: one trunk and a list of
/// branching levels below it.
///
/// `levels[0]` grows on the trunk (branch order 1), `levels[1]` on those
/// branches (order 2), and so on. Lengths are metres, angles radians,
/// coordinates tree space with `+Z` up and the trunk base at the origin.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Hierarchy {
    /// The root stem.
    pub trunk: Trunk,
    /// Branching levels, outermost last.
    pub levels: Vec<Level>,
    /// Optional crown envelope that prunes branches growing out of it.
    pub envelope: Option<Envelope>,
    /// Pipe-model radii applied after growth.
    pub radii: Radii,
    /// Target centerline segment length. Every branch has at least two
    /// segments, so short twigs are sampled more finely.
    pub segment_length: f32,
}

/// The root stem.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Trunk {
    /// Nominal length.
    pub length: f32,
    /// Keyed length variation as a fraction of `length` (`0.1` is ±10%).
    pub length_jitter: f32,
    /// Largest tilt from vertical at the base, in a keyed direction.
    pub lean: f32,
    /// Centerline shaping.
    pub shape: Shape,
}

impl Default for Trunk {
    fn default() -> Self {
        Self {
            length: 10.0,
            length_jitter: 0.0,
            lean: 0.0,
            shape: Shape::default(),
        }
    }
}

/// How a centerline bends as it grows. Shared by the trunk and every level.
///
/// Rates are radians per metre of branch length, so shapes do not depend on
/// [`Hierarchy::segment_length`] beyond sampling.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Shape {
    /// Total bend over the first half of the branch, upward when positive
    /// (toward `+Z`) and downward when negative.
    pub curve: f32,
    /// Total bend over the second half, in the same sense as `curve`. The
    /// opposite sign of `curve` gives an S-shaped branch.
    pub curve_back: f32,
    /// Amplitude of smooth keyed wandering, in radians per metre. Sinuous
    /// limbs come from this.
    pub gnarl: f32,
    /// Distance between independent gnarl values along the branch, in
    /// metres; larger values give slower, broader waves.
    pub gnarl_wavelength: f32,
    /// Gravitropism: turn toward `+Z` at this rate.
    pub up: f32,
    /// Gravity sag: turn toward `-Z` at a rate growing linearly from zero at
    /// the base to this value at the tip.
    pub sag: f32,
    /// Phototropism: turn toward the horizontal direction away from the trunk
    /// axis at this rate, spreading the crown outward.
    pub light: f32,
    /// Angle between consecutive internodes, in radians. Many trees grow
    /// sympodially: each season's shoot ends and a side bud takes over, so
    /// twigs zig-zag and old limbs are crooked rather than smoothly curved.
    /// Kinks zig-zag around the branch's smooth heading and never accumulate
    /// into it, so they add crookedness without steering the branch. Zero
    /// disables kinks.
    pub kink: f32,
    /// Internode length between kinks, in metres. The centerline gets a node
    /// at every kink, so this also bounds the node spacing.
    pub kink_interval: f32,
    /// Irregularity of kinks in `[0, 1]`. At 0 kinks alternate sides in one
    /// plane with a fixed angle (a clean zig-zag); larger values randomize
    /// the angle and swing the side, which reads as crooked growth.
    pub kink_jitter: f32,
}

/// One branching level: children grown along every branch of the level above.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Level {
    /// How many children each parent gets.
    pub count: Count,
    /// Parameter range `[start, end]` along the parent where children
    /// attach, as fractions of the parent's arc length.
    pub span: [f32; 2],
    /// Angular arrangement around the parent.
    pub arrangement: Arrangement,
    /// Keyed variation of each child's azimuth around the parent, radians.
    pub roll_jitter: f32,
    /// Keyed variation of attachment position, as a fraction of the spacing
    /// between attachment nodes.
    pub position_jitter: f32,
    /// Angle between parent tangent and child direction, over the parent
    /// parameter `t`. Small angles hug the parent; `pi / 2` is horizontal on a
    /// vertical parent.
    pub angle: Curve,
    /// Keyed angle variation, radians (±).
    pub angle_jitter: f32,
    /// Child length as a fraction of the parent's length, over `t`.
    pub length: Curve,
    /// Keyed length variation as a fraction (`0.2` is ±20%).
    pub length_jitter: f32,
    /// Crown balance among siblings, in `[0, 1]`. Jittered lengths and
    /// azimuths can leave a crown lopsided; this shortens the children of
    /// one parent that reach toward their combined horizontal lean and
    /// lengthens those opposite, in proportion to the imbalance. It matters
    /// most for scaffold limbs on the trunk. 0 leaves lengths unchanged.
    pub balance: f32,
    /// Centerline shaping of these children.
    pub shape: Shape,
    /// Optional foliage sites along these children.
    pub sites: Option<Sites>,
}

impl Default for Level {
    fn default() -> Self {
        Self {
            count: Count::Fixed(0),
            span: [0.0, 1.0],
            arrangement: Arrangement::default(),
            roll_jitter: 0.0,
            position_jitter: 0.0,
            angle: Curve::constant(core::f32::consts::FRAC_PI_4),
            angle_jitter: 0.0,
            length: Curve::constant(0.5),
            length_jitter: 0.0,
            balance: 0.0,
            shape: Shape::default(),
            sites: None,
        }
    }
}

/// Children per parent.
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Count {
    /// Exactly this many.
    Fixed(u32),
    /// Between `min` and `max` inclusive, keyed per parent, so each tree
    /// (and each parent) draws its own count.
    Range {
        /// Fewest children.
        min: u32,
        /// Most children.
        max: u32,
    },
    /// This many per metre of the parent's span; the fractional part rounds
    /// up with keyed probability, so the expected count is exact.
    PerMetre(f32),
}

/// Angular arrangement of children around their parent (phyllotaxis).
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Arrangement {
    /// One child per node, each turned by `divergence` radians from the last.
    /// The golden angle (about 2.39996) is the common spiral.
    Spiral {
        /// Azimuth step between consecutive children.
        divergence: f32,
    },
    /// One child per node, alternating sides (distichous).
    Alternate,
    /// Two children per node on opposite sides, each pair turned a quarter
    /// turn from the last (decussate).
    Opposite,
    /// `per_node` children evenly spaced at each node, successive whorls
    /// offset by half the spacing.
    Whorled {
        /// Children per whorl.
        per_node: u32,
    },
}

impl Default for Arrangement {
    fn default() -> Self {
        Self::Spiral {
            divergence: GOLDEN_ANGLE,
        }
    }
}

/// The golden angle in radians, `pi * (3 - sqrt 5)`.
pub const GOLDEN_ANGLE: f32 = 2.399_963_2;

/// Foliage sites along a level's branches.
///
/// Each site's frame points away from its branch at `angle` from the branch
/// axis, rolling by the golden angle from site to site. Many trees crowd
/// their leaves at the shoot ends (oak most visibly), which `tip_cluster`
/// models as a whorl of extra sites over the last `cluster_span` of each
/// branch.
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Sites {
    /// Sites per metre of the span; fractional counts round with keyed
    /// probability.
    pub per_metre: f32,
    /// Parameter range `[start, end]` along the branch.
    pub span: [f32; 2],
    /// Caller-defined site kind (leaf, fruit, ...), copied to each site.
    pub kind: u32,
    /// Angle between the branch axis and each site's outward direction, in
    /// radians, in `(0, pi)`.
    pub angle: f32,
    /// Extra sites crowded in a whorl at each branch tip.
    pub tip_cluster: u32,
    /// Fraction of the branch, ending at the tip, that the whorl occupies.
    pub cluster_span: f32,
}

impl Default for Sites {
    fn default() -> Self {
        Self {
            per_metre: 0.0,
            span: [0.0, 1.0],
            kind: 0,
            angle: 0.8,
            tip_cluster: 0,
            cluster_span: 0.1,
        }
    }
}

/// A crown envelope around the trunk axis.
///
/// A point at height `z` and horizontal distance `d` from the trunk axis is
/// inside when `u = (z - base) / height` lies in `[0, 1]` and
/// `d <= radius * profile(u)`. Points below ground (`z < 0`) are always
/// outside.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Envelope {
    /// Height of the envelope's bottom.
    pub base: f32,
    /// Height of the envelope from `base` to its top.
    pub height: f32,
    /// Largest horizontal radius.
    pub radius: f32,
    /// Radius fraction over the normalized height `u`.
    pub profile: Curve,
    /// First level (index into [`Hierarchy::levels`]) the envelope prunes;
    /// the trunk and earlier levels are never pruned.
    pub from_level: u32,
    /// A pruned branch shorter than this fraction of its intended length is
    /// removed instead of truncated.
    pub min_fraction: f32,
}

/// Pipe-model radius settings.
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Radii {
    /// Radius at every branch tip, metres.
    pub tip_radius: f32,
    /// Pipe-model exponent (`r_parent^e = sum r_child^e`).
    pub exponent: f32,
    /// Unmodeled shoots per metre of centerline, each carrying a tip's flow
    /// (see `sylva_skeleton::passes::PipeModel::shoots_per_metre`).
    pub shoots_per_metre: f32,
    /// Relative radius growth per metre below a branch's lowest child, so
    /// a bare bole tapers instead of standing as a column (see
    /// `sylva_skeleton::passes::PipeModel::bole_taper`).
    pub bole_taper: f32,
}

impl Default for Radii {
    fn default() -> Self {
        Self {
            tip_radius: 0.004,
            exponent: 2.3,
            shoots_per_metre: 0.0,
            bole_taper: 0.0,
        }
    }
}
