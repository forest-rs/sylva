// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Measured realism for sylva trees.
//!
//! [`measure`] reads a grown [`Skeleton`] and returns its [`TreeMeasures`]:
//! the allometry foresters record for real trees (height, crown width,
//! stem diameter at breast height, crown base) and the ratios between them,
//! stem taper, branch insertion angles per order, and statistics of the
//! crown's side silhouette, including the sky seen through it. Leaves are
//! taken at the skeleton's foliage sites, so a tree can be measured without
//! placing its leaves.
//!
//! [`TreeMeasures::report`] writes them in `dapple_lab`'s report format,
//! bounded by a species [`Reference`]: a list of named ranges, each with
//! its source. [`Reference::targets`] turns the same ranges into
//! `dapple_lab::fit` targets, so the numbers a regression test checks are
//! also the loss an inverse fit minimizes.
//!
//! Every measurement's name is listed in [`TreeMeasures::values`]; a
//! reference may bound any of them, and names it does not know stay
//! unbounded.
//!
//! # References
//!
//! Sylva's species references take their ranges from forestry and
//! allometry literature, as derived statistics only. The ones in the
//! gallery's presets draw on:
//!
//! - Pretzsch, H., Biber, P., Uhl, E., et al. (2015). *Crown size and
//!   growing space requirement of common tree species in urban centres,
//!   parks, and forests.* Urban Forestry & Urban Greening 14, 466–479:
//!   open-grown crown radius, height and crown projection against stem
//!   diameter.
//! - Hemery, G. E., Savill, P. S., Pryor, S. N. (2005). *Applications of
//!   the crown diameter–stem diameter relationship for different species of
//!   broadleaved trees.* Forest Ecology and Management 215, 285–294: crown
//!   to stem diameter ratios of British broadleaves, oak included.
//! - Kantola, A., Mäkelä, A. (2004). *Crown development in Norway spruce
//!   [Picea abies (L.) Karst.].* Trees 18, 408–421: crown length, width and
//!   branch structure of Norway spruce.
//!
//! Each range records which source it follows, or that it is a review
//! target of sylva's own where the literature gives no number (sky
//! fraction, texture colour).
//!
//! # Example
//! ```rust
//! use sylva_measure::{Range, Reference, measure};
//! use sylva_skeleton::glam::Vec3;
//! use sylva_skeleton::{Branch, BranchId, Node, Skeleton};
//!
//! let mut skeleton = Skeleton::new();
//! let mut nodes: Vec<Node> = (0..=10)
//!     .map(|i| Node::at(Vec3::new(0.0, 0.0, i as f32)))
//!     .collect();
//! for node in &mut nodes {
//!     node.radius = 0.2;
//! }
//! skeleton.push_branch(Branch { id: BranchId::root(0), order: 0, parent: None, nodes })?;
//! let measures = measure(&skeleton, 0.05);
//! assert_eq!(measures.height, 10.0);
//! let reference = Reference {
//!     species: "pole".into(),
//!     ranges: vec![Range::new("allometry.height", 8.0, 12.0, "test")],
//! };
//! assert!(measures.report("pole", &reference).passed());
//! # Ok::<(), sylva_skeleton::SkeletonError>(())
//! ```

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use dapple_lab::Report;
use dapple_lab::fit::Target;
use sylva_skeleton::Skeleton;
use sylva_skeleton::glam::{Vec2, Vec3};

/// Breast height, in metres, where stem diameter is measured.
pub const BREAST_HEIGHT: f32 = 1.3;

/// Side views the silhouette statistics average over.
const VIEWS: usize = 4;
/// Silhouette pixels per metre.
const PIXELS_PER_METRE: f32 = 10.0;
/// Branches thinner than this (radius, metres) are left out of silhouettes;
/// their leaves carry them.
const SILHOUETTE_MIN_RADIUS: f32 = 0.02;

/// A tree's measurements; see the [crate docs](crate).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeMeasures {
    /// Top of the highest node, metres.
    pub height: f32,
    /// Mean crown diameter over eight directions, metres: the spread of the
    /// foliage sites across each.
    pub crown_width: f32,
    /// Height of the crown's base, metres: the 2nd percentile of foliage
    /// site heights, so a stray low twig does not lower it.
    pub crown_base: f32,
    /// Stem diameter at [`BREAST_HEIGHT`], metres, from the root stem's
    /// pipe-model radius (without root flare).
    pub dbh: f32,
    /// Stem diameter halfway up the root stem over [`Self::dbh`].
    pub stem_taper: f32,
    /// Mean and standard deviation of branch insertion angles, degrees
    /// between a branch's first internode and its parent's tangent, per
    /// order from 1.
    pub branch_angles: Vec<(u32, f32, f32)>,
    /// Sky fraction through the crown's side silhouette.
    pub sky_fraction: f32,
    /// Height over width of the crown's side silhouette.
    pub silhouette_aspect: f32,
    /// Foliage sites measured.
    pub sites: usize,
}

