// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The skeleton IR: branches, attachments and sites.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use glam::Vec3;

use crate::{BranchId, FRAME_EPSILON, Frame, SkeletonError};

/// One sample of a branch centerline.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Node {
    /// Centerline position, in metres, tree space (Z up).
    pub position: Vec3,
    /// Wood radius at this node, in metres.
    ///
    /// Growth backends may leave radii at zero and run
    /// [`pipe_model_radii`](crate::passes::pipe_model_radii).
    pub radius: f32,
    /// Rotation-minimizing frame; see
    /// [`compute_frames`](crate::passes::compute_frames).
    pub frame: Frame,
    /// Age of the wood at this node, in growth units (years for simulated
    /// growth). Informational: bark and wind use it; structure does not.
    pub age: f32,
}

impl Node {
    /// A node at `position` with zero radius and age and a default frame.
    #[must_use]
    pub fn at(position: Vec3) -> Self {
        Self {
            position,
            radius: 0.0,
            frame: Frame::default(),
            age: 0.0,
        }
    }
}

/// Where a branch leaves its parent.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Attachment {
    /// The parent branch.
    pub parent: BranchId,
    /// Normalized arc-length position on the parent, in `[0, 1]`.
    pub t: f32,
}

/// A branch: a sampled centerline from its base to its tip.
#[derive(Clone, Debug, PartialEq)]
pub struct Branch {
    /// Stable identity.
    pub id: BranchId,
    /// Branching order: 0 for a root stem, parent's order plus one otherwise.
    pub order: u32,
    /// Attachment to the parent; `None` for a root stem.
    pub parent: Option<Attachment>,
    /// Centerline nodes from base to tip; at least two.
    pub nodes: Vec<Node>,
}

/// A branch sampled at a normalized arc-length parameter.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BranchSample {
    /// Interpolated position.
    pub position: Vec3,
    /// Interpolated radius.
    pub radius: f32,
    /// Interpolated, re-orthonormalized frame.
    pub frame: Frame,
}

impl Branch {
    /// Cumulative arc length at each node; the first entry is zero.
    #[must_use]
    pub fn arc_lengths(&self) -> Vec<f32> {
        let mut lengths = Vec::with_capacity(self.nodes.len());
        let mut total = 0.0;
        lengths.push(total);
        for pair in self.nodes.windows(2) {
            total += pair[0].position.distance(pair[1].position);
            lengths.push(total);
        }
        lengths
    }

    /// Total centerline length.
    #[must_use]
    pub fn length(&self) -> f32 {
        self.nodes
            .windows(2)
            .map(|pair| pair[0].position.distance(pair[1].position))
            .sum()
    }

    /// Samples the branch at normalized arc length `t`, clamped to `[0, 1]`.
    #[must_use]
    pub fn sample(&self, t: f32) -> BranchSample {
        let lengths = self.arc_lengths();
        let total = *lengths.last().unwrap_or(&0.0);
        let target = t.clamp(0.0, 1.0) * total;
        let segment = lengths
            .windows(2)
            .position(|pair| target <= pair[1])
            .unwrap_or(self.nodes.len().saturating_sub(2));
        let (a, b) = (&self.nodes[segment], &self.nodes[segment + 1]);
        let span = lengths[segment + 1] - lengths[segment];
        let s = if span > 0.0 {
            ((target - lengths[segment]) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let tangent = a.frame.tangent.lerp(b.frame.tangent, s);
        let normal = a.frame.normal.lerp(b.frame.normal, s);
        BranchSample {
            position: a.position.lerp(b.position, s),
            radius: a.radius + (b.radius - a.radius) * s,
            frame: Frame::from_tangent(tangent, normal).unwrap_or(a.frame),
        }
    }
}

/// A place where foliage, fruit or other organs attach.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Site {
    /// The carrying branch.
    pub branch: BranchId,
    /// Stable ordinal among the branch's sites of this kind.
    pub ordinal: u32,
    /// Caller-defined kind (leaf, bud, fruit, …).
    pub kind: u32,
    /// Normalized arc-length position on the branch, in `[0, 1]`.
    pub t: f32,
    /// Orientation of the attached organ; `tangent` points away from the
    /// branch.
    pub frame: Frame,
    /// Organ scale relative to its species size.
    pub scale: f32,
}

/// A tree skeleton: branches in parent-before-child order, plus sites.
///
/// The skeleton is the boundary between growth and everything downstream.
/// Growth backends push branches; shared passes fill radii and frames;
/// meshing, foliage, wind and LOD read it without caring which backend grew
/// it.
///
/// Storage order is push order, and every parent precedes its children, so a
/// forward walk visits parents first and a reverse walk visits children first.
///
/// # Example
/// ```rust
/// use glam::Vec3;
/// use sylva_skeleton::{Attachment, Branch, BranchId, Node, Skeleton};
///
/// let trunk = BranchId::root(0);
/// let mut skeleton = Skeleton::new();
/// skeleton.push_branch(Branch {
///     id: trunk,
///     order: 0,
///     parent: None,
///     nodes: vec![Node::at(Vec3::ZERO), Node::at(Vec3::Z)],
/// })?;
/// skeleton.push_branch(Branch {
///     id: trunk.child(1, 0),
///     order: 1,
///     parent: Some(Attachment { parent: trunk, t: 0.5 }),
///     nodes: vec![Node::at(Vec3::new(0.0, 0.0, 0.5)), Node::at(Vec3::new(0.5, 0.0, 0.8))],
/// })?;
/// assert_eq!(skeleton.children(trunk).count(), 1);
/// # Ok::<(), sylva_skeleton::SkeletonError>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    branches: Vec<Branch>,
    index: BTreeMap<BranchId, usize>,
    /// Storage indices of each branch's direct children, in push order.
    children: Vec<Vec<usize>>,
    sites: Vec<Site>,
}

