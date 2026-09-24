// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Hierarchical rule-based tree growth.
//!
//! A [`Hierarchy`] describes a trunk and a list of branching [`Level`]s in
//! the style of Weber and Penn (1995) and per-level generators such as
//! SpeedTree's. Each level says how many children grow on every branch of
//! the level above ([`Count`]), where along it ([`Level::span`]), how they
//! turn around it ([`Arrangement`]), at what angle and length ([`Curve`]s over
//! the parent position), and how their centerlines bend ([`Shape`]: planar
//! curve, smooth keyed gnarl, sympodial kinks, gravitropism, sag,
//! phototropism). [`Level::balance`] evens out lopsided sibling crowns, and an
//! optional crown [`Envelope`] prunes branches that grow out of it.
//!
//! [`grow()`] turns a hierarchy and a seed into a
//! [`Skeleton`](sylva_skeleton::Skeleton) with pipe-model radii, frames and
//! foliage sites, plus a [`GrowReport`] of deterministic counts.
//!
//! # Stability
//!
//! Every random decision is keyed by the seed, the deciding branch's
//! generation-path [`BranchId`](sylva_skeleton::BranchId) and a purpose tag.
//! Editing one level therefore leaves every branch of the levels above it
//! bit-identical, and an edit that does not change a branch's generation path
//! keeps its ID. Radii are the documented exception: the pipe model sizes a
//! branch by everything it supports.
//!
//! # Example
//! ```rust
//! use sylva_grow::{Count, Curve, Hierarchy, Level, Radii, Trunk, grow};
//!
//! let hierarchy = Hierarchy {
//!     trunk: Trunk { length: 8.0, ..Trunk::default() },
//!     levels: vec![Level {
//!         count: Count::Fixed(12),
//!         span: [0.3, 1.0],
//!         angle: Curve::linear(1.2, 0.6),
//!         length: Curve::linear(0.5, 0.2),
//!         ..Level::default()
//!     }],
//!     envelope: None,
//!     shade: None,
//!     radii: Radii::default(),
//!     segment_length: 0.5,
//! };
//! let grown = grow(&hierarchy, 7)?;
//! assert_eq!(grown.report.branches_by_level, [1, 12]);
//! # Ok::<(), sylva_grow::GrowError>(())
//! ```

#![no_std]

extern crate alloc;

mod curve;
mod error;
mod grow;
mod params;
mod validate;

pub use curve::{Curve, CurveError};
pub use error::GrowError;
pub use grow::{GrowReport, Grown, grow};
pub use params::{
    Arrangement, Count, DEFAULT_LUMP_SIZE, Envelope, GOLDEN_ANGLE, Hierarchy, Level, Radii, Shade,
    Shape, Sites, Trunk,
};

#[cfg(test)]
mod tests;