impl TreeMeasures {
    /// Height over crown width.
    #[must_use]
    pub fn height_to_crown_width(&self) -> f32 {
        self.height / self.crown_width.max(1e-6)
    }

    /// Crown width over stem diameter at breast height (Hemery's `K/d`).
    #[must_use]
    pub fn crown_width_to_dbh(&self) -> f32 {
        self.crown_width / self.dbh.max(1e-6)
    }

    /// Height over stem diameter at breast height (slenderness).
    #[must_use]
    pub fn slenderness(&self) -> f32 {
        self.height / self.dbh.max(1e-6)
    }

    /// Crown length over height.
    #[must_use]
    pub fn crown_ratio(&self) -> f32 {
        (self.height - self.crown_base) / self.height.max(1e-6)
    }

    /// Every measurement by name, with its unit, in report order.
    #[must_use]
    pub fn values(&self) -> Vec<(String, f64, &'static str)> {
        let mut out: Vec<(String, f64, &'static str)> = vec![
            ("allometry.height".into(), f64::from(self.height), "m"),
            (
                "allometry.crown_width".into(),
                f64::from(self.crown_width),
                "m",
            ),
            (
                "allometry.crown_base".into(),
                f64::from(self.crown_base),
                "m",
            ),
            ("allometry.dbh".into(), f64::from(self.dbh), "m"),
            (
                "allometry.height_to_crown_width".into(),
                f64::from(self.height_to_crown_width()),
                "",
            ),
            (
                "allometry.crown_width_to_dbh".into(),
                f64::from(self.crown_width_to_dbh()),
                "",
            ),
            (
                "allometry.slenderness".into(),
                f64::from(self.slenderness()),
                "",
            ),
            (
                "allometry.crown_ratio".into(),
                f64::from(self.crown_ratio()),
                "",
            ),
            ("form.stem_taper".into(), f64::from(self.stem_taper), ""),
        ];
        for &(order, mean, deviation) in &self.branch_angles {
            out.push((
                alloc::format!("form.branch_angle.order{order}.mean"),
                f64::from(mean),
                "deg",
            ));
            out.push((
                alloc::format!("form.branch_angle.order{order}.sd"),
                f64::from(deviation),
                "deg",
            ));
        }
        out.push((
            "crown.sky_fraction".into(),
            f64::from(self.sky_fraction),
            "",
        ));
        out.push((
            "crown.silhouette_aspect".into(),
            f64::from(self.silhouette_aspect),
            "",
        ));
        out
    }

    /// The value named `name` in [`Self::values`].
    #[must_use]
    pub fn value(&self, name: &str) -> Option<f64> {
        self.values()
            .into_iter()
            .find(|(n, ..)| n == name)
            .map(|(_, v, _)| v)
    }

    /// A report of every measurement, each bounded by `reference`'s range
    /// of the same name if it has one. Reference ranges with no matching
    /// measurement are reported as failed checks.
    #[must_use]
    pub fn report(&self, subject: &str, reference: &Reference) -> Report {
        let mut report = Report::new(subject);
        let values = self.values();
        for (name, value, unit) in &values {
            let bounds = reference
                .ranges
                .iter()
                .find(|r| &r.name == name)
                .map(|r| [r.lo, r.hi]);
            report.measure(name, *value, unit, bounds);
        }
        for range in &reference.ranges {
            if !values.iter().any(|(n, ..)| n == &range.name) && !range.name.starts_with("colour.")
            {
                report.check(&range.name, false, "no such measurement");
            }
        }
        report
    }
}

/// One reference range.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Range {
    /// The measurement it bounds, as in [`TreeMeasures::values`].
    pub name: String,
    /// Lowest accepted value.
    pub lo: f64,
    /// Highest accepted value.
    pub hi: f64,
    /// Where the range comes from.
    pub source: String,
}