impl Skeleton {
    /// Creates an empty skeleton.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a branch after checking its structure.
    ///
    /// Requires at least two finite nodes with non-negative radii and no
    /// coincident neighbours, a new ID, a parent pushed earlier, an attachment
    /// parameter in `[0, 1]`, and a consistent order. Radii and frames are not
    /// required to be complete; see [`Self::validate`].
    ///
    /// # Errors
    ///
    /// Returns the first structural problem found; the skeleton is unchanged.
    pub fn push_branch(&mut self, branch: Branch) -> Result<(), SkeletonError> {
        let id = branch.id;
        if self.index.contains_key(&id) {
            return Err(SkeletonError::DuplicateBranch { branch: id });
        }
        let (expected, parent_index) = match branch.parent {
            None => (0, None),
            Some(Attachment { parent, t }) => {
                let Some(&parent_index) = self.index.get(&parent) else {
                    return Err(SkeletonError::MissingParent { branch: id, parent });
                };
                if !(0.0..=1.0).contains(&t) {
                    return Err(SkeletonError::ParameterOutOfRange { branch: parent, t });
                }
                (
                    self.branches[parent_index].order.saturating_add(1),
                    Some(parent_index),
                )
            }
        };
        if branch.order != expected {
            return Err(SkeletonError::OrderMismatch {
                branch: id,
                order: branch.order,
                expected,
            });
        }
        check_nodes(&branch, false)?;
        let index = self.branches.len();
        self.index.insert(id, index);
        if let Some(parent) = parent_index {
            self.children[parent].push(index);
        }
        self.children.push(Vec::new());
        self.branches.push(branch);
        Ok(())
    }

    /// Appends a site after checking that its branch exists and its values are
    /// finite with `t` in `[0, 1]`.
    ///
    /// # Errors
    ///
    /// Returns the problem found; the skeleton is unchanged.
    pub fn push_site(&mut self, site: Site) -> Result<(), SkeletonError> {
        if !self.index.contains_key(&site.branch) {
            return Err(SkeletonError::UnknownSiteBranch {
                branch: site.branch,
            });
        }
        if !(0.0..=1.0).contains(&site.t) {
            return Err(SkeletonError::ParameterOutOfRange {
                branch: site.branch,
                t: site.t,
            });
        }
        if !site.scale.is_finite() || !site.frame.is_orthonormal(FRAME_EPSILON) {
            return Err(SkeletonError::FrameNotOrthonormal {
                branch: site.branch,
                node: usize::MAX,
            });
        }
        self.sites.push(site);
        Ok(())
    }

    /// Checks that the skeleton is complete: structure as for
    /// [`Self::push_branch`], plus positive radii and orthonormal frames on
    /// every node.
    ///
    /// # Errors
    ///
    /// Returns the first problem in branch, then node, order.
    pub fn validate(&self) -> Result<(), SkeletonError> {
        for branch in &self.branches {
            check_nodes(branch, true)?;
        }
        Ok(())
    }

    /// Branches in parent-before-child order.
    #[must_use]
    pub fn branches(&self) -> &[Branch] {
        &self.branches
    }

    /// Sites in push order.
    #[must_use]
    pub fn sites(&self) -> &[Site] {
        &self.sites
    }

    /// The branch with `id`.
    #[must_use]
    pub fn branch(&self, id: BranchId) -> Option<&Branch> {
        self.index.get(&id).map(|&index| &self.branches[index])
    }

    /// Storage index of `id`.
    #[must_use]
    pub fn index_of(&self, id: BranchId) -> Option<usize> {
        self.index.get(&id).copied()
    }

