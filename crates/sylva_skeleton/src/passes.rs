// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Post-passes shared by every growth backend.
//!
//! Growth produces topology and centerlines; these passes derive the values
//! downstream stages rely on, identically for every backend.

use alloc::vec::Vec;

use glam::Vec3;

use crate::{Frame, Skeleton, SkeletonError};

/// Parameters for [`compute_frames`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FrameParams {
    /// Direction the first normal of each root stem points toward, made
    /// perpendicular to the stem. Default `+X`.
    pub root_reference: Vec3,
}

impl Default for FrameParams {
    fn default() -> Self {
        Self {
            root_reference: Vec3::X,
        }
    }
}

/// Work done by [`compute_frames`].
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct FrameReport {
    /// Branches framed.
    pub branches: usize,
    /// Nodes framed.
    pub nodes: usize,
    /// Branches whose first normal could not use its reference (the root
    /// reference, or the parent's normal) and fell back to a world axis.
    pub fallback_normals: usize,
}

/// Computes rotation-minimizing frames on every node.
///
/// Node tangents are central differences (one-sided at the ends). A root
/// stem's first normal is [`FrameParams::root_reference`] made perpendicular to
/// its tangent. A child's first normal is its parent's normal at the
/// attachment point, made perpendicular to the child's tangent, so bark seams
/// and ring phase continue from parent to child. Normals then travel along
/// each branch by double reflection ([`Frame::transport`]).
///
/// Branches are processed in storage order, which puts parents first.
/// Positions are unchanged, and the skeleton's structural checks guarantee
/// every tangent exists.
pub fn compute_frames(skeleton: &mut Skeleton, params: &FrameParams) -> FrameReport {
    let mut report = FrameReport::default();
    for index in 0..skeleton.branches().len() {
        let reference = match skeleton.branches()[index].parent {
            None => params.root_reference,
            Some(attachment) => skeleton
                .branch(attachment.parent)
                .map_or(params.root_reference, |parent| {
                    parent.sample(attachment.t).frame.normal
                }),
        };
        let branch = &mut skeleton.branches_mut()[index];
        let positions: Vec<Vec3> = branch.nodes.iter().map(|node| node.position).collect();
        let tangent = |i: usize| {
            let last = positions.len() - 1;
            let (a, b) = match i {
                0 => (positions[0], positions[1]),
                i if i == last => (positions[last - 1], positions[last]),
                i => (positions[i - 1], positions[i + 1]),
            };
            // Central differences can cancel on a hairpin; fall back to the
            // forward segment, which is non-zero by the structural checks.
            (b - a)
                .try_normalize()
                .or_else(|| {
                    (positions[(i + 1).min(last)] - positions[i.min(last - 1)]).try_normalize()
                })
                .unwrap_or(Vec3::Z)
        };
        let (mut frame, fell_back) = Frame::from_tangent_or_axis(tangent(0), reference)
            .expect("structurally valid branches have a first tangent");
        if fell_back {
            report.fallback_normals += 1;
        }
        branch.nodes[0].frame = frame;
        for i in 1..positions.len() {
            frame = frame.transport(positions[i - 1], positions[i], tangent(i));
            branch.nodes[i].frame = frame;
        }
        report.branches += 1;
        report.nodes += positions.len();
    }
    report
}

/// Parameters for [`pipe_model_radii`].
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PipeModel {
    /// Radius at every branch tip, in metres.
    pub tip_radius: f32,
    /// Exponent `e` of `r_parent^e = Σ r_child^e`. Leonardo's rule is 2;
    /// measured trees mostly fall between 2 and 3.
    pub exponent: f32,
    /// Unmodeled shoots per metre of centerline, each carrying a tip's flow.
    ///
    /// A skeleton stops at some level of twigs, but the real tree carries
    /// finer shoots and leaves beyond it. Without this, trunk girth can only
    /// be matched by inflating `tip_radius`, which also makes every twig too
    /// thick. With it, a branch's flow grows by `tip_radius^e *
    /// shoots_per_metre` per metre from its tip, so tips stay thin, branches
    /// taper even without modeled children, and trunks keep their girth.
    /// Zero is the classic tip-only model.
    pub shoots_per_metre: f32,
    /// Relative radius growth per metre below a branch's lowest child.
    ///
    /// The pipe model carries no new flow below the last fork, so a bare
    /// bole comes out as a constant-radius column. Real stems keep
    /// thickening toward the ground, because the bending load of the crown
    /// grows with lever arm. A node `d` metres below the branch's lowest
    /// child has its radius scaled by `1 + bole_taper * d`, and the tapered
    /// base feeds the parent's flow, so parents stay at least as thick as
    /// their children. Branches without children are unaffected. Zero
    /// disables it.
    pub bole_taper: f32,
}

