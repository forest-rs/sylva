// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The chain's last level: an impostor of the whole tree.
//!
//! [`ImpostorLayout::Crossed`] stands the tree up as vertical quads crossed
//! about its trunk axis. Plane `i` faces azimuth `i * pi / planes` and samples
//! its own atlas cell, baked from that side ([`crate::bake_impostor`]). It
//! renders correctly in any glTF viewer, but seen from its back a plane shows
//! its cell mirrored, and crossed planes cover more screen than the tree does
//! from oblique views.
//!
//! [`ImpostorLayout::Octahedral`] bakes a `frames x frames` grid of views over
//! the upper hemisphere, laid out by the hemi-octahedral map
//! ([`hemi_octahedral_encode`]), and draws one camera-facing quad that samples
//! the frame nearest the view direction ([`Impostor::frame_for`]) or blends
//! the nearest few. The billboarding and blending need a renderer shader;
//! [`Impostor::geometry`] is then one quad placed for a viewer on `-Y`.

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
    /// How the views are laid out and drawn.
    pub layout: ImpostorLayout,
}

impl Default for ImpostorPolicy {
    fn default() -> Self {
        Self {
            screen_size: 0.015,
            crossfade: 0.25,
            layout: ImpostorLayout::Crossed { planes: 3 },
        }
    }
}

/// An impostor's views and how they are drawn.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ImpostorLayout {
    /// Vertical quads crossed about the tree's axis: 1 to 8. Needs no
    /// special shader.
    Crossed {
        /// Vertical planes.
        planes: u32,
    },
    /// A hemi-octahedral grid of `frames x frames` views over the upper
    /// hemisphere, 2 to 16 per side, drawn as one camera-facing quad by a
    /// renderer that picks or blends frames by view direction.
    Octahedral {
        /// Frames along each side of the grid.
        frames: u32,
    },
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
    /// Fits the policy's views around the tree's bark and leaves.
    ///
    /// Crossed planes are centred on the vertical axis through the trunk's
    /// base (so every plane draws the trunk in the same place) and span the
    /// tree's full height and its largest horizontal radius about that axis,
    /// so no rotation clips the crown. Octahedral frames are centred on the
    /// tree's bounding box and span its bounding sphere, so every frame shows
    /// the whole tree at one scale.
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
        let views = match policy.layout {
            ImpostorLayout::Crossed { planes } => {
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
                (0..planes)
                    .map(|plane| {
                        #[expect(clippy::cast_precision_loss, reason = "at most eight planes")]
                        let azimuth = core::f32::consts::PI * plane as f32 / planes as f32;
                        let right = Vec3::new(libm::cosf(azimuth), libm::sinf(azimuth), 0.0);
                        CardView::facing(
                            center,
                            right,
                            Vec3::Z,
                            Vec2::new(radius, half_height),
                            radius,
                        )
                    })
                    .collect()
            }
            ImpostorLayout::Octahedral { frames } => {
                let center = (lo + hi) * 0.5;
                let radius = points
                    .iter()
                    .map(|&(p, pad)| p.distance(center) + pad)
                    .fold(1e-3_f32, f32::max);
                (0..frames * frames)
                    .map(|cell| {
                        let toward = frame_direction(frames, cell);
                        let (right, up) = view_axes(toward);
                        CardView::facing(center, right, up, Vec2::splat(radius), radius)
                    })
                    .collect()
            }
        };
        Self { policy, views }
    }

    /// For an octahedral impostor, the atlas cell of the frame whose view
    /// direction is nearest `toward` (a direction from the tree to the
    /// viewer; below the horizon it clamps to the horizon). `None` for
    /// crossed planes.
    #[must_use]
    pub fn frame_for(&self, toward: Vec3) -> Option<u32> {
        let ImpostorLayout::Octahedral { frames } = self.policy.layout else {
            return None;
        };
        let upper = Vec3::new(toward.x, toward.y, toward.z.max(0.0));
        let uv = hemi_octahedral_encode(upper.try_normalize().unwrap_or(Vec3::Z));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "uv lies in [0, 1] and frames is small"
        )]
        let index = |x: f32| ((x * frames as f32) as u32).min(frames - 1);
        Some(index(uv.y) * frames + index(uv.x))
    }

    /// The atlas grid the views fill; an octahedral grid is `frames` square.
    #[must_use]
    pub fn layout(&self) -> AtlasLayout {
        AtlasLayout::for_cells(self.views.len())
    }

    /// Crossed planes: one quad per plane over its view, sampling its atlas
    /// cell, with the view direction as the vertex normal. Octahedral: one
    /// quad over the frame nearest a viewer on `-Y`, facing that viewer;
    /// renderers billboard it and choose frames per view.
    #[must_use]
    pub fn geometry(&self) -> TriMesh {
        let layout = self.layout();
        let mut out = TriMesh::default();
        let cells: Vec<u32> = match self.frame_for(Vec3::NEG_Y) {
            Some(frame) => alloc::vec![frame],
            None => (0..self.views.len())
                .map(|cell| u32::try_from(cell).expect("few planes"))
                .collect(),
        };
        for cell in cells {
            let view = &self.views[cell as usize];
            let (right, up) = if self.frame_for(Vec3::NEG_Y).is_some() {
                (Vec3::X, Vec3::Z)
            } else {
                (view.right, view.up)
            };
            push_quad(
                &mut out,
                view.center,
                right,
                up,
                view.half,
                layout.cell(cell),
                right.cross(up),
            );
        }
        out
    }
}

/// Maps a direction in the upper hemisphere (`z >= 0`) to `[0, 1]^2` by the
/// hemi-octahedral map: project onto the octahedron `|x| + |y| + z = 1`,
/// then rotate the resulting diamond by 45 degrees to fill the square.
/// Directions below the horizon are the caller's to clamp.
#[must_use]
pub fn hemi_octahedral_encode(direction: Vec3) -> Vec2 {
    let sum = direction.x.abs() + direction.y.abs() + direction.z.max(0.0);
    let p = if sum > 0.0 {
        Vec2::new(direction.x, direction.y) / sum
    } else {
        Vec2::ZERO
    };
    let square = Vec2::new(p.x + p.y, p.x - p.y);
    (square + Vec2::ONE) * 0.5
}

/// The inverse of [`hemi_octahedral_encode`]: the unit direction for a
/// point of `[0, 1]^2`.
#[must_use]
pub fn hemi_octahedral_decode(uv: Vec2) -> Vec3 {
    let square = uv * 2.0 - Vec2::ONE;
    let p = Vec2::new(square.x + square.y, square.x - square.y) * 0.5;
    let z = 1.0 - p.x.abs() - p.y.abs();
    Vec3::new(p.x, p.y, z).try_normalize().unwrap_or(Vec3::Z)
}

/// The view direction of frame `cell` of a `frames x frames` grid, at the
/// cell's centre, row-major from `v = 0`.
fn frame_direction(frames: u32, cell: u32) -> Vec3 {
    #[expect(clippy::cast_precision_loss, reason = "grids are small")]
    let uv = Vec2::new(
        ((cell % frames) as f32 + 0.5) / frames as f32,
        ((cell / frames) as f32 + 0.5) / frames as f32,
    );
    hemi_octahedral_decode(uv)
}

/// Card axes for a view from `toward`: `up` is world `+Z` made
/// perpendicular to the view (world `+Y` when looking straight down), and
/// `right x up = toward`.
fn view_axes(toward: Vec3) -> (Vec3, Vec3) {
    let hint = if toward.z.abs() > 0.999 {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let up = (hint - toward * toward.dot(hint)).normalize();
    (up.cross(toward), up)
}
