// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leaf placement on skeleton sites.

use alloc::vec::Vec;

use exedra_mesh::Mesh;
use glam::{Mat3, Quat, Vec3};
use sylva_skeleton::Skeleton;
use sylva_skeleton::keyed::{Key, SignedUnit, tag};

use crate::{FoliageError, LeafShape, leaf_mesh};

/// Per-variant variation of the leaf shape, as fractions (`0.2` is ±20%).
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Variation {
    /// Blade length variation.
    pub size: f32,
    /// Width variation.
    pub width: f32,
    /// Lobe depth variation.
    pub lobe_depth: f32,
}

impl Default for Variation {
    fn default() -> Self {
        Self {
            size: 0.2,
            width: 0.15,
            lobe_depth: 0.25,
        }
    }
}

/// How leaves are shaped and placed on a skeleton's sites.
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct FoliageParams {
    /// Site kind that carries leaves; other sites are ignored.
    pub site_kind: u32,
    /// The base leaf shape.
    pub shape: LeafShape,
    /// Number of template variants derived from `shape`; at least 1.
    /// Instances pick one by keyed hash.
    pub variants: u32,
    /// How variants differ from `shape`.
    pub variation: Variation,
    /// Downward tilt of each leaf from its site's outward direction, in
    /// radians (the petiole bends under the blade's weight).
    pub droop: f32,
    /// How far each blade turns its upper surface toward `+Z`, in `[0, 1]`:
    /// 0 keeps the site frame's normal, 1 faces the sky as much as the leaf
    /// direction allows.
    pub light: f32,
    /// Keyed roll of each blade about its midrib, in radians (±).
    pub roll_jitter: f32,
    /// Blades per site, at least 1. A site's blades share its position and
    /// direction and are rolled evenly about their common midrib, half a
    /// turn apart in all: two cross, three make a six-pointed star. A
    /// needle spray repeated this way clothes its shoot all round, as a
    /// conifer's needles do.
    pub whorl: u32,
    /// Seed for template variation and per-leaf choices.
    pub seed: u64,
}

impl Default for FoliageParams {
    fn default() -> Self {
        Self {
            site_kind: 0,
            shape: LeafShape::default(),
            variants: 4,
            variation: Variation::default(),
            droop: 0.35,
            light: 0.6,
            roll_jitter: 0.5,
            whorl: 1,
            seed: 0,
        }
    }
}

/// One leaf template: a shape variant and its mesh.
#[derive(Clone, Debug)]
pub struct LeafTemplate {
    /// The variant's shape.
    pub shape: LeafShape,
    /// Its blade mesh ([`leaf_mesh`]).
    pub mesh: Mesh,
    /// Triangles after extraction.
    pub triangles: u64,
}

/// One placed leaf: a template under a rigid transform and uniform scale.
///
/// A leaf-space point `p` lands at `position + rotation * (scale * p)`.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LeafInstance {
    /// Index into [`Foliage::templates`].
    pub template: u32,
    /// Index of the carrying site in [`Skeleton::sites`].
    pub site: u32,
    /// Blade base position, at the end of the petiole.
    pub position: Vec3,
    /// Rotation from leaf space (midrib `+Y`, upper surface `+Z`).
    pub rotation: Quat,
    /// Uniform scale, the site's organ scale.
    pub scale: f32,
    /// Unit direction from the crown's leaf centroid to this leaf. Shading a
    /// crown with normals bent toward it gives the soft, volumetric look
    /// real-time foliage relies on.
    pub canopy_normal: Vec3,
}

/// Deterministic counts describing one placement run.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct FoliageReport {
    /// Leaves placed.
    pub leaves: u64,
    /// Sites of other kinds, left without leaves.
    pub other_sites: u64,
    /// Templates built.
    pub templates: u64,
    /// Triangles over all placed leaves at full detail.
    pub instanced_triangles: u64,
    /// Centroid of all leaf positions, the origin of canopy normals.
    pub crown_centroid: Vec3,
}

/// Leaves of one tree: templates plus instances.
///
/// Leaves stay instances until level-of-detail packaging merges them into
/// render buffers, so thousands of leaves share a handful of meshes.
#[derive(Clone, Debug)]
pub struct Foliage {
    /// Shape variants and their meshes.
    pub templates: Vec<LeafTemplate>,
    /// Placed leaves, in site order.
    pub instances: Vec<LeafInstance>,
    /// Deterministic counts.
    pub report: FoliageReport,
}

fn signed(key: Key, purpose: &str) -> f32 {
    key.with(tag(purpose)).signed_unit_f32()
}

