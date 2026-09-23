// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Grows a small keyed test tree, runs the shared passes, and writes debug
//! dumps for visual review.
//!
//! ```sh
//! cargo run -p skeleton_dump -- target/skeleton-dump
//! blender --background --python examples/skeleton_dump/tools/render.py -- target/skeleton-dump
//! ```
//!
//! Outputs:
//! - `skeleton.json`: every branch (ID, order, parent, nodes with position,
//!   radius and frame) and every site; the Blender script reads this.
//! - `skeleton.obj`: centerlines as polylines, one object per branch order,
//!   for any OBJ viewer.
//! - `stats.json`: skeleton stats and pass reports.
//!
//! The generator here is deliberately minimal: a fixed number of levels with
//! golden-angle phyllotaxis, keyed jitter, and upward or downward bending. It
//! exercises the IR and passes; it is not a species. The real hierarchical
//! generator is `sylva_grow`.

use std::path::PathBuf;
use std::time::Instant;

use glam::{Quat, Vec3};
use skeleton_dump::{skeleton_json, skeleton_obj};
use sylva_skeleton::keyed::tag;
use sylva_skeleton::passes::{FrameParams, PipeModel, compute_frames, pipe_model_radii};
use sylva_skeleton::{Attachment, Branch, BranchId, Frame, Node, Site, Skeleton, SkeletonError};

/// Growth settings for one level below the trunk.
#[derive(Copy, Clone, Debug)]
struct Level {
    /// Children per parent branch.
    children: u32,
    /// Child length relative to its parent.
    length_ratio: f32,
    /// Angle between parent tangent and child start direction, radians.
    angle: f32,
    /// Vertical bend per metre: positive curls up, negative droops.
    bend: f32,
}

const SEED: u64 = 0x5EED;
const TRUNK_LENGTH: f32 = 6.0;
const NODES_PER_METRE: f32 = 6.0;
const GOLDEN_ANGLE: f32 = 2.399_963_2;

const LEVELS: [Level; 3] = [
    Level {
        children: 9,
        length_ratio: 0.6,
        angle: 0.95,
        bend: 0.12,
    },
    Level {
        children: 6,
        length_ratio: 0.5,
        angle: 0.8,
        bend: -0.08,
    },
    Level {
        children: 4,
        length_ratio: 0.5,
        angle: 0.7,
        bend: 0.0,
    },
];

/// Samples a bent centerline starting at `start` heading along `direction`.
fn centerline(id: BranchId, start: Vec3, direction: Vec3, length: f32, bend: f32) -> Vec<Node> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "branch lengths are a few metres, so the segment count is small and positive"
    )]
    let segments = ((length * NODES_PER_METRE).ceil() as usize).max(2);
    let step = length / segments as f32;
    let mut position = start;
    let mut heading = direction.normalize();
    let mut nodes = vec![Node::at(position)];
    for segment in 0..segments {
        // Keyed gnarl: a small random turn per segment, then the vertical bend.
        let key = id.key(SEED).with(tag("gnarl")).with(segment as u64);
        let axis = Vec3::new(
            key.with(0).signed_unit_f32(),
            key.with(1).signed_unit_f32(),
            key.with(2).signed_unit_f32(),
        );
        if let Some(axis) = axis.try_normalize() {
            heading = Quat::from_axis_angle(axis, 0.06) * heading;
        }
        heading = (heading + Vec3::Z * (bend * step)).normalize();
        position += heading * step;
        nodes.push(Node::at(position));
    }
    nodes
}

/// The parent's tangent at `t`, from its sampled centerline.
fn parent_tangent(parent: &Branch, t: f32) -> Vec3 {
    let before = parent.sample((t - 0.02).max(0.0)).position;
    let after = parent.sample((t + 0.02).min(1.0)).position;
    (after - before).normalize()
}

/// A direction perpendicular to `tangent`, rotated by the golden angle per
/// ordinal plus a keyed phase.
fn side_direction(tangent: Vec3, ordinal: u32, phase: f32) -> Vec3 {
    let (frame, _) =
        Frame::from_tangent_or_axis(tangent, Vec3::X).expect("tangent is finite and nonzero");
    let angle = GOLDEN_ANGLE * ordinal as f32 + phase * core::f32::consts::TAU;
    frame.normal * angle.cos() + frame.binormal() * angle.sin()
}

