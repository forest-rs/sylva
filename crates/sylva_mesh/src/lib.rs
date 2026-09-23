// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bark surface meshing for sylva skeletons.
//!
//! [`mesh_skeleton`] turns a [`Skeleton`](sylva_skeleton::Skeleton) with
//! radii and frames into an [`exedra_mesh::Mesh`] of bark tubes, one per
//! branch:
//!
//! - **Rings** have a constant segment count per branch, chosen from its base
//!   circumference ([`RingResolution`]), at curvature- and spacing-adaptive
//!   stations ([`Stations`]). Ring angle zero follows the skeleton's
//!   rotation-minimizing frame, so rings do not twist.
//! - **Bark UVs** wrap each branch an integer number of times with an
//!   explicit, tagged seam, and advance by arc length so texels are square at
//!   the base and uniform across branch sizes ([`BarkMapping`]).
//! - **Junctions** are a replaceable strategy ([`Junction`]). The
//!   [`Junction::Embedded`] strategy keeps the child's base inside its
//!   parent, flares a collar, and blends collar normals toward the parent's
//!   surface.
//! - **Root flare** ([`RootFlare`]) buttresses the base of root stems.
//! - **Provenance**: [`BRANCH_LAYER`] records each vertex's branch index, for
//!   inspection and for per-branch tables such as wind pivots.
//!
//! Normals are authored per corner; extract with
//! `NormalsSource::CustomOnly` and carry [`BRANCH_LAYER`] as an
//! `ExtractAttribute` to keep provenance in the render buffers. The
//! [`MeshReport`] counts rings, faces and segment ranges deterministically.
//!
//! # Example
//! ```rust
//! use sylva_mesh::{MeshParams, mesh_skeleton};
//! use sylva_skeleton::glam::Vec3;
//! use sylva_skeleton::passes::{FrameParams, PipeModel, compute_frames, pipe_model_radii};
//! use sylva_skeleton::{Branch, BranchId, Node, Skeleton};
//!
//! let mut skeleton = Skeleton::new();
//! skeleton.push_branch(Branch {
//!     id: BranchId::root(0),
//!     order: 0,
//!     parent: None,
//!     nodes: vec![Node::at(Vec3::ZERO), Node::at(Vec3::new(0.0, 0.0, 2.0))],
//! })?;
//! compute_frames(&mut skeleton, &FrameParams::default());
//! pipe_model_radii(&mut skeleton, &PipeModel::default())?;
//!
//! let bark = mesh_skeleton(&skeleton, &MeshParams::default())?;
//! assert_eq!(bark.report.branches, 1);
//! assert!(bark.report.triangles() > 0);
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

#![no_std]

extern crate alloc;

mod build;
mod error;
mod params;

pub use build::{BRANCH_LAYER, BarkMesh, MeshReport, branch_of, mesh_skeleton};
pub use error::MeshError;
pub use params::{BarkMapping, Collar, Junction, MeshParams, RingResolution, RootFlare, Stations};

#[cfg(test)]
mod tests;
