// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The hierarchical generator.

use alloc::vec::Vec;
use core::f32::consts::{PI, TAU};

use sylva_skeleton::glam::{Quat, Vec3};
use sylva_skeleton::keyed::{Key, SignedUnit, tag};
use sylva_skeleton::passes::{
    FrameParams, FrameReport, PipeModel, PipeReport, compute_frames, pipe_model_radii,
};
use sylva_skeleton::{Attachment, Branch, BranchId, Frame, Node, Site, Skeleton};

use crate::{Arrangement, Count, Envelope, GrowError, Hierarchy, Level, Shape, Sites};

/// Lineage keys are `level index + 1`, so the trunk's children use lineage 1.
const TRUNK_LINEAGE_OFFSET: u64 = 1;

/// A grown tree and what it took.
#[derive(Clone, Debug)]
pub struct Grown {
    /// The skeleton, with pipe-model radii, frames and sites.
    pub skeleton: Skeleton,
    /// Deterministic work counts.
    pub report: GrowReport,
}

/// Deterministic counts from one [`grow()`] call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GrowReport {
    /// Branches kept per level; index 0 is the trunk.
    pub branches_by_level: Vec<usize>,
    /// Centerline nodes across all branches.
    pub nodes: usize,
    /// Branches shortened where they left the envelope or reached the ground.
    pub truncated: usize,
    /// Branches removed because pruning left too little of them.
    pub removed: usize,
    /// Children skipped because their length rounded to nothing.
    pub skipped: usize,
    /// Foliage sites placed.
    pub sites: usize,
    /// The final pipe-model pass.
    pub pipe: PipeReport,
    /// The final frame pass.
    pub frames: FrameReport,
}

/// Grows `hierarchy` from `seed`.
///
/// Every random decision draws from the keyed hash of the seed, the deciding
/// branch's [`BranchId`] and a purpose tag. Branch IDs follow the generation
/// path (parent, level, ordinal), so changing one level's parameters leaves
/// every branch of the levels above it bit-identical in position and ID. Pipe
/// radii are the exception: they depend on everything a branch supports.
///
/// Levels grow breadth-first. After each level, frames are recomputed so the
/// next level's azimuths are measured from each parent's rotation-minimizing
/// normal. Radii, final frames and sites follow the last level.
///
/// # Errors
///
/// [`GrowError::InvalidParameter`] for out-of-range parameters, checked before
/// any growth; [`GrowError::Skeleton`] if the result fails skeleton checks,
/// which indicates a generator bug.
pub fn grow(hierarchy: &Hierarchy, seed: u64) -> Result<Grown, GrowError> {
    crate::validate::check(hierarchy)?;
    let mut skeleton = Skeleton::new();
    let mut report = GrowReport::default();
    let step = hierarchy.segment_length;

    let trunk_id = BranchId::root(0);
    let trunk_key = trunk_id.key(seed);
    let trunk = &hierarchy.trunk;
    let length = trunk.length * (1.0 + trunk.length_jitter * signed(trunk_key, "length"));
    let lean = trunk.lean * trunk_key.with(tag("lean")).unit_f32();
    let lean_azimuth = TAU * trunk_key.with(tag("lean.azimuth")).unit_f32();
    let direction = Quat::from_axis_angle(
        Vec3::new(-libm::sinf(lean_azimuth), libm::cosf(lean_azimuth), 0.0),
        lean,
    ) * Vec3::Z;
    let nodes = centerline(Vec3::ZERO, direction, length, &trunk.shape, trunk_key, step);
    skeleton.push_branch(Branch {
        id: trunk_id,
        order: 0,
        parent: None,
        nodes,
    })?;
    report.branches_by_level.push(1);

    let mut parents = alloc::vec![trunk_id];
    for (level_index, level) in hierarchy.levels.iter().enumerate() {
        compute_frames(&mut skeleton, &FrameParams::default());
        let lineage = level_index as u64 + TRUNK_LINEAGE_OFFSET;
        let envelope = hierarchy
            .envelope
            .as_ref()
            .filter(|e| level_index >= e.from_level as usize)
            .map(|e| (e, Key::new(seed).with(tag("envelope"))));
        let mut next = Vec::new();
        for parent_id in parents {
            let parent = skeleton.branch(parent_id).expect("parents were pushed");
            let (mut children, skipped) = place_children(parent, level, lineage, seed);
            if level.balance > 0.0 {
                balance(&mut children, level.balance);
            }
            report.skipped += skipped;
            let parent_order = parent.order;
            for child in children {
                let key = child.id.key(seed);
                let mut nodes = centerline(
                    child.start,
                    child.direction,
                    child.length,
                    &level.shape,
                    key,
                    step,
                );
                match prune(&mut nodes, child.length, envelope) {
                    Pruned::Kept => {}
                    Pruned::Truncated => report.truncated += 1,
                    Pruned::Removed => {
                        report.removed += 1;
                        continue;
                    }
                }
                skeleton.push_branch(Branch {
                    id: child.id,
                    order: parent_order + 1,
                    parent: Some(Attachment {
                        parent: parent_id,
                        t: child.t,
                    }),
                    nodes,
                })?;
                next.push(child.id);
            }
        }
        report.branches_by_level.push(next.len());
        parents = next;
    }

    report.pipe = pipe_model_radii(
        &mut skeleton,
        &PipeModel {
            tip_radius: hierarchy.radii.tip_radius,
            exponent: hierarchy.radii.exponent,
            shoots_per_metre: hierarchy.radii.shoots_per_metre,
            bole_taper: hierarchy.radii.bole_taper,
        },
    )?;
    report.frames = compute_frames(&mut skeleton, &FrameParams::default());
    report.sites = place_sites(&mut skeleton, hierarchy, seed)?;
    report.nodes = skeleton.branches().iter().map(|b| b.nodes.len()).sum();
    skeleton.validate()?;
    Ok(Grown { skeleton, report })
}