/// Grows the test tree: a trunk, then [`LEVELS`] of children.
fn grow() -> Result<Skeleton, SkeletonError> {
    let mut skeleton = Skeleton::new();
    let trunk = BranchId::root(0);
    skeleton.push_branch(Branch {
        id: trunk,
        order: 0,
        parent: None,
        nodes: centerline(trunk, Vec3::ZERO, Vec3::Z, TRUNK_LENGTH, 0.0),
    })?;

    let mut parents = vec![trunk];
    for (level_index, level) in LEVELS.iter().enumerate() {
        let lineage = level_index as u64 + 1;
        let mut next = Vec::new();
        for parent_id in parents {
            let parent = skeleton
                .branch(parent_id)
                .expect("parent was pushed")
                .clone();
            let parent_length = parent.length();
            // Children spread over the upper part of the parent; the trunk
            // keeps a clear bole.
            let lowest = if parent.order == 0 { 0.35 } else { 0.2 };
            for ordinal in 0..level.children {
                let id = parent_id.child(lineage, u64::from(ordinal));
                let key = id.key(SEED);
                let slot = (ordinal as f32 + 0.2 + key.with(tag("t")).unit_f32() * 0.6)
                    / level.children as f32;
                let t = (lowest + (1.0 - lowest) * slot).min(0.98);
                let sample = parent.sample(t);
                let tangent = parent_tangent(&parent, t);
                let side = side_direction(tangent, ordinal, key.with(tag("phase")).unit_f32());
                let angle = level.angle * key.with(tag("angle")).range_f32(0.85, 1.15);
                let direction = (tangent * angle.cos() + side * angle.sin()).normalize();
                // Shorter children toward the tip, like most crowns.
                let taper = 1.0 - 0.6 * t;
                let length = parent_length
                    * level.length_ratio
                    * taper
                    * key.with(tag("length")).range_f32(0.8, 1.2);
                skeleton.push_branch(Branch {
                    id,
                    order: parent.order + 1,
                    parent: Some(Attachment {
                        parent: parent_id,
                        t,
                    }),
                    nodes: centerline(id, sample.position, direction, length, level.bend),
                })?;
                next.push(id);
            }
        }
        parents = next;
    }
    Ok(skeleton)
}

/// Adds one leaf site at the tip of every terminal branch.
fn add_tip_sites(skeleton: &mut Skeleton) -> Result<usize, SkeletonError> {
    let tips: Vec<_> = skeleton
        .branches()
        .iter()
        .filter(|branch| skeleton.children(branch.id).next().is_none())
        .map(|branch| (branch.id, branch.sample(1.0).frame))
        .collect();
    for (branch, frame) in &tips {
        skeleton.push_site(Site {
            branch: *branch,
            ordinal: 0,
            kind: 0,
            t: 1.0,
            frame: *frame,
            scale: 1.0,
        })?;
    }
    Ok(tips.len())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "target/skeleton-dump".to_owned()),
    );
    std::fs::create_dir_all(&out_dir)?;

    let started = Instant::now();
    let mut skeleton = grow()?;
    let grow_time = started.elapsed();

    let started = Instant::now();
    // A thicker twig than the default keeps the renders readable at this
    // small tip count; real crowns have thousands of tips.
    let pipe = pipe_model_radii(
        &mut skeleton,
        &PipeModel {
            tip_radius: 0.01,
            ..PipeModel::default()
        },
    )?;
    let frames = compute_frames(&mut skeleton, &FrameParams::default());
    let tip_sites = add_tip_sites(&mut skeleton)?;
    skeleton.validate()?;
    let pass_time = started.elapsed();

    let stats = skeleton.stats();
    let by_order: Vec<String> = stats
        .branches_by_order
        .iter()
        .map(ToString::to_string)
        .collect();
    let stats_json = format!(
        "{{\"branches\":{},\"branches_by_order\":[{}],\"nodes\":{},\"sites\":{},\
         \"total_length_m\":{},\"min_radius_m\":{},\"max_radius_m\":{},\
         \"pipe\":{{\"branches\":{},\"nodes\":{},\"max_radius_m\":{}}},\
         \"frames\":{{\"branches\":{},\"nodes\":{},\"fallback_normals\":{}}},\
         \"tip_sites\":{tip_sites},\"grow_us\":{},\"passes_us\":{}}}\n",
        stats.branches,
        by_order.join(","),
        stats.nodes,
        stats.sites,
        stats.total_length,
        stats.min_radius,
        stats.max_radius,
        pipe.branches,
        pipe.nodes,
        pipe.max_radius,
        frames.branches,
        frames.nodes,
        frames.fallback_normals,
        grow_time.as_micros(),
        pass_time.as_micros(),
    );

    std::fs::write(out_dir.join("skeleton.json"), skeleton_json(&skeleton)?)?;
    std::fs::write(out_dir.join("skeleton.obj"), skeleton_obj(&skeleton)?)?;
    std::fs::write(out_dir.join("stats.json"), &stats_json)?;
    print!("{stats_json}");
    println!("wrote {}", out_dir.display());
    Ok(())
}