impl Range {
    /// A range named `name` from `lo` to `hi`, following `source`.
    #[must_use]
    pub fn new(name: &str, lo: f64, hi: f64, source: &str) -> Self {
        Self {
            name: name.into(),
            lo,
            hi,
            source: source.into(),
        }
    }
}

/// A species' reference ranges.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Reference {
    /// The species the ranges describe.
    pub species: String,
    /// The ranges.
    pub ranges: Vec<Range>,
}

impl Reference {
    /// Fit targets for the ranges whose names start with `prefix`: each
    /// asks for the range's middle, with half its width as the tolerance,
    /// so a value at either end of the range costs one unit of loss.
    #[must_use]
    pub fn targets(&self, prefix: &str) -> Vec<Target> {
        self.ranges
            .iter()
            .filter(|r| r.name.starts_with(prefix))
            .map(|r| Target::equal(&r.name, 0.5 * (r.lo + r.hi), 0.5 * (r.hi - r.lo)))
            .collect()
    }
}

/// Measures a grown skeleton; `leaf_radius` (metres) is the radius each
/// foliage site covers in the crown's silhouette.
#[must_use]
pub fn measure(skeleton: &Skeleton, leaf_radius: f32) -> TreeMeasures {
    let branches = skeleton.branches();
    let height = branches
        .iter()
        .flat_map(|b| &b.nodes)
        .map(|n| n.position.z)
        .fold(0.0, f32::max);
    let leaves: Vec<Vec3> = skeleton
        .sites()
        .iter()
        .filter_map(|s| Some(skeleton.branch(s.branch)?.sample(s.t).position))
        .collect();

    let mut crown_width = 0.0;
    for k in 0..8 {
        #[expect(clippy::cast_precision_loss, reason = "eight directions")]
        let angle = core::f32::consts::PI * k as f32 / 8.0;
        let dir = Vec2::new(libm::cosf(angle), libm::sinf(angle));
        let (lo, hi) = leaves
            .iter()
            .map(|p| p.truncate().dot(dir))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), x| {
                (lo.min(x), hi.max(x))
            });
        if lo <= hi {
            crown_width += (hi - lo) / 8.0;
        }
    }
    let crown_base = {
        let mut z: Vec<f32> = leaves.iter().map(|p| p.z).collect();
        z.sort_by(f32::total_cmp);
        z.get(z.len() / 50).copied().unwrap_or(height)
    };

    let (dbh, stem_taper) = match branches.iter().find(|b| b.parent.is_none()) {
        Some(trunk) => {
            let radius_at_height = |z: f32| {
                trunk
                    .nodes
                    .windows(2)
                    .find(|w| w[0].position.z <= z && z <= w[1].position.z)
                    .map_or(trunk.nodes[0].radius, |w| {
                        let span = (w[1].position.z - w[0].position.z).max(1e-6);
                        let f = (z - w[0].position.z) / span;
                        w[0].radius + (w[1].radius - w[0].radius) * f
                    })
            };
            let dbh = 2.0 * radius_at_height(BREAST_HEIGHT);
            let middle = 2.0 * trunk.sample(0.5).radius;
            (dbh, middle / dbh.max(1e-6))
        }
        None => (0.0, 0.0),
    };

    let max_order = branches.iter().map(|b| b.order).max().unwrap_or(0);
    let mut branch_angles = Vec::new();
    for order in 1..=max_order {
        let angles: Vec<f32> = branches
            .iter()
            .filter(|b| b.order == order && b.nodes.len() >= 2)
            .filter_map(|b| {
                let a = b.parent?;
                let tangent = skeleton.branch(a.parent)?.sample(a.t).frame.tangent;
                let first = (b.nodes[1].position - b.nodes[0].position).try_normalize()?;
                Some(libm::acosf(first.dot(tangent).clamp(-1.0, 1.0)).to_degrees())
            })
            .collect();
        if angles.is_empty() {
            continue;
        }
        #[expect(clippy::cast_precision_loss, reason = "a mean over branches")]
        let n = angles.len() as f32;
        let mean = angles.iter().sum::<f32>() / n;
        let variance = angles.iter().map(|a| (a - mean) * (a - mean)).sum::<f32>() / n;
        branch_angles.push((order, mean, libm::sqrtf(variance)));
    }

    let (sky_fraction, silhouette_aspect) = silhouette(skeleton, &leaves, leaf_radius);
    TreeMeasures {
        height,
        crown_width,
        crown_base,
        dbh,
        stem_taper,
        branch_angles,
        sky_fraction,
        silhouette_aspect,
        sites: leaves.len(),
    }
}

