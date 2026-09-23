// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Meshing failures.

use core::fmt;

use exedra_mesh::BuildError;
use sylva_skeleton::SkeletonError;

use crate::{Junction, MeshParams};

/// Why [`mesh_skeleton`](crate::mesh_skeleton) failed.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MeshError {
    /// A parameter is out of range.
    Params {
        /// The offending parameter.
        name: &'static str,
    },
    /// The skeleton failed validation, typically missing radii or frames.
    Skeleton(SkeletonError),
    /// The mesh kernel rejected the generated topology.
    Build(BuildError),
    /// A mesh kernel operation on a freshly built element failed.
    Kernel,
    /// The branch layer could not be registered.
    LayerConflict,
    /// The skeleton has more branches than a `u32` index can name.
    TooLarge,
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Params { name } => write!(f, "mesh parameter {name} is out of range"),
            Self::Skeleton(error) => write!(f, "skeleton is not meshable: {error}"),
            Self::Build(error) => write!(f, "mesh kernel rejected the bark: {error}"),
            Self::Kernel => f.write_str("mesh kernel refused an attribute write"),
            Self::LayerConflict => f.write_str("the branch layer could not be registered"),
            Self::TooLarge => f.write_str("too many branches to index"),
        }
    }
}

impl core::error::Error for MeshError {}

impl MeshParams {
    /// Checks every parameter range.
    ///
    /// # Errors
    ///
    /// [`MeshError::Params`] naming the first parameter out of range.
    pub fn validate(&self) -> Result<(), MeshError> {
        let bad = |ok: bool, name| {
            if ok {
                Ok(())
            } else {
                Err(MeshError::Params { name })
            }
        };
        let positive = |v: f32| v.is_finite() && v > 0.0;
        let r = &self.rings;
        bad(r.min_segments >= 3, "rings.min_segments")?;
        bad(r.max_segments >= r.min_segments, "rings.max_segments")?;
        bad(r.max_segments <= 1024, "rings.max_segments")?;
        bad(
            r.segments_per_metre.is_finite() && r.segments_per_metre >= 0.0,
            "rings.segments_per_metre",
        )?;
        bad(positive(self.stations.max_bend), "stations.max_bend")?;
        bad(positive(self.stations.max_spacing), "stations.max_spacing")?;
        bad(positive(self.bark.tile_size), "bark.tile_size")?;
        match self.junction {
            Junction::Embedded(c) => {
                bad(c.length.is_finite() && c.length >= 0.0, "junction.length")?;
                bad(positive(c.flare), "junction.flare")?;
                bad(
                    (0.0..=1.0).contains(&c.normal_blend),
                    "junction.normal_blend",
                )?;
                bad(c.rings <= 64, "junction.rings")?;
            }
        }
        if let Some(f) = self.root_flare {
            bad(positive(f.height), "root_flare.height")?;
            bad(f.flare.is_finite() && f.flare >= 0.0, "root_flare.flare")?;
            bad((0.0..1.0).contains(&f.lobe_depth), "root_flare.lobe_depth")?;
            bad(f.lobes <= 64, "root_flare.lobes")?;
            bad(f.rings <= 64, "root_flare.rings")?;
        }
        Ok(())
    }
}
