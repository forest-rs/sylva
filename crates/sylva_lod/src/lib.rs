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
//! Coarse levels can replace leaves with [`ClusterCards`]: every branch
//! subtree of one order becomes a few crossed quads sampling an atlas of
//! baked exemplar clusters ([`bake_clusters`]), and that order's bark is
//! pruned, because the cards carry it. The chain can end in an [`Impostor`],
//! crossed vertical billboards of the whole tree ([`bake_impostor`]).
//! [`build_lods`] returns the levels and impostor as geometry and views;
//! baking, which needs the bark textures, is a separate step.

#![no_std]

extern crate alloc;

mod bake;
mod clusters;
mod impostor;

use alloc::vec::Vec;
use core::fmt;

pub use bake::{Atlas, AtlasSettings, CardMaterials, bake_clusters, bake_impostor};
pub use clusters::{
    AtlasLayout, ClusterCard, ClusterCards, ClusterVariant, Clusters, push_unit_quad,
};
pub use impostor::{
    Impostor, ImpostorLayout, ImpostorPolicy, hemi_octahedral_decode, hemi_octahedral_encode,
};

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
    /// Leaf geometry for leaves not in a cluster.
    pub leaf_detail: LeafDetail,
    /// Cluster cards replacing the leaves (and bark) of every branch subtree
    /// of one order; `None` draws every kept leaf.
    pub clusters: Option<ClusterCards>,
}

/// An ordered chain of levels, finest first.
#[derive(Clone, Debug, PartialEq)]
pub struct LodPolicy {
    /// Levels from full detail down.
    pub levels: Vec<LodLevel>,
    /// Seed for the keyed leaf subsets.
    pub seed: u64,
    /// The billboard impostor ending the chain, if any.
    pub impostor: Option<ImpostorPolicy>,
}

impl Default for LodPolicy {
    /// Four levels for a broadleaf, then an impostor: full bark with a card
    /// per leaf, lighter bark with a leaf-area-preserving subset of leaf
    /// cards, coarse bark with a card per twig cluster, and scaffold bark
    /// with a card per branch cluster. Leaves are alpha-tested cards from the
    /// finest level, as in real-time trees; [`LeafDetail::Mesh`] draws folded
    /// blades where the budget allows.
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
                    rings: rings(base.rings.segments_per_metre, 3, base.rings.max_segments),
                    stations: base.stations,
                    min_branch_radius: 0.0,
                    leaf_fraction: 1.0,
                    leaf_detail: LeafDetail::Card,
                    clusters: None,
                },
                LodLevel {
                    screen_size: 0.25,
                    crossfade: 0.1,
                    rings: rings(10.0, 3, 16),
                    stations: stations(0.3, 1.5),
                    min_branch_radius: 0.012,
                    leaf_fraction: 0.6,
                    leaf_detail: LeafDetail::Card,
                    clusters: None,
                },
                LodLevel {
                    screen_size: 0.1,
                    crossfade: 0.15,
                    rings: rings(5.0, 3, 10),
                    stations: stations(0.5, 2.5),
                    min_branch_radius: 0.015,
                    leaf_fraction: 0.3,
                    leaf_detail: LeafDetail::Card,
                    clusters: Some(ClusterCards {
                        root_order: 3,
                        variants: 8,
                        planes: 2,
                        leaf_facing: 0.35,
                    }),
                },
                LodLevel {
                    screen_size: 0.04,
                    crossfade: 0.2,
                    rings: rings(3.0, 3, 6),
                    stations: stations(0.8, 4.0),
                    min_branch_radius: 0.04,
                    leaf_fraction: 0.12,
                    leaf_detail: LeafDetail::Card,
                    clusters: Some(ClusterCards {
                        root_order: 2,
                        variants: 4,
                        planes: 2,
                        leaf_facing: 0.0,
                    }),
                },
            ],
            seed: 0,
            impostor: Some(ImpostorPolicy::default()),
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
    /// Cluster cards.
    pub cluster_cards: u64,
    /// Leaves standing in a cluster card instead of drawn.
    pub clustered_leaves: u64,
    /// Card triangles: two per plane per card.
    pub card_triangles: u64,
}

