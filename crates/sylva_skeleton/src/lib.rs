// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The skeleton IR shared by every sylva growth backend.
//!
//! A [`Skeleton`] is a tree of [`Branch`]es, each a sampled centerline of
//! [`Node`]s (position, radius, rotation-minimizing [`Frame`], age), attached
//! to its parent at a normalized arc-length parameter, plus [`Site`]s where
//! foliage and other organs attach. It is the boundary between growth and
//! everything downstream: rule-based and simulated growth both produce it, and
//! branch meshing, foliage, wind and LOD consume it without knowing which
//! backend grew it.
//!
//! - [`BranchId`]s hash the generation path, not storage order or the seed, so
//!   regeneration reproduces the IDs of every unaffected branch.
//! - [`keyed`] randomness derives every decision from `hash(seed, keys)`, so
//!   changing one decision never reshuffles unrelated ones.
//! - [`passes`] fill in what growth leaves open: pipe-model radii and
//!   rotation-minimizing frames.
//! - Structure is checked on every push ([`Skeleton::push_branch`]);
//!   completeness (positive radii, orthonormal frames) by
//!   [`Skeleton::validate`]. Both report typed [`SkeletonError`]s.
//!
//! Coordinates are metres in tree space with `+Z` up. Math goes through glam
//! with its `libm` and `scalar-math` features, so results are bit-identical
//! across platforms.
//!
//! # Example
//! ```rust
//! use glam::Vec3;
//! use sylva_skeleton::passes::{FrameParams, PipeModel, compute_frames, pipe_model_radii};
//! use sylva_skeleton::{Branch, BranchId, Node, Skeleton};
//!
//! let mut skeleton = Skeleton::new();
//! skeleton.push_branch(Branch {
//!     id: BranchId::root(0),
//!     order: 0,
//!     parent: None,
//!     nodes: vec![Node::at(Vec3::ZERO), Node::at(Vec3::Z), Node::at(Vec3::new(0.1, 0.0, 2.0))],
//! })?;
//! pipe_model_radii(&mut skeleton, &PipeModel::default())?;
//! compute_frames(&mut skeleton, &FrameParams::default());
//! skeleton.validate()?;
//! assert_eq!(skeleton.stats().nodes, 3);
//! # Ok::<(), sylva_skeleton::SkeletonError>(())
//! ```

#![no_std]

extern crate alloc;

mod error;
mod frame;
mod id;
pub mod keyed;
pub mod passes;
mod skeleton;

pub use error::SkeletonError;
pub use frame::{FRAME_EPSILON, Frame};
pub use id::BranchId;
pub use skeleton::{Attachment, Branch, BranchSample, Node, Site, Skeleton, SkeletonStats};

/// Re-exported so callers use the same glam version as the IR.
pub use glam;
