// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Stable branch identity.

use core::fmt;

use crate::keyed::{Key, hash, tag};

/// Stable identity of a branch: a hash of its generation path.
///
/// An ID is derived from the parent's ID, the lineage that produced the branch
/// (for example a generator level or bud kind), and the branch's ordinal within
/// that lineage. It is independent of storage order and of the tree's random
/// seed, so regeneration after an edit reproduces the IDs of every branch whose
/// path did not change. Caches, provenance and art-directed overrides key on
/// it.
///
/// IDs are 64-bit hashes; distinct paths collide with negligible probability,
/// and [`Skeleton::push_branch`](crate::Skeleton::push_branch) rejects a
/// duplicate rather than trusting that.
///
/// # Example
/// ```rust
/// use sylva_skeleton::BranchId;
///
/// let trunk = BranchId::root(0);
/// let first = trunk.child(1, 0);
/// assert_eq!(first, BranchId::root(0).child(1, 0));
/// assert_ne!(first, trunk.child(1, 1));
/// ```
#[derive(Copy, Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BranchId(u64);

impl BranchId {
    const DOMAIN: u64 = tag("sylva.branch");

    /// The ID of root stem number `stem` (usually `0`; multi-stemmed plants
    /// use one root per stem).
    #[must_use]
    pub const fn root(stem: u64) -> Self {
        Self(hash(Self::DOMAIN, &[stem]))
    }

    /// The ID of this branch's child number `ordinal` within `lineage`.
    #[must_use]
    pub const fn child(self, lineage: u64, ordinal: u64) -> Self {
        Self(hash(self.0, &[lineage, ordinal]))
    }

    /// Reconstructs an ID from [`Self::bits`].
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The raw hash.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// A random key for decisions about this branch under `seed`.
    ///
    /// Fold a purpose (and any ordinal) in before drawing:
    /// `id.key(seed).with(tag("angle")).unit_f32()`.
    #[must_use]
    pub const fn key(self, seed: u64) -> Key {
        Key::new(seed).with(self.0)
    }
}

impl fmt::Debug for BranchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BranchId({:016x})", self.0)
    }
}

impl fmt::Display for BranchId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::BranchId;

    #[test]
    fn ids_depend_on_path_only() {
        let a = BranchId::root(0).child(1, 2).child(3, 4);
        let b = BranchId::root(0).child(1, 2).child(3, 4);
        assert_eq!(a, b);
        assert_ne!(a, BranchId::root(1).child(1, 2).child(3, 4));
        assert_ne!(a, BranchId::root(0).child(1, 2).child(4, 3));
        assert_ne!(BranchId::root(0).child(1, 0), BranchId::root(0).child(2, 0));
    }

    #[test]
    fn keys_differ_by_seed_but_ids_do_not() {
        let id = BranchId::root(0).child(1, 0);
        assert_ne!(id.key(1).bits(), id.key(2).bits());
        assert_eq!(BranchId::from_bits(id.bits()), id);
    }
}