impl LodReport {
    /// Bark, leaf and card triangles together.
    #[must_use]
    pub fn triangles(&self) -> u64 {
        self.bark_triangles + self.leaf_triangles + self.card_triangles
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
    /// Kept leaves outside clusters, rescaled to preserve leaf area.
    pub leaves: Vec<LeafInstance>,
    /// The level's cluster cards, when it has any.
    pub clusters: Option<Clusters>,
    /// Deterministic counts.
    pub report: LodReport,
}

/// A built chain: levels finest first, then the impostor.
#[derive(Clone, Debug)]
pub struct LodChain {
    /// The levels.
    pub levels: Vec<LodMesh>,
    /// The impostor, when the policy asks for one.
    pub impostor: Option<Impostor>,
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
    /// The impostor policy is out of range or not coarser than the last
    /// level.
    Impostor(&'static str),
    /// Baking an atlas failed.
    Bake(&'static str),
}

impl fmt::Display for LodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Params { level, name } => write!(f, "LOD level {level}: {name} is out of range"),
            Self::Skeleton(error) => write!(f, "pruned skeleton: {error}"),
            Self::Mesh(error) => write!(f, "LOD bark: {error}"),
            Self::Foliage(error) => write!(f, "LOD leaves: {error}"),
            Self::Impostor(name) => write!(f, "impostor: {name} is out of range"),
            Self::Bake(what) => write!(f, "atlas bake: {what}"),
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
            if let Some(clusters) = level.clusters {
                bad((1..=64).contains(&clusters.variants), "clusters.variants")?;
                bad((1..=3).contains(&clusters.planes), "clusters.planes")?;
                bad(
                    (0.0..=1.0).contains(&clusters.leaf_facing),
                    "clusters.leaf_facing",
                )?;
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
        if let Some(impostor) = self.impostor {
            let bad = |ok: bool, name| {
                if ok {
                    Ok(())
                } else {
                    Err(LodError::Impostor(name))
                }
            };
            bad(
                impostor.screen_size.is_finite() && impostor.screen_size > 0.0,
                "screen_size",
            )?;
            bad(
                previous.is_none_or(|last| impostor.screen_size < last.screen_size),
                "screen_size order",
            )?;
            bad((0.0..1.0).contains(&impostor.crossfade), "crossfade")?;
            match impostor.layout {
                ImpostorLayout::Crossed { planes } => bad((1..=8).contains(&planes), "planes")?,
                ImpostorLayout::Octahedral { frames } => {
                    bad((2..=16).contains(&frames), "frames")?;
                }
            }
        }
        Ok(())
    }
}

/// A copy of `skeleton` without the branches whose base radius is below
/// `min_radius`, nor those of order `card_order` or above (their cluster
/// cards carry them), nor their descendants; sites are dropped (leaves are
/// instances already).
fn pruned(
    skeleton: &Skeleton,
    min_radius: f32,
    card_order: Option<u32>,
) -> Result<(Skeleton, u64), LodError> {
    let mut out = Skeleton::new();
    let mut kept = alloc::vec![false; skeleton.branches().len()];
    let mut dropped = 0;
    for (index, branch) in skeleton.branches().iter().enumerate() {
        let parent_kept = branch
            .parent
            .is_none_or(|a| skeleton.index_of(a.parent).is_some_and(|p| kept[p]));
        let thick = branch.nodes[0].radius >= min_radius
            && card_order.is_none_or(|order| branch.order < order);
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
/// [`LodError::Params`] or [`LodError::Impostor`] for an invalid policy, or
/// a meshing error.
pub fn build_lods(
    skeleton: &Skeleton,
    foliage: &Foliage,
    base: &MeshParams,
    policy: &LodPolicy,
) -> Result<LodChain, LodError> {
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
        let (clusters, clustered) = match level.clusters {
            Some(params) => {
                let (clusters, clustered) = clusters::build_clusters(skeleton, foliage, params);
                (Some(clusters), clustered)
            }
            None => (None, alloc::vec![false; foliage.instances.len()]),
        };
        let (pruned_skeleton, pruned_branches) = pruned(
            skeleton,
            level.min_branch_radius,
            level.clusters.map(|c| c.root_order),
        )?;
        let params = MeshParams {
            rings: level.rings,
            stations: level.stations,
            ..*base
        };
        let bark = mesh_skeleton(&pruned_skeleton, &params).map_err(LodError::Mesh)?;
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
            .zip(&clustered)
            .filter(|&((_, &k), &c)| k < level.leaf_fraction && !c)
            .map(|((leaf, _), _)| LeafInstance {
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
            cluster_cards: clusters.as_ref().map_or(0, |c| c.cards.len() as u64),
            clustered_leaves: clustered.iter().filter(|&&c| c).count() as u64,
            card_triangles: clusters
                .as_ref()
                .map_or(0, |c| 2 * u64::from(c.params.planes) * c.cards.len() as u64),
        };
        chain.push(LodMesh {
            level: *level,
            bark,
            templates,
            leaves,
            clusters,
            report,
        });
    }
    Ok(LodChain {
        levels: chain,
        impostor: policy
            .impostor
            .map(|impostor| Impostor::fit(skeleton, foliage, impostor)),
    })
}

#[cfg(test)]
mod tests;
