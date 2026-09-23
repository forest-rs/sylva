// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Cluster cards: a branch subtree's twigs and leaves drawn as crossed,
//! textured quads.
//!
//! A level with [`ClusterCards`] groups every leaf under its ancestor branch
//! of [`ClusterCards::root_order`] into one cluster. Each cluster becomes a
//! [`ClusterCard`] fitted around its twigs and leaves, facing between the
//! crown's outward direction and the leaves' mean upper surface. A few exemplar clusters, chosen by keyed quantiles of leaf count,
//! are baked into an atlas ([`crate::bake_clusters`]); every card samples the
//! exemplar whose aspect is closest to its own. At a distance that reads as
//! the crown's foliage masses for a few triangles per cluster, instead of
//! thousands of sub-pixel leaves.

use alloc::vec;
use alloc::vec::Vec;

use exedra_mesh::TriMesh;
use glam::{Vec2, Vec3};
use sylva_bake::CardView;
use sylva_foliage::Foliage;
use sylva_skeleton::Skeleton;

/// How a level draws its leaves as cluster cards.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClusterCards {
    /// Each branch of this order, with its descendants, forms one cluster.
    /// Leaves on branches of lower order stay individual leaves.
    pub root_order: u32,
    /// Distinct baked exemplars (atlas cells) the cards share; at least 1.
    pub variants: u32,
    /// Card planes per cluster, crossed about its up axis: 1 to 3.
    pub planes: u32,
}

/// One cluster's card.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClusterCard {
    /// Index in [`Skeleton::branches`] of the cluster's root branch.
    pub root: u32,
    /// Card centre.
    pub center: Vec3,
    /// Unit direction of the card's `+X`.
    pub right: Vec3,
    /// Unit direction of the card's `+Y`: from the root's base toward its
    /// leaves.
    pub up: Vec3,
    /// Half extents along `right` and `up`, in metres.
    pub half: Vec2,
    /// Half depth of the cluster along `right x up`, in metres.
    pub half_depth: f32,
    /// Leaves the card stands in for.
    pub leaves: u32,
    /// Atlas cell it samples: an index into [`Clusters::variants`].
    pub variant: u32,
    /// Unit direction from the crown's leaf centroid, for soft shading.
    pub canopy_normal: Vec3,
}

impl ClusterCard {
    /// The card's frame as a bake view.
    #[must_use]
    pub fn view(&self) -> CardView {
        CardView::facing(self.center, self.right, self.up, self.half, self.half_depth)
    }
}

/// One exemplar the atlas holds.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClusterVariant {
    /// Index into [`Clusters::cards`] of the exemplar cluster.
    pub exemplar: u32,
    /// The exemplar's view, which the bake renders into this cell.
    pub view: CardView,
}

/// A level's cluster cards.
#[derive(Clone, Debug, PartialEq)]
pub struct Clusters {
    /// The level's settings.
    pub params: ClusterCards,
    /// One card per non-empty cluster, in branch storage order.
    pub cards: Vec<ClusterCard>,
    /// Leaf instance indices of each card, parallel to `cards`.
    pub members: Vec<Vec<u32>>,
    /// Baked exemplars, in atlas cell order.
    pub variants: Vec<ClusterVariant>,
}

impl Clusters {
    /// The atlas grid the variants fill.
    #[must_use]
    pub fn layout(&self) -> AtlasLayout {
        AtlasLayout::for_cells(self.variants.len())
    }

    /// Crossed card quads: `planes` quads per card, each spanning the card's
    /// `up` and its `right` turned about `up` by `plane * pi / planes`, with
    /// UVs in the card's atlas cell and every vertex normal set to the card's
    /// canopy normal.
    #[must_use]
    pub fn geometry(&self) -> TriMesh {
        let layout = self.layout();
        let mut out = TriMesh::default();
        for card in &self.cards {
            let toward = card.right.cross(card.up);
            for plane in 0..self.params.planes {
                #[expect(clippy::cast_precision_loss, reason = "at most three planes")]
                let angle = core::f32::consts::PI * plane as f32 / self.params.planes as f32;
                let right = card.right * libm::cosf(angle) + toward * libm::sinf(angle);
                push_quad(
                    &mut out,
                    card.center,
                    right,
                    card.up,
                    card.half,
                    layout.cell(card.variant),
                    card.canopy_normal,
                );
            }
        }
        out
    }
}

