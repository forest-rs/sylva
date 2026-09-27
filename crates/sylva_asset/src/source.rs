// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Generated vegetation, before a rendering or storage strategy is chosen.

use alloc::collections::BTreeSet;
use sylva_foliage::{Foliage, FoliageParams, FoliageReport, place_leaves};
use sylva_skeleton::Skeleton;

use crate::{AssetError, TreeMaterials};

/// A generated tree in metres, right-handed, with `+Z` up.
///
/// Owns the semantic skeleton, reusable leaf shapes, placements and authored
/// materials. It contains no meshes, LOD policy, encoded textures, or storage
/// schema. Growth recipes and seeds remain with the authoring caller; this
/// value is the generated result, not a recipe or a serialized format.
///
/// Leaf identity is scoped to this tree. Shape UVs define a shared material
/// coordinate frame for blades, tissue geometry and cards. Surface orientation
/// follows each shape and placement; canopy normals remain separate artistic
/// shading data. Branch associations describe attachment, not a skeletal rig.
///
/// The immutable accessors keep site indices, stable identities and placements
/// consistent. Construct a new snapshot after editing the skeleton or foliage.
#[derive(Clone, Debug)]
pub struct GeneratedTree {
    skeleton: Skeleton,
    foliage: Foliage,
    materials: TreeMaterials,
}

impl GeneratedTree {
    /// Describes foliage on a completed skeleton without constructing meshes.
    /// `None` produces a bare tree.
    ///
    /// # Errors
    ///
    /// Rejects invalid skeletons, duplicate site identities, non-positive site
    /// scales, and invalid foliage parameters. Duplicate sites would otherwise
    /// give different organs the same stable identity.
    pub fn new(
        skeleton: Skeleton,
        foliage: Option<&FoliageParams>,
        materials: TreeMaterials,
    ) -> Result<Self, AssetError> {
        skeleton.validate().map_err(AssetError::Skeleton)?;
        let mut sites = BTreeSet::new();
        for site in skeleton.sites() {
            if !sites.insert((site.branch, site.kind, site.ordinal)) {
                return Err(AssetError::Source("duplicate site identity"));
            }
            if site.scale <= 0.0 {
                return Err(AssetError::Source("site scale must be positive"));
            }
        }
        let foliage = match foliage {
            Some(params) => place_leaves(&skeleton, params).map_err(AssetError::Foliage)?,
            None => Foliage {
                templates: alloc::vec![],
                instances: alloc::vec![],
                report: FoliageReport::default(),
            },
        };
        Ok(Self {
            skeleton,
            foliage,
            materials,
        })
    }

    /// Botanical structure, stable branch identities and attachment sites.
    #[must_use]
    pub fn skeleton(&self) -> &Skeleton {
        &self.skeleton
    }

    /// Reusable shapes and identified placements, without compiled meshes.
    #[must_use]
    pub fn foliage(&self) -> &Foliage {
        &self.foliage
    }

    /// Authored bark and leaf material parameters.
    #[must_use]
    pub fn materials(&self) -> &TreeMaterials {
        &self.materials
    }
}