    /// Direct children of `id`, in storage order.
    ///
    /// Runs in time proportional to the number of children: child lists are
    /// kept as branches are pushed.
    pub fn children(&self, id: BranchId) -> impl Iterator<Item = &Branch> + '_ {
        self.index
            .get(&id)
            .into_iter()
            .flat_map(|&index| self.child_indices(index))
            .map(|&child| &self.branches[child])
    }

    /// Storage indices of the direct children of the branch stored at
    /// `index`, in push order; empty for an index out of range.
    #[must_use]
    pub fn child_indices(&self, index: usize) -> &[usize] {
        self.children.get(index).map_or(&[], Vec::as_slice)
    }

    /// Mutable node access for the crate's passes, which keep structure intact.
    pub(crate) fn branches_mut(&mut self) -> &mut [Branch] {
        &mut self.branches
    }

    /// Summary counts and extents.
    #[must_use]
    pub fn stats(&self) -> SkeletonStats {
        let mut stats = SkeletonStats {
            branches: self.branches.len(),
            sites: self.sites.len(),
            min_radius: f32::INFINITY,
            max_radius: 0.0,
            ..SkeletonStats::default()
        };
        for branch in &self.branches {
            let order = branch.order as usize;
            if stats.branches_by_order.len() <= order {
                stats.branches_by_order.resize(order + 1, 0);
            }
            stats.branches_by_order[order] += 1;
            if branch.parent.is_none() {
                stats.roots += 1;
            }
            stats.nodes += branch.nodes.len();
            stats.total_length += branch.length();
            for node in &branch.nodes {
                stats.min_radius = stats.min_radius.min(node.radius);
                stats.max_radius = stats.max_radius.max(node.radius);
            }
        }
        if stats.nodes == 0 {
            stats.min_radius = 0.0;
        }
        stats
    }
}

/// Counts and extents of a skeleton, for reports and regression checks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkeletonStats {
    /// Number of branches.
    pub branches: usize,
    /// Number of root stems.
    pub roots: usize,
    /// Number of nodes over all branches.
    pub nodes: usize,
    /// Number of sites.
    pub sites: usize,
    /// Branch count per order; index 0 counts root stems.
    pub branches_by_order: Vec<usize>,
    /// Summed centerline length, in metres.
    pub total_length: f32,
    /// Smallest node radius (0 for an empty skeleton).
    pub min_radius: f32,
    /// Largest node radius.
    pub max_radius: f32,
}

fn check_nodes(branch: &Branch, complete: bool) -> Result<(), SkeletonError> {
    let id = branch.id;
    if branch.nodes.len() < 2 {
        return Err(SkeletonError::TooFewNodes {
            branch: id,
            count: branch.nodes.len(),
        });
    }
    for (index, node) in branch.nodes.iter().enumerate() {
        let finite = node.position.is_finite()
            && node.radius.is_finite()
            && node.age.is_finite()
            && node.frame.tangent.is_finite()
            && node.frame.normal.is_finite();
        if !finite {
            return Err(SkeletonError::NonFinite {
                branch: id,
                node: index,
            });
        }
        let radius_ok = if complete {
            node.radius > 0.0
        } else {
            node.radius >= 0.0
        };
        if !radius_ok {
            return Err(SkeletonError::InvalidRadius {
                branch: id,
                node: index,
                radius: node.radius,
            });
        }
        if complete && !node.frame.is_orthonormal(FRAME_EPSILON) {
            return Err(SkeletonError::FrameNotOrthonormal {
                branch: id,
                node: index,
            });
        }
    }
    for (index, pair) in branch.nodes.windows(2).enumerate() {
        if pair[0].position == pair[1].position {
            return Err(SkeletonError::ZeroLengthSegment {
                branch: id,
                node: index,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use glam::Vec3;

    use crate::{Attachment, Branch, BranchId, Node, Skeleton};

    fn branch(id: BranchId, order: u32, parent: Option<BranchId>) -> Branch {
        Branch {
            id,
            order,
            parent: parent.map(|parent| Attachment { parent, t: 0.5 }),
            nodes: vec![Node::at(Vec3::ZERO), Node::at(Vec3::Z)],
        }
    }

    #[test]
    fn child_lists_follow_push_order_per_parent() {
        let trunk = BranchId::root(0);
        let (a, b) = (trunk.child(1, 0), trunk.child(1, 1));
        let a1 = a.child(2, 0);
        let mut skeleton = Skeleton::new();
        skeleton.push_branch(branch(trunk, 0, None)).expect("trunk");
        skeleton.push_branch(branch(a, 1, Some(trunk))).expect("a");
        skeleton.push_branch(branch(a1, 2, Some(a))).expect("a1");
        skeleton.push_branch(branch(b, 1, Some(trunk))).expect("b");
        let ids = |skeleton: &Skeleton, id| skeleton.children(id).map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(&skeleton, trunk), [a, b]);
        assert_eq!(ids(&skeleton, a), [a1]);
        assert!(ids(&skeleton, b).is_empty());
        assert!(
            ids(&skeleton, BranchId::root(9)).is_empty(),
            "unknown IDs have none"
        );
        assert_eq!(skeleton.child_indices(0), [1, 3]);
        assert!(skeleton.child_indices(99).is_empty());
        // A rejected push leaves the child lists unchanged.
        assert!(skeleton.push_branch(branch(a1, 2, Some(a))).is_err());
        assert_eq!(ids(&skeleton, a), [a1]);
    }
}
