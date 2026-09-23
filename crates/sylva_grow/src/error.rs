// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::fmt;

use sylva_skeleton::SkeletonError;

/// Why [`grow()`](crate::grow()) failed.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum GrowError {
    /// A parameter is out of range. Checked before any growth.
    InvalidParameter {
        /// The level index into [`Hierarchy::levels`](crate::Hierarchy::levels),
        /// or `None` for tree-wide and trunk parameters.
        level: Option<usize>,
        /// The parameter's field name.
        name: &'static str,
    },
    /// The grown skeleton failed a structural check. This indicates a
    /// generator bug, not bad input.
    Skeleton(SkeletonError),
}

impl fmt::Display for GrowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParameter {
                level: Some(level),
                name,
            } => write!(f, "level {level}: invalid parameter `{name}`"),
            Self::InvalidParameter { level: None, name } => {
                write!(f, "invalid parameter `{name}`")
            }
            Self::Skeleton(error) => write!(f, "grown skeleton is invalid: {error}"),
        }
    }
}

impl core::error::Error for GrowError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Skeleton(error) => Some(error),
            Self::InvalidParameter { .. } => None,
        }
    }
}

impl From<SkeletonError> for GrowError {
    fn from(error: SkeletonError) -> Self {
        Self::Skeleton(error)
    }
}
