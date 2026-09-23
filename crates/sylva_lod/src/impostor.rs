// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The chain's last level: a billboard impostor.
//!
//! [`Impostor`] stands the whole tree up as `planes` vertical quads crossed
//! about its vertical axis. Plane `i` faces azimuth `i * pi / planes` and
//! samples its own atlas cell, baked from that side
//! ([`crate::bake_impostor`]). Seen from its back a plane shows its cell
//! mirrored, the usual price of crossed billboards; octahedral impostors,
//! which bake many directions and blend between them, are a later refinement.

use alloc::vec::Vec;

use exedra_mesh::TriMesh;
use glam::{Vec2, Vec3};
use sylva_bake::CardView;
use sylva_foliage::Foliage;
use sylva_skeleton::Skeleton;

use crate::clusters::{AtlasLayout, push_quad};

/// When and how the chain ends in an impostor.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ImpostorPolicy {
    /// Use the impostor while the tree's projected height is at least this
    /// fraction of the screen height (and below the last level's).
    pub screen_size: f32,
    /// Width of the dithered crossfade from the last level, as a fraction
    /// of `screen_size`.
    pub crossfade: f32,
    /// Vertical planes, crossed about the tree's axis: 1 to 8.
    pub planes: u32,
}

impl Default for ImpostorPolicy {
    fn default() -> Self {
        Self {
            screen_size: 0.015,
            crossfade: 0.25,
            planes: 3,
        }
    }
}

/// A built impostor: its views, one per plane and atlas cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Impostor {
    /// The policy it was built from.
    pub policy: ImpostorPolicy,
    /// One vertical view per plane, in atlas cell order.
    pub views: Vec<CardView>,
}

impl Impostor {
    /// Fits `policy.planes` views around the tree's bark and leaves: every
    /// view is centred on the vertical axis through the trunk's base (so all
    /// planes draw the trunk in the same place) and spans the tree's full
    /// height and its largest horizontal radius about that axis, so no
    /// rotation clips the crown.
    #[must_use]
    pub fn fit(skeleton: &Skeleton, foliage: &Foliage, policy: ImpostorPolicy) -> Self {
        let mut points: Vec<(Vec3, f32)> = Vec::new();
        for branch in skeleton.branches() {
            points.extend(branch.nodes.iter().map(|n| (n.position, n.radius)));
        }
        for leaf in &foliage.instances {
            let shape = &foliage.templates[leaf.template as usize].shape;
            points.push((leaf.position, shape.length * leaf.scale));
        }
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for &(p, pad) in &points {
            lo = lo.min(p - Vec3::splat(pad));
            hi = hi.max(p + Vec3::splat(pad));
        }
        if points.is_empty() {
            lo = Vec3::ZERO;
            hi = Vec3::ONE;
        }
        let axis = skeleton
            .branches()
            .iter()
            .find(|b| b.parent.is_none())
            .map_or((lo + hi) * 0.5, |trunk| trunk.nodes[0].position);
        let center = Vec3::new(axis.x, axis.y, (lo.z + hi.z) * 0.5);
        let radius = points
            .iter()
            .map(|&(p, pad)| {
                let d = p - center;
                libm::sqrtf(d.x * d.x + d.y * d.y) + pad
            })
            .fold(1e-3_f32, f32::max);
        let half_height = ((hi.z - lo.z) * 0.5).max(1e-3);
        let views = (0..policy.planes)
            .map(|plane| {
                #[expect(clippy::cast_precision_loss, reason = "at most eight planes")]
                let azimuth = core::f32::consts::PI * plane as f32 / policy.planes as f32;
                let right = Vec3::new(libm::cosf(azimuth), libm::sinf(azimuth), 0.0);
                CardView::facing(
                    center,
                    right,
                    Vec3::Z,
                    Vec2::new(radius, half_height),
                    radius,
                )
            })
            .collect();
        Self { policy, views }
    }

    /// The atlas grid the planes fill.
    #[must_use]
    pub fn layout(&self) -> AtlasLayout {
        AtlasLayout::for_cells(self.views.len())
    }

    /// One quad per plane over its view, sampling its atlas cell, with the
    /// view direction as the vertex normal.
    #[must_use]
    pub fn geometry(&self) -> TriMesh {
        let layout = self.layout();
        let mut out = TriMesh::default();
        for (cell, view) in self.views.iter().enumerate() {
            push_quad(
                &mut out,
                view.center,
                view.right,
                view.up,
                view.half,
                layout.cell(u32::try_from(cell).expect("few planes")),
                view.toward(),
            );
        }
        out
    }
}