/// Rebalances sibling lengths against their combined horizontal lean.
///
/// Each child's horizontal reach is its direction's horizontal part times its
/// length. With `f` the net reach over the total reach and `b` the net
/// direction, a child whose reach points along `b` by `cos` is scaled by
/// `1 - strength * f * cos`, so the heavy side shortens and the light side
/// lengthens. IDs and every keyed decision are untouched.
fn balance(children: &mut [Placement], strength: f32) {
    let reach = |c: &Placement| Vec3::new(c.direction.x, c.direction.y, 0.0) * c.length;
    let net: Vec3 = children.iter().map(reach).sum();
    let total: f32 = children.iter().map(|c| reach(c).length()).sum();
    let Some(towards) = net.try_normalize() else {
        return;
    };
    if total <= 0.0 {
        return;
    }
    let imbalance = net.length() / total;
    for child in children {
        let along = reach(child).try_normalize().map_or(0.0, |r| r.dot(towards));
        child.length *= (1.0 - strength * imbalance * along).max(0.25);
    }
}

/// A value in `[-1, 1)` for `purpose` under `key`.
fn signed(key: Key, purpose: &str) -> f32 {
    key.with(tag(purpose)).signed_unit_f32()
}

/// Rounds `x` down or up with probability equal to its fractional part.
fn keyed_round(x: f32, key: Key) -> u32 {
    let floor = libm::floorf(x);
    let up = key.unit_f32() < x - floor;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "counts are validated finite and non-negative, and small"
    )]
    let base = floor as u32;
    base + u32::from(up)
}

/// One child's placement on its parent.
struct Placement {
    id: BranchId,
    t: f32,
    start: Vec3,
    direction: Vec3,
    length: f32,
}

