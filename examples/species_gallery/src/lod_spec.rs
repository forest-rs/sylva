// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A species' level-of-detail chain and its budget, as preset data
//! (`<species>_lod.ron`).

use serde::Deserialize;
use sylva_lod::{ClusterCards, LeafDetail, LodLevel, LodPolicy};
use sylva_mesh::{Collar, Junction, MeshParams, RingResolution, Stations, Weld};

/// A species' LOD chain: its levels, finest first, each with its budget.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct LodSpec {
    /// Levels from full detail down.
    pub(crate) levels: Vec<LevelSpec>,
}

/// One level and what it may cost.
#[derive(Copy, Clone, Debug, Deserialize)]
pub(crate) struct LevelSpec {
    /// Projected tree height, as a fraction of the screen, above which the
    /// level draws.
    pub(crate) screen_size: f32,
    /// Crossfade band into the next level, as a fraction of `screen_size`.
    pub(crate) crossfade: f32,
    /// Bark ring segments per metre of circumference, and their range.
    pub(crate) rings: (f32, u32, u32),
    /// Turning angle and run, in radians and metres, that force a ring.
    pub(crate) stations: (f32, f32),
    /// Branches thinner at their base lose their bark, in metres.
    pub(crate) min_branch_radius: f32,
    /// Fraction of the leaves outside clusters kept, area preserved.
    pub(crate) leaf_fraction: f32,
    /// Cluster cards: root order, baked exemplars, planes, leaf facing.
    pub(crate) clusters: Option<(u32, u32, u32, f32)>,
    /// How children meet their parents.
    #[serde(default)]
    pub(crate) junction: JunctionSpec,
    /// Texels across one baked exemplar's atlas cell.
    #[serde(default = "default_cell")]
    pub(crate) cell: u32,
    /// Most triangles the level may draw, instances counted.
    pub(crate) triangles: u64,
    /// Most bytes its instanced GLB may take, textures included.
    pub(crate) bytes: u64,
}

/// A level's junction strategy.
#[derive(Copy, Clone, Debug, Default, Deserialize)]
pub(crate) enum JunctionSpec {
    /// Children sit inside their parent behind a fillet collar.
    #[default]
    Embedded,
    /// Major forks are welded with a smoothed skin that continues the
    /// bark; forks the skin refuses stay embedded.
    Welded,
}

impl JunctionSpec {
    fn junction(self) -> Junction {
        match self {
            Self::Embedded => Junction::Embedded(Collar::default()),
            Self::Welded => Junction::Welded(Weld::default()),
        }
    }
}

fn default_cell() -> u32 {
    256
}

impl LodSpec {
    /// The chain as a [`LodPolicy`], with the default impostor.
    pub(crate) fn policy(&self) -> LodPolicy {
        let base = MeshParams::default();
        LodPolicy {
            levels: self
                .levels
                .iter()
                .map(|l| LodLevel {
                    screen_size: l.screen_size,
                    crossfade: l.crossfade,
                    rings: RingResolution {
                        segments_per_metre: l.rings.0,
                        min_segments: l.rings.1,
                        max_segments: l.rings.2,
                        ..base.rings
                    },
                    stations: Stations {
                        max_bend: l.stations.0,
                        max_spacing: l.stations.1,
                    },
                    min_branch_radius: l.min_branch_radius,
                    leaf_fraction: l.leaf_fraction,
                    leaf_detail: LeafDetail::Card,
                    junction: Some(l.junction.junction()),
                    clusters: l
                        .clusters
                        .map(|(root_order, variants, planes, leaf_facing)| ClusterCards {
                            root_order,
                            variants,
                            planes,
                            leaf_facing,
                        }),
                })
                .collect(),
            ..LodPolicy::default()
        }
    }

    /// The default policy's chain with the default atlas cell and no
    /// budget, for species without a spec.
    pub(crate) fn unbudgeted() -> Self {
        Self {
            levels: LodPolicy::default()
                .levels
                .iter()
                .map(|l| LevelSpec {
                    screen_size: l.screen_size,
                    crossfade: l.crossfade,
                    rings: (
                        l.rings.segments_per_metre,
                        l.rings.min_segments,
                        l.rings.max_segments,
                    ),
                    stations: (l.stations.max_bend, l.stations.max_spacing),
                    min_branch_radius: l.min_branch_radius,
                    leaf_fraction: l.leaf_fraction,
                    clusters: l
                        .clusters
                        .map(|c| (c.root_order, c.variants, c.planes, c.leaf_facing)),
                    junction: JunctionSpec::Embedded,
                    cell: default_cell(),
                    triangles: u64::MAX,
                    bytes: u64::MAX,
                })
                .collect(),
        }
    }
}