impl Default for PipeModel {
    fn default() -> Self {
        Self {
            tip_radius: 0.004,
            exponent: 2.3,
            shoots_per_metre: 0.0,
            bole_taper: 0.0,
        }
    }
}

/// Work done by [`pipe_model_radii`].
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct PipeReport {
    /// Branches updated.
    pub branches: usize,
    /// Nodes updated.
    pub nodes: usize,
    /// Largest radius assigned (a root stem's base).
    pub max_radius: f32,
}

/// Assigns radii by the pipe model, from the tips down.
///
/// Every tip carries `tip_radius^e` of "flow". Walking a branch from tip to
/// base, a node carries the tip's flow, the flow of the unmodeled shoots along
/// the centerline between it and the tip ([`PipeModel::shoots_per_metre`]),
/// and the base flow of every child attached at or beyond it
/// (`child.t >= node.t`); its radius is that flow to the power `1/e`, scaled
/// below the lowest child by [`PipeModel::bole_taper`]. A branch's base
/// radius is therefore the combined size of everything it supports, which is
/// what makes forks read correctly.
///
/// Branches are processed in reverse storage order, so children are sized
/// before their parents. Existing radii are overwritten.
///
/// # Errors
///
/// [`SkeletonError::InvalidParameter`] when `tip_radius` is not positive and
/// finite, `exponent` is not at least 1 and finite, or `shoots_per_metre` or
/// `bole_taper` is negative or not finite; the skeleton is unchanged.
pub fn pipe_model_radii(
    skeleton: &mut Skeleton,
    params: &PipeModel,
) -> Result<PipeReport, SkeletonError> {
    if !(params.tip_radius.is_finite() && params.tip_radius > 0.0) {
        return Err(SkeletonError::InvalidParameter { name: "tip_radius" });
    }
    if !(params.exponent.is_finite() && params.exponent >= 1.0) {
        return Err(SkeletonError::InvalidParameter { name: "exponent" });
    }
    if !(params.shoots_per_metre.is_finite() && params.shoots_per_metre >= 0.0) {
        return Err(SkeletonError::InvalidParameter {
            name: "shoots_per_metre",
        });
    }
    if !(params.bole_taper.is_finite() && params.bole_taper >= 0.0) {
        return Err(SkeletonError::InvalidParameter { name: "bole_taper" });
    }
    let shoot_flow = libm::powf(params.tip_radius, params.exponent) * params.shoots_per_metre;
    let e = params.exponent;
    let tip_flow = libm::powf(params.tip_radius, e);
    let count = skeleton.branches().len();
    // Base flow of each branch, filled as children are sized first.
    let mut base_flow = alloc::vec![0.0_f32; count];
    let mut report = PipeReport::default();
    for index in (0..count).rev() {
        // (t, flow) of each direct child, sorted by t descending; the stable
        // sort keeps storage order for equal t.
        let mut children: Vec<(f32, f32)> = skeleton
            .child_indices(index)
            .iter()
            .map(|&child| {
                let t = skeleton.branches()[child]
                    .parent
                    .expect("children have attachments")
                    .t;
                (t, base_flow[child])
            })
            .collect();
        children.sort_by(|a, b| b.0.total_cmp(&a.0));

        let branch = &mut skeleton.branches_mut()[index];
        let lengths = branch.arc_lengths();
        let total = *lengths.last().expect("branches have nodes");
        // Arc length of the lowest child; below it the bole tapers.
        let lowest = children.last().map_or(f32::NEG_INFINITY, |c| c.0 * total);
        let mut flow = tip_flow;
        let mut next_child = 0;
        let mut radius = 0.0;
        for (node_index, node) in branch.nodes.iter_mut().enumerate().rev() {
            let t = if total > 0.0 {
                lengths[node_index] / total
            } else {
                0.0
            };
            while next_child < children.len() && children[next_child].0 >= t {
                flow += children[next_child].1;
                next_child += 1;
            }
            let shoots = shoot_flow * (total - lengths[node_index]);
            let below = (lowest - lengths[node_index]).max(0.0);
            radius = libm::powf(flow + shoots, 1.0 / e) * (1.0 + params.bole_taper * below);
            node.radius = radius;
            report.max_radius = report.max_radius.max(radius);
        }
        // The base node sits at arc length zero, so its radius already
        // carries every shoot and the full taper.
        base_flow[index] = libm::powf(radius, e);
        report.branches += 1;
        report.nodes += branch.nodes.len();
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use glam::Vec3;

    use super::{FrameParams, PipeModel, compute_frames, pipe_model_radii};
    use crate::{Attachment, Branch, BranchId, Node, Skeleton, SkeletonError};

    fn line(from: Vec3, to: Vec3, count: usize) -> Vec<Node> {
        (0..count)
            .map(|i| Node::at(from.lerp(to, i as f32 / (count - 1) as f32)))
            .collect()
    }

    /// A vertical trunk with two equal children at t = 0.5 and one at t = 1.
    fn fork() -> Skeleton {
        let trunk = BranchId::root(0);
        let mut skeleton = Skeleton::new();
        skeleton
            .push_branch(Branch {
                id: trunk,
                order: 0,
                parent: None,
                nodes: line(Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0), 5),
            })
            .expect("trunk");
        for (ordinal, (t, direction)) in [(0.5, Vec3::X), (0.5, -Vec3::X), (1.0, Vec3::Y)]
            .into_iter()
            .enumerate()
        {
            let base = Vec3::new(0.0, 0.0, 2.0 * t);
            skeleton
                .push_branch(Branch {
                    id: trunk.child(1, ordinal as u64),
                    order: 1,
                    parent: Some(Attachment { parent: trunk, t }),
                    nodes: line(base, base + direction + Vec3::Z * 0.5, 3),
                })
                .expect("child");
        }
        skeleton
    }

    #[test]
    fn pipe_model_accumulates_child_flow_toward_the_base() {
        let mut skeleton = fork();
        let params = PipeModel {
            tip_radius: 0.01,
            exponent: 2.0,
            shoots_per_metre: 0.0,
            bole_taper: 0.0,
        };
        let report = pipe_model_radii(&mut skeleton, &params).expect("radii");
        assert_eq!(report.branches, 4);
        let r = |flow_tips: f32| libm::sqrtf(flow_tips) * 0.01;
        let trunk = &skeleton.branches()[0];
        // Tip node (t = 1): own tip + the child attached at t = 1.
        assert!((trunk.nodes[4].radius - r(2.0)).abs() < 1e-6);
        // t = 0.75: unchanged.
        assert!((trunk.nodes[3].radius - r(2.0)).abs() < 1e-6);
        // t = 0.5 and below: all four tips.
        assert!((trunk.nodes[2].radius - r(4.0)).abs() < 1e-6);
        assert!((trunk.nodes[0].radius - r(4.0)).abs() < 1e-6);
        for child in &skeleton.branches()[1..] {
            assert!(child.nodes.iter().all(|n| (n.radius - 0.01).abs() < 1e-7));
        }
        assert!((report.max_radius - r(4.0)).abs() < 1e-6);
    }

    #[test]
    fn pipe_model_rejects_bad_parameters_without_changes() {
        let mut skeleton = fork();
        let before = skeleton.branches().to_vec();
        for (params, name) in [
            (
                PipeModel {
                    tip_radius: 0.0,
                    exponent: 2.0,
                    shoots_per_metre: 0.0,
                    bole_taper: 0.0,
                },
                "tip_radius",
            ),
            (
                PipeModel {
                    tip_radius: 0.01,
                    exponent: 0.5,
                    shoots_per_metre: 0.0,
                    bole_taper: 0.0,
                },
                "exponent",
            ),
        ] {
            assert_eq!(
                pipe_model_radii(&mut skeleton, &params),
                Err(SkeletonError::InvalidParameter { name })
            );
        }
        assert_eq!(skeleton.branches(), &before[..]);
    }

    #[test]
    fn frames_are_orthonormal_and_continue_from_the_parent() {
        let mut skeleton = fork();
        pipe_model_radii(&mut skeleton, &PipeModel::default()).expect("radii");
        let report = compute_frames(&mut skeleton, &FrameParams::default());
        assert_eq!(report.branches, 4);
        assert_eq!(report.nodes, 5 + 3 * 3);
        assert_eq!(skeleton.validate(), Ok(()));
        let trunk = &skeleton.branches()[0];
        assert!(
            trunk
                .nodes
                .iter()
                .all(|n| (n.frame.normal - Vec3::X).length() < 1e-6)
        );
        // The +Y child's first normal is the trunk normal (+X) projected
        // perpendicular to its tangent; +X already is.
        let y_child = &skeleton.branches()[3];
        assert!((y_child.nodes[0].frame.normal - Vec3::X).length() < 1e-5);
        // The +X child starts parallel to the trunk normal: its projection
        // is still well defined because the tangent also rises.
        assert_eq!(report.fallback_normals, 0);
    }

    #[test]
    fn a_child_along_the_parent_normal_falls_back_and_reports_it() {
        let trunk = BranchId::root(0);
        let mut skeleton = Skeleton::new();
        skeleton
            .push_branch(Branch {
                id: trunk,
                order: 0,
                parent: None,
                nodes: line(Vec3::ZERO, Vec3::Z, 2),
            })
            .expect("trunk");
        skeleton
            .push_branch(Branch {
                id: trunk.child(1, 0),
                order: 1,
                parent: Some(Attachment {
                    parent: trunk,
                    t: 0.5,
                }),
                nodes: line(Vec3::Z * 0.5, Vec3::new(1.0, 0.0, 0.5), 2),
            })
            .expect("child");
        let report = compute_frames(&mut skeleton, &FrameParams::default());
        assert_eq!(report.fallback_normals, 1);
        assert!(skeleton.branches()[1].nodes[0].frame.is_orthonormal(1e-5));
    }

    #[test]
    fn structure_is_checked_on_push() {
        let trunk = BranchId::root(0);
        let mut skeleton = Skeleton::new();
        let nodes = line(Vec3::ZERO, Vec3::Z, 3);
        assert_eq!(
            skeleton.push_branch(Branch {
                id: trunk,
                order: 1,
                parent: None,
                nodes: nodes.clone(),
            }),
            Err(SkeletonError::OrderMismatch {
                branch: trunk,
                order: 1,
                expected: 0
            })
        );
        assert_eq!(
            skeleton.push_branch(Branch {
                id: trunk,
                order: 0,
                parent: None,
                nodes: vec![Node::at(Vec3::ZERO)],
            }),
            Err(SkeletonError::TooFewNodes {
                branch: trunk,
                count: 1
            })
        );
        assert_eq!(
            skeleton.push_branch(Branch {
                id: trunk,
                order: 0,
                parent: None,
                nodes: vec![Node::at(Vec3::ZERO), Node::at(Vec3::ZERO)],
            }),
            Err(SkeletonError::ZeroLengthSegment {
                branch: trunk,
                node: 0
            })
        );
        let child = trunk.child(1, 0);
        assert_eq!(
            skeleton.push_branch(Branch {
                id: child,
                order: 1,
                parent: Some(Attachment {
                    parent: trunk,
                    t: 0.5
                }),
                nodes: nodes.clone(),
            }),
            Err(SkeletonError::MissingParent {
                branch: child,
                parent: trunk
            })
        );
        skeleton
            .push_branch(Branch {
                id: trunk,
                order: 0,
                parent: None,
                nodes: nodes.clone(),
            })
            .expect("trunk");
        assert_eq!(
            skeleton.push_branch(Branch {
                id: trunk,
                order: 0,
                parent: None,
                nodes: nodes.clone(),
            }),
            Err(SkeletonError::DuplicateBranch { branch: trunk })
        );
        assert_eq!(
            skeleton.push_branch(Branch {
                id: child,
                order: 1,
                parent: Some(Attachment {
                    parent: trunk,
                    t: 1.5
                }),
                nodes,
            }),
            Err(SkeletonError::ParameterOutOfRange {
                branch: trunk,
                t: 1.5
            })
        );
        // Zero radii are structurally fine but incomplete.
        assert!(matches!(
            skeleton.validate(),
            Err(SkeletonError::InvalidRadius { .. })
        ));
    }

    #[test]
    fn unmodeled_shoots_taper_branches_and_thicken_their_parents() {
        let trunk = BranchId::root(0);
        let mut skeleton = Skeleton::new();
        skeleton
            .push_branch(Branch {
                id: trunk,
                order: 0,
                parent: None,
                nodes: (0..=4)
                    .map(|i| Node::at(Vec3::new(0.0, 0.0, i as f32)))
                    .collect(),
            })
            .expect("trunk");
        let params = PipeModel {
            tip_radius: 0.01,
            exponent: 2.0,
            shoots_per_metre: 3.0,
            bole_taper: 0.0,
        };
        pipe_model_radii(&mut skeleton, &params).expect("radii");
        let radii: Vec<f32> = skeleton.branches()[0]
            .nodes
            .iter()
            .map(|n| n.radius)
            .collect();
        assert!(
            (radii[4] - 0.01).abs() < 1e-6,
            "the tip keeps the tip radius"
        );
        // Four metres of shoots at 3 per metre add 12 tips' flow.
        let expected = libm::sqrtf(0.01 * 0.01 * 13.0);
        assert!(
            (radii[0] - expected).abs() < 1e-6,
            "{} vs {expected}",
            radii[0]
        );
        assert!(
            radii.windows(2).all(|w| w[0] > w[1]),
            "tapers toward the tip"
        );
        let err = pipe_model_radii(
            &mut skeleton,
            &PipeModel {
                shoots_per_metre: -1.0,
                ..params
            },
        );
        assert_eq!(
            err,
            Err(SkeletonError::InvalidParameter {
                name: "shoots_per_metre"
            })
        );
    }

    #[test]
    fn bole_taper_thickens_the_stem_below_its_lowest_child() {
        let mut plain = fork();
        let params = PipeModel {
            tip_radius: 0.01,
            exponent: 2.0,
            shoots_per_metre: 0.0,
            bole_taper: 0.5,
        };
        pipe_model_radii(
            &mut plain,
            &PipeModel {
                bole_taper: 0.0,
                ..params
            },
        )
        .expect("radii");
        let mut tapered = fork();
        pipe_model_radii(&mut tapered, &params).expect("radii");
        let (plain, tapered) = (&plain.branches()[0], &tapered.branches()[0]);
        // The lowest children sit at t = 0.5, one metre up: nodes 2..=4 are
        // unchanged, node 1 is half a metre below, node 0 a whole metre.
        for i in 2..5 {
            assert!((tapered.nodes[i].radius - plain.nodes[i].radius).abs() < 1e-7);
        }
        assert!((tapered.nodes[1].radius - plain.nodes[1].radius * 1.25).abs() < 1e-6);
        assert!((tapered.nodes[0].radius - plain.nodes[0].radius * 1.5).abs() < 1e-6);
        let err = pipe_model_radii(
            &mut fork(),
            &PipeModel {
                bole_taper: -0.1,
                ..params
            },
        );
        assert_eq!(
            err,
            Err(SkeletonError::InvalidParameter { name: "bole_taper" })
        );
    }
}