/// Places `level`'s children on `parent`; also returns how many were skipped
/// for having no length.
fn place_children(
    parent: &Branch,
    level: &Level,
    lineage: u64,
    seed: u64,
) -> (Vec<Placement>, usize) {
    let parent_key = parent.id.key(seed).with(lineage);
    let parent_length = parent.length();
    let [lo, hi] = level.span;
    let count = match level.count {
        Count::Fixed(n) => n,
        Count::Range { min, max } => {
            #[expect(clippy::cast_precision_loss, reason = "counts are small")]
            let spread = (max - min) as f32 + 1.0;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "unit_f32 is in [0, 1), so the draw is in [0, spread)"
            )]
            let draw = (parent_key.with(tag("count")).unit_f32() * spread) as u32;
            min + draw.min(max - min)
        }
        Count::PerMetre(density) => keyed_round(
            density * parent_length * (hi - lo),
            parent_key.with(tag("count")),
        ),
    };
    let (per_node, node_step) = match level.arrangement {
        Arrangement::Spiral { divergence } => (1, divergence),
        Arrangement::Alternate => (1, PI),
        Arrangement::Opposite => (2, PI * 0.5),
        Arrangement::Whorled { per_node } => (per_node, PI / per_node as f32),
    };
    let node_count = count.div_ceil(per_node);
    let phase = TAU * parent_key.with(tag("phase")).unit_f32();
    let mut placements = Vec::with_capacity(count as usize);
    let mut skipped = 0;
    for ordinal in 0..count {
        let node = ordinal / per_node;
        let within = ordinal % per_node;
        let id = parent.id.child(lineage, u64::from(ordinal));
        let key = id.key(seed);
        // Children of one node share its position; jitter is keyed by node so
        // opposite and whorled children stay together.
        let node_key = parent_key.with(tag("node")).with(u64::from(node));
        let slot = (node as f32 + 0.5 + 0.5 * level.position_jitter * node_key.signed_unit_f32())
            / node_count as f32;
        let t = (lo + (hi - lo) * slot).clamp(lo, hi).clamp(0.0, 1.0);
        let azimuth = phase
            + node as f32 * node_step
            + within as f32 * TAU / per_node as f32
            + level.roll_jitter * signed(key, "roll");
        let angle = level.angle.eval(t) + level.angle_jitter * signed(key, "angle");
        let length = parent_length
            * level.length.eval(t)
            * (1.0 + level.length_jitter * signed(key, "length"));
        if length <= 1e-3 {
            skipped += 1;
            continue;
        }
        let sample = parent.sample(t);
        let frame = sample.frame;
        let side = frame.normal * libm::cosf(azimuth) + frame.binormal() * libm::sinf(azimuth);
        let direction = (frame.tangent * libm::cosf(angle) + side * libm::sinf(angle))
            .normalize_or(frame.tangent);
        placements.push(Placement {
            id,
            t,
            start: sample.position,
            direction,
            length,
        });
    }
    (placements, skipped)
}

/// Smooth keyed 1D value noise in `[-1, 1)` at lattice spacing 1.
fn smooth_noise(key: Key, x: f32) -> f32 {
    let cell = libm::floorf(x);
    let f = x - cell;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "arc lengths over wavelengths are small and non-negative"
    )]
    let i = cell as i64 as u64;
    let a = key.with(i).signed_unit_f32();
    let b = key.with(i.wrapping_add(1)).signed_unit_f32();
    let s = f * f * (3.0 - 2.0 * f);
    a + (b - a) * s
}

/// Turns `heading` toward `target` by at most `angle` radians.
fn turn_toward(heading: Vec3, target: Vec3, angle: f32) -> Vec3 {
    let axis = heading.cross(target);
    let Some(axis) = axis.try_normalize() else {
        return heading;
    };
    let between = libm::acosf(heading.dot(target).clamp(-1.0, 1.0));
    Quat::from_axis_angle(axis, angle.min(between)) * heading
}

