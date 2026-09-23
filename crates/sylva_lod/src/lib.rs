// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Level-of-detail chains for sylva trees.
//!
//! [`build_lods`] regenerates every level from the skeleton rather than
//! decimating the full-detail mesh, so silhouettes stay clean and bark UVs
//! stay continuous. Each [`LodLevel`]:
//!
//! - re-meshes the bark with its own ring resolution and station spacing;
//! - drops the bark of branches thinner than `min_branch_radius` (and their
//!   descendants), while their leaves stay: at that distance the twig is
//!   sub-pixel but the foliage mass is not;
//! - keeps a keyed `leaf_fraction` of the leaves, scaling survivors by
//!   `1 / sqrt(leaf_fraction)` so the canopy's leaf area is preserved;
//! - draws leaves as full blades with its own station count, or as
//!   two-triangle cards sampling the leaf mask ([`LeafDetail`]).
//!
//! Leaf subsets nest: a leaf kept at a coarser level is kept at every finer
//! one, because each leaf's keep decision compares one keyed value against
//! the level's fraction. Transitions therefore only remove leaves, never
//! swap them. Each level also carries the screen-size threshold and
//! crossfade band a renderer switches on, and a report of what it holds.
//!
//! Cluster cards and impostor levels are later additions to the chain.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::fmt;

use exedra_mesh::Mesh;
use sylva_foliage::{Foliage, FoliageError, LeafInstance, LeafShape, card_mesh, leaf_mesh};
use sylva_mesh::{BarkMesh, MeshError, MeshParams, RingResolution, Stations, mesh_skeleton};
use sylva_skeleton::keyed::tag;
use sylva_skeleton::{Branch, Skeleton, SkeletonError};

/// How a level draws its leaves.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LeafDetail {
    /// Full blades meshed with this many midrib stations.
    Mesh {
        /// Midrib stations; at least 3.
        stations: u32,
    },
    /// Two-triangle cards over the leaf's mask frame, for alpha testing.
    Card,
}

/// One level of the chain.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LodLevel {
    /// Use this level while the tree's projected height is at least this
    /// fraction of the screen height.
    pub screen_size: f32,
    /// Width of the dithered crossfade into the next coarser level, as a
    /// fraction of `screen_size`.
    pub crossfade: f32,
    /// Bark ring resolution.
    pub rings: RingResolution,
    /// Bark ring placement.
    pub stations: Stations,
    /// Branches whose base radius is below this, in metres, lose their bark
    /// (with their descendants); 0 keeps every branch.
    pub min_branch_radius: f32,
    /// Fraction of leaves kept, in `(0, 1]`.
    pub leaf_fraction: f32,
    /// Leaf geometry.
    pub leaf_detail: LeafDetail,
}

/// An ordered chain of levels, finest first.
#[derive(Clone, Debug, PartialEq)]
pub struct LodPolicy {
    /// Levels from full detail down.
    pub levels: Vec<LodLevel>,
    /// Seed for the keyed leaf subsets.
    pub seed: u64,
}

impl Default for LodPolicy {
    /// Four levels for a broadleaf: full detail, a lighter mesh, a coarse
    /// mesh with leaf cards, and a sparse card crown.
    fn default() -> Self {
        let base = MeshParams::default();
        let rings = |segments_per_metre, min_segments, max_segments| RingResolution {
            segments_per_metre,
            min_segments,
            max_segments,
            ..base.rings
        };
        let stations = |max_bend, max_spacing| Stations {
            max_bend,
            max_spacing,
        };
        Self {
            levels: alloc::vec![
                LodLevel {
                    screen_size: 0.5,
                    crossfade: 0.1,
                    rings: base.rings,
                    stations: base.stations,
                    min_branch_radius: 0.0,
                    leaf_fraction: 1.0,
                    leaf_detail: LeafDetail::Mesh { stations: 32 },
                },
                LodLevel {
                    screen_size: 0.25,
                    crossfade: 0.1,
                    rings: rings(10.0, 4, 16),
                    stations: stations(0.3, 1.5),
                    min_branch_radius: 0.006,
                    leaf_fraction: 0.6,
                    leaf_detail: LeafDetail::Mesh { stations: 8 },
                },
                LodLevel {
                    screen_size: 0.1,
                    crossfade: 0.15,
                    rings: rings(5.0, 3, 10),
                    stations: stations(0.5, 2.5),
                    min_branch_radius: 0.015,
                    leaf_fraction: 0.3,
                    leaf_detail: LeafDetail::Card,
                },
                LodLevel {
                    screen_size: 0.04,
                    crossfade: 0.2,
                    rings: rings(3.0, 3, 6),
                    stations: stations(0.8, 4.0),
                    min_branch_radius: 0.04,
                    leaf_fraction: 0.12,
                    leaf_detail: LeafDetail::Card,
                },
            ],
            seed: 0,
        }
    }
}

