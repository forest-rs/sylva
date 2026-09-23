// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leaves for sylva trees: shapes, meshes, cards and placement.
//!
//! A [`LeafShape`] describes a blade by one half-width function along its
//! midrib. From that single description come:
//!
//! - [`leaf_mesh`]: the full-detail blade, folded and curled, with UVs;
//! - [`card_mesh`]: a flat quad for alpha-tested foliage;
//! - [`LeafShape::outline_at`]: the closed contour, from which
//!   `sylva_texture` rasterizes the coverage mask the card samples.
//!
//! All three share one UV frame ([`LeafShape::uv`]), so the mesh, card and
//! mask cannot disagree about where the leaf is.
//!
//! [`place_leaves`] builds a few keyed shape variants as [`LeafTemplate`]s and
//! places a [`LeafInstance`] at every leaf site of a
//! [`Skeleton`](sylva_skeleton::Skeleton): template, rigid transform, scale,
//! and a canopy normal for soft crown shading. Leaves stay instances until
//! level-of-detail packaging decides how to render them. Every choice is keyed
//! by the site, so foliage edits never disturb growth.
//!
//! # Example
//! ```rust
//! use sylva_foliage::{LeafShape, leaf_mesh};
//!
//! let oak = LeafShape::default();
//! let blade = leaf_mesh(&oak)?;
//! assert!(blade.faces().count() > 0);
//! assert!(oak.outline_at(256).len() > oak.outline().len());
//! # Ok::<(), sylva_foliage::FoliageError>(())
//! ```

#![no_std]

extern crate alloc;

mod error;
mod mesh;
mod place;
mod shape;

pub use error::FoliageError;
pub use mesh::{card_mesh, leaf_mesh};
pub use place::{
    Foliage, FoliageParams, FoliageReport, LeafInstance, LeafTemplate, Variation, place_leaves,
};
pub use shape::LeafShape;

#[cfg(test)]
mod tests;