/// Samples a centerline of `length` from `start` along `direction`, bending by
/// `shape`.
fn centerline(
    start: Vec3,
    direction: Vec3,
    length: f32,
    shape: &Shape,
    key: Key,
    segment_length: f32,
) -> Vec<Node> {
    let spacing = if shape.kink != 0.0 {
        segment_length.min(shape.kink_interval)
    } else {
        segment_length
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "lengths over the validated spacing are small and positive"
    )]
    let segments = (libm::ceilf(length / spacing) as usize).max(2);
    let step = length / segments as f32;
    let mut position = start;
    let mut heading = direction.normalize_or(Vec3::Z);
    // A frame carried along the branch gives the gnarl its two bend axes
    // and the vertical curve its axis when the heading is vertical.
    let (mut frame, _) =
        Frame::from_tangent_or_axis(heading, Vec3::Z).expect("heading is a finite unit vector");
    let wander = [key.with(tag("gnarl.u")), key.with(tag("gnarl.v"))];
    let wavelength = shape.gnarl_wavelength.max(1e-3);
    let kink_key = key.with(tag("kink"));
    let kink_azimuth = TAU * kink_key.unit_f32();
    let fallback_axis = {
        let azimuth = TAU * key.with(tag("curve.azimuth")).unit_f32();
        Vec3::new(libm::cosf(azimuth), libm::sinf(azimuth), 0.0)
    };
    let mut nodes = Vec::with_capacity(segments + 1);
    nodes.push(Node::at(position));
    for segment in 0..segments {
        let s = (segment as f32 + 0.5) / segments as f32;
        let distance = s * length;

        // Planar curve: rotate about the horizontal axis perpendicular to
        // the heading, so positive values bend up.
        let curve_total = if s < 0.5 {
            shape.curve
        } else {
            shape.curve_back
        };
        let curve_angle = curve_total * 2.0 * step / length;
        if curve_angle != 0.0 {
            let axis = heading
                .cross(Vec3::Z)
                .try_normalize()
                .unwrap_or(fallback_axis);
            // A positive rotation about heading x Z turns toward +Z.
            heading = Quat::from_axis_angle(axis, curve_angle) * heading;
        }

        if shape.gnarl != 0.0 {
            let u = smooth_noise(wander[0], distance / wavelength);
            let v = smooth_noise(wander[1], distance / wavelength);
            let turn = shape.gnarl * step;
            heading = Quat::from_axis_angle(frame.normal, turn * u) * heading;
            heading = Quat::from_axis_angle(frame.binormal(), turn * v) * heading;
        }
        if shape.up != 0.0 {
            heading = turn_toward(heading, Vec3::Z, shape.up * step);
        }
        if shape.sag != 0.0 {
            heading = turn_toward(heading, -Vec3::Z, shape.sag * s * step);
        }
        if shape.light != 0.0 {
            let radial = Vec3::new(position.x, position.y, 0.0)
                .try_normalize()
                .or_else(|| Vec3::new(heading.x, heading.y, 0.0).try_normalize());
            if let Some(radial) = radial {
                heading = turn_toward(heading, radial, shape.light * step);
            }
        }
        heading = heading.normalize_or(Vec3::Z);
        // Sympodial growth zig-zags around the trend: each internode leans
        // half a kink to alternate sides of the smooth heading, so
        // consecutive internodes meet at the kink angle while the trend,
        // steered by the terms above, never accumulates the kinks.
        let internode = if shape.kink == 0.0 {
            heading
        } else {
            let k = kink_key.with(segment as u64);
            let angle = 0.5 * shape.kink * (1.0 + shape.kink_jitter * k.with(0).signed_unit_f32());
            let side = if segment % 2 == 0 { 0.0 } else { PI };
            let azimuth =
                kink_azimuth + side + shape.kink_jitter * PI * k.with(1).signed_unit_f32();
            let axis = frame.normal * libm::cosf(azimuth) + frame.binormal() * libm::sinf(azimuth);
            (Quat::from_axis_angle(axis, angle) * heading).normalize_or(heading)
        };
        let next = position + internode * step;
        frame = frame.transport(position, next, heading);
        position = next;
        nodes.push(Node::at(position));
    }
    nodes
}

enum Pruned {
    Kept,
    Truncated,
    Removed,
}

/// Smooth keyed 3D value noise in `[-1, 1)` at lattice spacing 1.
fn smooth_noise_3d(key: Key, p: Vec3) -> f32 {
    let cell = p.floor();
    let f = p - cell;
    let s = f * f * (Vec3::splat(3.0) - 2.0 * f);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "crown coordinates over the lump size are small"
    )]
    let [x, y, z] = cell.to_array().map(|c| c as i64);
    let corner = |dx: i64, dy: i64, dz: i64| {
        #[expect(clippy::cast_sign_loss, reason = "a lattice coordinate as hash input")]
        let hash = |v: i64| v as u64;
        key.with(hash(x + dx))
            .with(hash(y + dy))
            .with(hash(z + dz))
            .signed_unit_f32()
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let face = |dz| {
        lerp(
            lerp(corner(0, 0, dz), corner(1, 0, dz), s.x),
            lerp(corner(0, 1, dz), corner(1, 1, dz), s.x),
            s.y,
        )
    };
    lerp(face(0), face(1), s.z)
}

