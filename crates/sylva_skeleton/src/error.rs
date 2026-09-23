// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Typed skeleton errors.

use core::fmt;

use crate::BranchId;

/// A structural or completeness problem in a [`Skeleton`](crate::Skeleton).
///
/// [`Skeleton::push_branch`](crate::Skeleton::push_branch) and
/// [`Skeleton::push_site`](crate::Skeleton::push_site) reject structural
/// problems; [`Skeleton::validate`](crate::Skeleton::validate) additionally
/// requires positive radii and orthonormal frames. Node indices refer to the
/// branch's node list.
#[derive(Copy, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SkeletonError {
    /// A branch needs at least two nodes.
    TooFewNodes {
        /// The rejected branch.
        branch: BranchId,
        /// Its node count.
        count: usize,
    },
    /// The ID is already present.
    DuplicateBranch {
        /// The repeated ID.
        branch: BranchId,
    },
    /// The attachment names a branch not pushed before this one.
    MissingParent {
        /// The rejected branch.
        branch: BranchId,
        /// The unknown parent.
        parent: BranchId,
    },
    /// An attachment or site parameter is outside `[0, 1]` or not finite.
    ParameterOutOfRange {
        /// The branch being attached to.
        branch: BranchId,
        /// The rejected parameter.
        t: f32,
    },
    /// A root must have order 0 and a child its parent's order plus one.
    OrderMismatch {
        /// The rejected branch.
        branch: BranchId,
        /// The order it declared.
        order: u32,
        /// The order its position in the tree requires.
        expected: u32,
    },
    /// A node position, radius, age or frame is not finite.
    NonFinite {
        /// The branch.
        branch: BranchId,
        /// The node.
        node: usize,
    },
    /// A radius is negative (structural) or not positive (complete).
    InvalidRadius {
        /// The branch.
        branch: BranchId,
        /// The node.
        node: usize,
        /// The rejected radius.
        radius: f32,
    },
    /// Two consecutive nodes coincide, leaving no direction between them.
    ZeroLengthSegment {
        /// The branch.
        branch: BranchId,
        /// The first node of the segment.
        node: usize,
    },
    /// A frame is not orthonormal within
    /// [`FRAME_EPSILON`](crate::FRAME_EPSILON).
    FrameNotOrthonormal {
        /// The branch.
        branch: BranchId,
        /// The node, or `usize::MAX` for a site on this branch.
        node: usize,
    },
    /// A site names a branch not in the skeleton.
    UnknownSiteBranch {
        /// The unknown branch.
        branch: BranchId,
    },
    /// A pass parameter is invalid.
    InvalidParameter {
        /// The parameter's name.
        name: &'static str,
    },
}

impl fmt::Display for SkeletonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewNodes { branch, count } => {
                write!(
                    f,
                    "branch {branch} has {count} nodes; at least 2 are required"
                )
            }
            Self::DuplicateBranch { branch } => write!(f, "branch {branch} is already present"),
            Self::MissingParent { branch, parent } => {
                write!(f, "branch {branch} attaches to unknown parent {parent}")
            }
            Self::ParameterOutOfRange { branch, t } => {
                write!(f, "parameter {t} on branch {branch} is outside [0, 1]")
            }
            Self::OrderMismatch {
                branch,
                order,
                expected,
            } => write!(f, "branch {branch} has order {order}; expected {expected}"),
            Self::NonFinite { branch, node } => {
                write!(f, "node {node} of branch {branch} is not finite")
            }
            Self::InvalidRadius {
                branch,
                node,
                radius,
            } => write!(
                f,
                "node {node} of branch {branch} has invalid radius {radius}"
            ),
            Self::ZeroLengthSegment { branch, node } => {
                write!(
                    f,
                    "nodes {node} and {} of branch {branch} coincide",
                    node + 1
                )
            }
            Self::FrameNotOrthonormal { branch, node } => {
                write!(
                    f,
                    "frame at node {node} of branch {branch} is not orthonormal"
                )
            }
            Self::UnknownSiteBranch { branch } => {
                write!(f, "site references unknown branch {branch}")
            }
            Self::InvalidParameter { name } => write!(f, "invalid parameter {name}"),
        }
    }
}

impl core::error::Error for SkeletonError {}