/// A grid of equal atlas cells, filled row by row from `v = 0`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AtlasLayout {
    /// Cells across.
    pub columns: u32,
    /// Cells up.
    pub rows: u32,
}

impl AtlasLayout {
    /// The squarest grid holding `cells` cells.
    #[must_use]
    pub fn for_cells(cells: usize) -> Self {
        let cells = u32::try_from(cells.max(1)).unwrap_or(u32::MAX);
        let mut columns = 1;
        while columns * columns < cells {
            columns += 1;
        }
        Self {
            columns,
            rows: cells.div_ceil(columns),
        }
    }

    /// UV rectangle `[u0, v0, u1, v1]` of `cell`.
    #[must_use]
    pub fn cell(&self, cell: u32) -> [f32; 4] {
        #[expect(clippy::cast_precision_loss, reason = "atlas grids are small")]
        let (c, r, w, h) = (
            (cell % self.columns) as f32,
            (cell / self.columns) as f32,
            self.columns as f32,
            self.rows as f32,
        );
        [c / w, r / h, (c + 1.0) / w, (r + 1.0) / h]
    }
}

/// Appends one two-triangle quad centred at `center`.
pub(crate) fn push_quad(
    out: &mut TriMesh,
    center: Vec3,
    right: Vec3,
    up: Vec3,
    half: Vec2,
    [u0, v0, u1, v1]: [f32; 4],
    normal: Vec3,
) {
    let base = u32::try_from(out.positions.len()).expect("card meshes stay small");
    for (x, y, u, v) in [
        (-1.0, -1.0, u0, v0),
        (1.0, -1.0, u1, v0),
        (1.0, 1.0, u1, v1),
        (-1.0, 1.0, u0, v1),
    ] {
        let p = center + right * (x * half.x) + up * (y * half.y);
        out.positions.push(p.to_array());
        out.uvs.push([u, v]);
        out.normals.push(normal.to_array());
    }
    out.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Each branch's cluster root: itself at `root_order`, its parent's root
/// above it, and none below. Branches are stored parents first.
pub(crate) fn cluster_roots(skeleton: &Skeleton, root_order: u32) -> Vec<Option<usize>> {
    let branches = skeleton.branches();
    let mut roots = vec![None; branches.len()];
    for (index, branch) in branches.iter().enumerate() {
        roots[index] = match branch.order {
            o if o == root_order => Some(index),
            o if o > root_order => branch
                .parent
                .and_then(|a| skeleton.index_of(a.parent))
                .and_then(|p| roots[p]),
            _ => None,
        };
    }
    roots
}

/// Groups `foliage` into cluster cards; returns the cards (with their
/// members) and, per leaf instance, whether it went into a card.
pub(crate) fn build_clusters(
    skeleton: &Skeleton,
    foliage: &Foliage,
    params: ClusterCards,
) -> (Clusters, Vec<bool>) {
    let roots = cluster_roots(skeleton, params.root_order);
    let mut slot = vec![u32::MAX; roots.len()];
    let mut order = Vec::new();
    let mut members: Vec<Vec<u32>> = Vec::new();
    let mut clustered = vec![false; foliage.instances.len()];
    for (index, leaf) in foliage.instances.iter().enumerate() {
        let site = &skeleton.sites()[leaf.site as usize];
        let Some(root) = skeleton.index_of(site.branch).and_then(|b| roots[b]) else {
            continue;
        };
        if slot[root] == u32::MAX {
            slot[root] = u32::try_from(order.len()).expect("branch counts fit u32");
            order.push(root);
            members.push(Vec::new());
        }
        members[slot[root] as usize].push(u32::try_from(index).expect("leaf counts fit u32"));
        clustered[index] = true;
    }
    // Cards in branch storage order, independent of leaf order.
    let mut by_branch: Vec<usize> = (0..order.len()).collect();
    by_branch.sort_by_key(|&c| order[c]);
    let order: Vec<usize> = by_branch.iter().map(|&c| order[c]).collect();
    let members: Vec<Vec<u32>> = by_branch
        .iter()
        .map(|&c| core::mem::take(&mut members[c]))
        .collect();

    let centroid = foliage.report.crown_centroid;
    let mut cards: Vec<ClusterCard> = order
        .iter()
        .zip(&members)
        .map(|(&root, leaves)| fit_card(skeleton, foliage, &roots, root, leaves, centroid))
        .collect();

    // Exemplars at the quantiles of leaf count, ties by storage order.
    let mut ranked: Vec<usize> = (0..cards.len()).collect();
    ranked.sort_by_key(|&c| (cards[c].leaves, c));
    let count = (params.variants as usize).min(cards.len());
    let variants: Vec<ClusterVariant> = (0..count)
        .map(|i| {
            let exemplar = ranked[(2 * i + 1) * ranked.len() / (2 * count)];
            ClusterVariant {
                exemplar: u32::try_from(exemplar).expect("card counts fit u32"),
                view: cards[exemplar].view(),
            }
        })
        .collect();
    let aspect = |half: Vec2| libm::logf(half.y / half.x);
    for card in &mut cards {
        let own = aspect(card.half);
        card.variant = (0..variants.len())
            .min_by(|&a, &b| {
                let da = (aspect(variants[a].view.half) - own).abs();
                let db = (aspect(variants[b].view.half) - own).abs();
                da.total_cmp(&db)
            })
            .map_or(0, |v| u32::try_from(v).expect("few variants"));
    }
    (
        Clusters {
            params,
            cards,
            members,
            variants,
        },
        clustered,
    )
}

/// Fits a card around one cluster's leaves and bark.
fn fit_card(
    skeleton: &Skeleton,
    foliage: &Foliage,
    roots: &[Option<usize>],
    root: usize,
    leaves: &[u32],
    crown_centroid: Vec3,
) -> ClusterCard {
    let branch = &skeleton.branches()[root];
    let base = branch.nodes[0].position;
    let tip = branch.nodes[branch.nodes.len() - 1].position;
    #[expect(clippy::cast_precision_loss, reason = "a mean over leaf positions")]
    let centroid = leaves
        .iter()
        .map(|&l| foliage.instances[l as usize].position)
        .sum::<Vec3>()
        / leaves.len() as f32;
    let up = (centroid - base)
        .try_normalize()
        .or_else(|| (tip - base).try_normalize())
        .unwrap_or(Vec3::Z);
    // Face halfway between the leaves' mean upper surface and the crown's
    // outward direction: leaves mostly face the sky, so a card facing only
    // outward would see many of them edge-on, while one facing only their
    // normals would lie flat and vanish from the side.
    let outward = centroid - crown_centroid;
    let surface: Vec3 = leaves
        .iter()
        .map(|&l| foliage.instances[l as usize].rotation * Vec3::Z)
        .sum();
    let facing = outward.normalize_or_zero() + surface.normalize_or_zero();
    let toward = (facing - up * facing.dot(up))
        .try_normalize()
        .or_else(|| (outward - up * outward.dot(up)).try_normalize())
        .unwrap_or_else(|| up.any_orthonormal_vector());
    let right = up.cross(toward);

    // Every leaf blade's extent, and the bark of the cluster's branches.
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    let mut add = |p: Vec3, pad: f32| {
        let d = p - base;
        let q = Vec3::new(d.dot(right), d.dot(up), d.dot(toward));
        lo = lo.min(q - Vec3::splat(pad));
        hi = hi.max(q + Vec3::splat(pad));
    };
    for &l in leaves {
        let leaf = &foliage.instances[l as usize];
        let shape = &foliage.templates[leaf.template as usize].shape;
        let length = shape.length * leaf.scale;
        add(leaf.position, shape.max_half_width() * leaf.scale);
        add(
            leaf.position + leaf.rotation * Vec3::new(0.0, length, 0.0),
            shape.max_half_width() * leaf.scale,
        );
    }
    for (index, b) in skeleton.branches().iter().enumerate() {
        if roots[index] == Some(root) {
            for node in &b.nodes {
                add(node.position, node.radius);
            }
        }
    }
    let middle = (lo + hi) * 0.5;
    let half = ((hi - lo) * 0.5).max(Vec3::splat(1e-3));
    ClusterCard {
        root: u32::try_from(root).expect("branch counts fit u32"),
        center: base + right * middle.x + up * middle.y + toward * middle.z,
        right,
        up,
        half: Vec2::new(half.x, half.y),
        half_depth: half.z,
        leaves: u32::try_from(leaves.len()).expect("leaf counts fit u32"),
        variant: 0,
        canopy_normal: outward.try_normalize().unwrap_or(toward),
    }
}