fn inside(envelope: Option<(&Envelope, Key)>, p: Vec3) -> bool {
    if p.z < 0.0 {
        return false;
    }
    let Some((e, key)) = envelope else {
        return true;
    };
    let p = if e.lumps > 0.0 {
        // Pull the point toward the middle where the surface bulges out, and
        // push it away where it dips in.
        let middle = Vec3::new(0.0, 0.0, e.base + 0.5 * e.height);
        let bulge = 1.0 + e.lumps * smooth_noise_3d(key, p / e.lump_size);
        middle + (p - middle) / bulge
    } else {
        p
    };
    let u = (p.z - e.base) / e.height;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    Vec3::new(p.x, p.y, 0.0).length() <= e.radius * e.profile.eval(u)
}

/// Truncates `nodes` at the first node outside the envelope (or below
/// ground). The first node is the attachment point and is never tested.
fn prune(nodes: &mut Vec<Node>, intended: f32, envelope: Option<(&Envelope, Key)>) -> Pruned {
    let Some(first_out) = nodes
        .iter()
        .skip(1)
        .position(|node| !inside(envelope, node.position))
        .map(|i| i + 1)
    else {
        return Pruned::Kept;
    };
    let min_fraction = envelope.map_or(0.0, |(e, _)| e.min_fraction);
    if first_out < 2 {
        return Pruned::Removed;
    }
    nodes.truncate(first_out);
    let kept: f32 = nodes
        .windows(2)
        .map(|pair| pair[0].position.distance(pair[1].position))
        .sum();
    if kept < min_fraction * intended {
        Pruned::Removed
    } else {
        Pruned::Truncated
    }
}

fn place_sites(
    skeleton: &mut Skeleton,
    hierarchy: &Hierarchy,
    seed: u64,
) -> Result<usize, GrowError> {
    let mut sites = Vec::new();
    for branch in skeleton.branches() {
        let Some(level) = (branch.order as usize)
            .checked_sub(1)
            .and_then(|index| hierarchy.levels.get(index))
        else {
            continue;
        };
        let Some(Sites {
            per_metre,
            span: [lo, hi],
            kind,
            angle,
            tip_cluster,
            cluster_span,
        }) = level.sites
        else {
            continue;
        };
        let key = branch.id.key(seed).with(tag("sites"));
        let count = keyed_round(
            per_metre * branch.length() * (hi - lo),
            key.with(tag("count")),
        );
        // An outward frame at `t`, rolled by `roll` about the branch.
        let outward = |t: f32, roll: f32| {
            let axis = branch.sample(t).frame;
            let side = Quat::from_axis_angle(axis.tangent, roll) * axis.normal;
            let out = side * libm::sinf(angle) + axis.tangent * libm::cosf(angle);
            Frame::from_tangent(out, axis.tangent).unwrap_or(axis)
        };
        for ordinal in 0..count {
            let site_key = key.with(u64::from(ordinal));
            let t = (lo
                + (hi - lo) * (ordinal as f32 + 0.5 + 0.4 * site_key.signed_unit_f32())
                    / count as f32)
                .clamp(0.0, 1.0);
            sites.push(Site {
                branch: branch.id,
                ordinal,
                kind,
                t,
                frame: outward(t, crate::GOLDEN_ANGLE * ordinal as f32),
                scale: 1.0,
            });
        }
        // The tip whorl: evenly spread around the branch, crowded toward the
        // tip.
        let whorl = key.with(tag("whorl"));
        for i in 0..tip_cluster {
            let k = whorl.with(u64::from(i));
            let t = (1.0 - cluster_span * (i as f32 + 0.5 * k.unit_f32()) / tip_cluster as f32)
                .clamp(0.0, 1.0);
            let roll = TAU * (i as f32 + 0.3 * k.with(1).signed_unit_f32()) / tip_cluster as f32;
            sites.push(Site {
                branch: branch.id,
                ordinal: count + i,
                kind,
                t,
                frame: outward(t, roll),
                scale: 1.0,
            });
        }
    }
    let placed = sites.len();
    for site in sites {
        skeleton.push_site(site)?;
    }
    Ok(placed)
}