/// Deterministic counts for one level.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct LodReport {
    /// Branches with bark.
    pub branches: u64,
    /// Branches whose bark was pruned.
    pub pruned_branches: u64,
    /// Bark triangles.
    pub bark_triangles: u64,
    /// Leaves kept.
    pub leaves: u64,
    /// Leaf triangles over all kept leaves.
    pub leaf_triangles: u64,
}

impl LodReport {
    /// Bark and leaf triangles together.
    #[must_use]
    pub fn triangles(&self) -> u64 {
        self.bark_triangles + self.leaf_triangles
    }
}

/// One built level.
#[derive(Clone, Debug)]
pub struct LodMesh {
    /// The level's settings.
    pub level: LodLevel,
    /// Its bark.
    pub bark: BarkMesh,
    /// One leaf template per foliage template, at this level's detail.
    pub templates: Vec<Mesh>,
    /// Kept leaves, rescaled to preserve leaf area.
    pub leaves: Vec<LeafInstance>,
    /// Deterministic counts.
    pub report: LodReport,
}

/// Why a chain could not be built.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LodError {
    /// A level parameter is out of range, or levels are not ordered finest
    /// first.
    Params {
        /// Index of the offending level.
        level: usize,
        /// The offending parameter.
        name: &'static str,
    },
    /// Rebuilding a pruned skeleton failed.
    Skeleton(SkeletonError),
    /// Meshing a level's bark failed.
    Mesh(MeshError),
    /// Meshing a leaf template failed.
    Foliage(FoliageError),
}

impl fmt::Display for LodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Params { level, name } => write!(f, "LOD level {level}: {name} is out of range"),
            Self::Skeleton(error) => write!(f, "pruned skeleton: {error}"),
            Self::Mesh(error) => write!(f, "LOD bark: {error}"),
            Self::Foliage(error) => write!(f, "LOD leaves: {error}"),
        }
    }
}

impl core::error::Error for LodError {}

impl LodPolicy {
    /// Checks every level and their order.
    ///
    /// # Errors
    ///
    /// [`LodError::Params`] for the first problem found.
    pub fn validate(&self) -> Result<(), LodError> {
        let mut previous: Option<&LodLevel> = None;
        for (index, level) in self.levels.iter().enumerate() {
            let bad = |ok: bool, name| {
                if ok {
                    Ok(())
                } else {
                    Err(LodError::Params { level: index, name })
                }
            };
            bad(
                level.screen_size.is_finite() && level.screen_size > 0.0,
                "screen_size",
            )?;
            bad((0.0..1.0).contains(&level.crossfade), "crossfade")?;
            bad(
                level.min_branch_radius.is_finite() && level.min_branch_radius >= 0.0,
                "min_branch_radius",
            )?;
            bad(
                level.leaf_fraction > 0.0 && level.leaf_fraction <= 1.0,
                "leaf_fraction",
            )?;
            if let LeafDetail::Mesh { stations } = level.leaf_detail {
                bad((3..=1024).contains(&stations), "leaf_detail")?;
            }
            if let Some(prev) = previous {
                bad(level.screen_size < prev.screen_size, "screen_size order")?;
                bad(
                    level.leaf_fraction <= prev.leaf_fraction,
                    "leaf_fraction order",
                )?;
            }
            previous = Some(level);
        }
        Ok(())
    }
}

