// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Species descriptions: what a kind of tree is, as plain data.
//!
//! A [`Species`] names a tree and describes how it grows. It is data, not
//! code: presets live as data files (with the `serde` feature) and a tree is
//! `species.grow(seed)`. It carries growth and, optionally, foliage
//! ([`FoliageParams`]); bark, LOD and wind descriptions join the species as
//! the stages that consume them land.
//!
//! The growth description is backend-neutral at this level: [`Growth`] names
//! the backend and carries its parameters, and everything downstream of the
//! skeleton ignores which backend grew it.
//!
//! # Example
//! ```rust
//! use sylva_grow::{Hierarchy, Radii, Trunk};
//! use sylva_species::{Growth, Species};
//!
//! let species = Species {
//!     name: "pole".into(),
//!     growth: Growth::Hierarchical(Hierarchy {
//!         trunk: Trunk::default(),
//!         levels: vec![],
//!         envelope: None,
//!         radii: Radii::default(),
//!         segment_length: 0.5,
//!     }),
//!     foliage: None,
//! };
//! let grown = species.grow(1)?;
//! assert_eq!(grown.skeleton.branches().len(), 1);
//! # Ok::<(), sylva_grow::GrowError>(())
//! ```

#![no_std]

extern crate alloc;

use alloc::string::String;

pub use sylva_foliage::FoliageParams;
use sylva_grow::{GrowError, Grown, Hierarchy};

/// A kind of tree.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Species {
    /// Display name.
    pub name: String,
    /// How the species grows.
    pub growth: Growth,
    /// Leaves on the grown skeleton's sites; `None` for a bare tree.
    #[cfg_attr(feature = "serde", serde(default))]
    pub foliage: Option<FoliageParams>,
}

/// A growth backend and its parameters.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Growth {
    /// Rule-based, per-level growth ([`sylva_grow`]).
    Hierarchical(Hierarchy),
}

impl Species {
    /// Grows one tree from `seed`.
    ///
    /// # Errors
    ///
    /// Whatever the growth backend reports for invalid parameters.
    pub fn grow(&self, seed: u64) -> Result<Grown, GrowError> {
        match &self.growth {
            Growth::Hierarchical(hierarchy) => sylva_grow::grow(hierarchy, seed),
        }
    }
}