/// Side silhouettes of the crown from [`VIEWS`] azimuths: the sky fraction
/// through it and its height over width, averaged.
///
/// Leaves are disks of `leaf_radius`; branches of at least
/// [`SILHOUETTE_MIN_RADIUS`] are their centerlines drawn at their radius.
/// Within each image row, the crown spans from its leftmost to its
/// rightmost covered pixel; the sky fraction is the uncovered share of
/// those spans over the crown's height (the rows from the crown's base up).
fn silhouette(skeleton: &Skeleton, leaves: &[Vec3], leaf_radius: f32) -> (f32, f32) {
    if leaves.is_empty() {
        return (0.0, 0.0);
    }
    let reach = leaves
        .iter()
        .map(|p| p.truncate().length())
        .fold(0.0, f32::max)
        + 1.0;
    let top = leaves.iter().map(|p| p.z).fold(0.0, f32::max) + 1.0;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "tree extents in metres are small and positive"
    )]
    let (w, h) = (
        (2.0 * reach * PIXELS_PER_METRE) as usize + 1,
        (top * PIXELS_PER_METRE) as usize + 1,
    );
    let crown_base = leaves.iter().map(|p| p.z).fold(f32::INFINITY, f32::min);
    let mut sky = 0.0;
    let mut aspect = 0.0;
    for view in 0..VIEWS {
        #[expect(clippy::cast_precision_loss, reason = "few views")]
        let angle = core::f32::consts::PI * view as f32 / VIEWS as f32;
        let across = Vec2::new(libm::cosf(angle), libm::sinf(angle));
        let mut image = vec![false; w * h];
        let mut disk = |p: Vec3, r: f32| {
            let x = (p.truncate().dot(across) + reach) * PIXELS_PER_METRE;
            let y = p.z * PIXELS_PER_METRE;
            let rp = (r * PIXELS_PER_METRE).max(0.5);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to the image below"
            )]
            let (x0, x1, y0, y1) = (
                (x - rp).max(0.0) as usize,
                ((x + rp) as usize).min(w - 1),
                (y - rp).max(0.0) as usize,
                ((y + rp) as usize).min(h - 1),
            );
            for py in y0..=y1 {
                for px in x0..=x1 {
                    #[expect(clippy::cast_precision_loss, reason = "pixel indices are small")]
                    let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
                    if dx * dx + dy * dy <= rp * rp {
                        image[py * w + px] = true;
                    }
                }
            }
        };
        for leaf in leaves {
            disk(*leaf, leaf_radius);
        }
        for branch in skeleton.branches() {
            for pair in branch.nodes.windows(2) {
                let r = pair[0].radius;
                if r < SILHOUETTE_MIN_RADIUS {
                    continue;
                }
                let length = pair[0].position.distance(pair[1].position);
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "segment lengths in pixels are small"
                )]
                let steps = (length * PIXELS_PER_METRE / (0.5 * r * PIXELS_PER_METRE).max(0.5))
                    as usize
                    + 1;
                for i in 0..=steps {
                    #[expect(clippy::cast_precision_loss, reason = "few steps")]
                    let t = i as f32 / steps as f32;
                    disk(pair[0].position.lerp(pair[1].position, t), r);
                }
            }
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the crown base is within the image"
        )]
        let first_row = ((crown_base * PIXELS_PER_METRE).max(0.0) as usize).min(h - 1);
        let (mut covered, mut spanned, mut widest, mut rows) = (0_usize, 0_usize, 0_usize, 0_usize);
        for row in image[first_row * w..].chunks(w) {
            let (Some(left), Some(right)) =
                (row.iter().position(|&c| c), row.iter().rposition(|&c| c))
            else {
                continue;
            };
            rows += 1;
            spanned += right - left + 1;
            widest = widest.max(right - left + 1);
            covered += row[left..=right].iter().filter(|&&c| c).count();
        }
        #[expect(clippy::cast_precision_loss, reason = "pixel counts")]
        {
            sky += 1.0 - covered as f32 / spanned.max(1) as f32;
            aspect += rows as f32 / widest.max(1) as f32;
        }
    }
    #[expect(clippy::cast_precision_loss, reason = "few views")]
    let views = VIEWS as f32;
    (sky / views, aspect / views)
}

#[cfg(test)]
mod tests;