/// Turns `v` toward `target` by at most `angle` radians.
fn turn_toward(v: Vec3, target: Vec3, angle: f32) -> Vec3 {
    let Some(axis) = v.cross(target).try_normalize() else {
        return v;
    };
    let between = libm::acosf(v.dot(target).clamp(-1.0, 1.0));
    Quat::from_axis_angle(axis, angle.min(between)) * v
}

/// Builds leaf templates and places a leaf at every site of
/// [`FoliageParams::site_kind`].
///
/// Every choice is keyed by the seed and the site's branch ID, kind and
/// ordinal, so editing foliage parameters never reshuffles growth, and a
/// site keeps its leaf across edits that keep the site.
///
/// Each leaf's midrib leaves its site along the site's outward direction,
/// tilted down by `droop`; its upper surface turns toward the sky by
/// `light`, then rolls by a keyed angle. The blade base sits one twig radius
/// plus the petiole away from the twig's centerline.
///
/// # Errors
///
/// [`FoliageError::Params`] for invalid parameters,
/// [`FoliageError::MissingBranch`] for a site whose branch is absent, or a
/// kernel error while meshing templates.
pub fn place_leaves(skeleton: &Skeleton, params: &FoliageParams) -> Result<Foliage, FoliageError> {
    params.validate()?;
    let mut report = FoliageReport::default();
    let templates = (0..params.variants)
        .map(|k| {
            let key = Key::new(params.seed)
                .with(tag("leaf.variant"))
                .with(u64::from(k));
            let v = params.variation;
            let mut shape = params.shape;
            if k > 0 {
                shape.length *= 1.0 + v.size * signed(key, "size");
                shape.width *= 1.0 + v.width * signed(key, "width");
                shape.lobe_depth =
                    (shape.lobe_depth * (1.0 + v.lobe_depth * signed(key, "lobes"))).min(0.95);
            }
            let mesh = leaf_mesh(&shape)?;
            let triangles = mesh
                .faces()
                .map(|f| mesh.face_loop(f).count().saturating_sub(2) as u64)
                .sum();
            Ok(LeafTemplate {
                shape,
                mesh,
                triangles,
            })
        })
        .collect::<Result<Vec<_>, FoliageError>>()?;
    report.templates = templates.len() as u64;

    let mut instances = Vec::new();
    for (index, site) in skeleton.sites().iter().enumerate() {
        if site.kind != params.site_kind {
            report.other_sites += 1;
            continue;
        }
        let site_index = u32::try_from(index).map_err(|_| FoliageError::TooLarge)?;
        let branch = skeleton
            .branch(site.branch)
            .ok_or(FoliageError::MissingBranch)?;
        let key = site
            .branch
            .key(params.seed)
            .with(tag("leaf"))
            .with(u64::from(site.kind))
            .with(u64::from(site.ordinal));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "a unit value scaled into the small template range"
        )]
        let template = ((key.with(tag("template")).unit_f32() * params.variants as f32) as u32)
            .min(params.variants - 1);
        let sample = branch.sample(site.t);
        let out = site.frame.tangent;
        let direction = turn_toward(out, -Vec3::Z, params.droop).normalize_or(out);
        let side = site.frame.normal - direction * site.frame.normal.dot(direction);
        let sky = Vec3::Z - direction * direction.dot(Vec3::Z);
        let mut normal = side
            .normalize_or(Vec3::X)
            .lerp(sky.normalize_or(side), params.light)
            .normalize_or(side.normalize_or(Vec3::X));
        normal =
            Quat::from_axis_angle(direction, params.roll_jitter * signed(key, "roll")) * normal;
        let shape = &templates[template as usize].shape;
        let position = sample.position + direction * (sample.radius + shape.petiole * site.scale);
        for k in 0..params.whorl {
            #[expect(clippy::cast_precision_loss, reason = "whorl sizes are small")]
            let turn = core::f32::consts::PI * k as f32 / params.whorl as f32;
            let normal = Quat::from_axis_angle(direction, turn) * normal;
            let across = direction.cross(normal).normalize_or(Vec3::X);
            let normal = across.cross(direction);
            let rotation = Quat::from_mat3(&Mat3::from_cols(across, direction, normal));
            report.instanced_triangles += templates[template as usize].triangles;
            instances.push(LeafInstance {
                template,
                site: site_index,
                position,
                rotation,
                scale: site.scale,
                canopy_normal: normal,
            });
        }
    }
    report.leaves = instances.len() as u64;
    if !instances.is_empty() {
        #[expect(clippy::cast_precision_loss, reason = "a mean over leaf positions")]
        let centroid = instances.iter().map(|i| i.position).sum::<Vec3>() / instances.len() as f32;
        report.crown_centroid = centroid;
        for leaf in &mut instances {
            leaf.canopy_normal = (leaf.position - centroid).normalize_or(leaf.canopy_normal);
        }
    }
    Ok(Foliage {
        templates,
        instances,
        report,
    })
}