/// A copy of `skeleton` without the branches whose base radius is below
/// `min_radius`, nor their descendants; sites are dropped (leaves are
/// instances already).
fn pruned(skeleton: &Skeleton, min_radius: f32) -> Result<(Skeleton, u64), LodError> {
    let mut out = Skeleton::new();
    let mut kept = alloc::vec![false; skeleton.branches().len()];
    let mut dropped = 0;
    for (index, branch) in skeleton.branches().iter().enumerate() {
        let parent_kept = branch
            .parent
            .is_none_or(|a| skeleton.index_of(a.parent).is_some_and(|p| kept[p]));
        let thick = branch.nodes[0].radius >= min_radius;
        if parent_kept && (thick || branch.parent.is_none()) {
            kept[index] = true;
            out.push_branch(Branch::clone(branch))
                .map_err(LodError::Skeleton)?;
        } else {
            dropped += 1;
        }
    }
    Ok((out, dropped))
}

/// Builds every level of `policy` for a grown, foliated tree.
///
/// `base` supplies the bark mapping, junction and root-flare settings shared
/// by all levels; each level substitutes its own rings and stations.
///
/// # Errors
///
/// [`LodError::Params`] for an invalid policy, or a meshing error.
pub fn build_lods(
    skeleton: &Skeleton,
    foliage: &Foliage,
    base: &MeshParams,
    policy: &LodPolicy,
) -> Result<Vec<LodMesh>, LodError> {
    policy.validate()?;
    // One keyed value per leaf, shared by every level, so subsets nest.
    let keep: Vec<f32> = foliage
        .instances
        .iter()
        .map(|leaf| {
            let site = &skeleton.sites()[leaf.site as usize];
            site.branch
                .key(policy.seed)
                .with(tag("lod.leaf"))
                .with(u64::from(site.kind))
                .with(u64::from(site.ordinal))
                .unit_f32()
        })
        .collect();
    let mut chain = Vec::with_capacity(policy.levels.len());
    for level in &policy.levels {
        let (skeleton, pruned_branches) = pruned(skeleton, level.min_branch_radius)?;
        let params = MeshParams {
            rings: level.rings,
            stations: level.stations,
            ..*base
        };
        let bark = mesh_skeleton(&skeleton, &params).map_err(LodError::Mesh)?;
        let templates: Vec<Mesh> = foliage
            .templates
            .iter()
            .map(|t| match level.leaf_detail {
                LeafDetail::Mesh { stations } => leaf_mesh(&LeafShape {
                    stations,
                    ..t.shape
                }),
                LeafDetail::Card => card_mesh(&t.shape),
            })
            .collect::<Result<_, _>>()
            .map_err(LodError::Foliage)?;
        let template_triangles: Vec<u64> = templates
            .iter()
            .map(|mesh| {
                mesh.faces()
                    .map(|f| mesh.face_loop(f).count().saturating_sub(2) as u64)
                    .sum()
            })
            .collect();
        let grow = 1.0 / libm::sqrtf(level.leaf_fraction);
        let leaves: Vec<LeafInstance> = foliage
            .instances
            .iter()
            .zip(&keep)
            .filter(|&(_, &k)| k < level.leaf_fraction)
            .map(|(leaf, _)| LeafInstance {
                scale: leaf.scale * grow,
                ..*leaf
            })
            .collect();
        let report = LodReport {
            branches: bark.report.branches,
            pruned_branches,
            bark_triangles: bark.report.triangles(),
            leaves: leaves.len() as u64,
            leaf_triangles: leaves
                .iter()
                .map(|l| template_triangles[l.template as usize])
                .sum(),
        };
        chain.push(LodMesh {
            level: *level,
            bark,
            templates,
            leaves,
            report,
        });
    }
    Ok(chain)
}

#[cfg(test)]
mod tests;
