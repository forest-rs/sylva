// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leaf contours.

use alloc::vec::Vec;
use core::f32::consts::PI;

use glam::Vec2;

/// A leaf blade described by its outline along the midrib.
///
/// Leaf space puts the blade base at the origin, the midrib along `+Y`, the
/// blade across `X`, and the upper surface facing `+Z`. The outline is a
/// half-width function of the midrib parameter `t` in `[0, 1]`:
///
/// ```text
/// half(t) = width * length / 2 * envelope(t) * lobing(t)
/// ```
///
/// `envelope` is a unimodal profile, zero at base and tip and 1 at
/// `widest_at`; `lobing` cuts narrow sinuses of relative depth `lobe_depth`
/// between broad rounded lobes along each side, deepest mid-blade. An
/// optional `auricle` adds a small rounded ear on each side of the base.
///
/// The blade then sweeps forward: [`LeafShape::skewed`] moves every point
/// toward the tip in proportion to its distance from the midrib, by
/// `lobe_skew`, tapering to nothing at the tip. Lobe tips, far from the
/// midrib, move further than the sinuses between them, so lobes point
/// toward the tip as an oak's do. The midrib does not move.
///
/// The leaf mesh, the card and the coverage mask all derive from this one
/// outline ([`LeafShape::contains`]) and cannot disagree.
///
/// # Leaflets
///
/// A compound blade (a conifer's needle spray, a fern or palm frond, an ash
/// leaf) is a midrib carrying `leaflets` narrow leaflets a side. The outline
/// is then the frond's envelope: the mesh still covers it, and the coverage
/// mask ([`LeafShape::covers`]) keeps only the midrib and the leaflets, so
/// alpha testing cuts the strip into needles. Leaflets start square to the
/// midrib, evenly spaced from `leaflet_span` of the way up to the tip, reach
/// the outline, and sweep forward with the rest of the blade by
/// `lobe_skew`.
#[derive(Copy, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct LeafShape {
    /// Blade length along the midrib, in metres.
    pub length: f32,
    /// Largest blade width, as a fraction of the length.
    pub width: f32,
    /// Midrib parameter of the widest point, in `(0, 1)`. Above 0.5 gives
    /// an obovate blade, widest toward the tip.
    pub widest_at: f32,
    /// Sharpness of the tip, in `(0, 1]`: small values round the tip, 1
    /// narrows it to a point.
    pub tip: f32,
    /// Rounded lobes along each side; 0 for an entire margin.
    pub lobes: u32,
    /// Depth of the sinuses between lobes, as a fraction of the half-width,
    /// in `[0, 1)`.
    pub lobe_depth: f32,
    /// Petiole (leaf stalk) length, in metres. Placement offsets the blade
    /// by it; the stalk itself is not meshed.
    pub petiole: f32,
    /// V-fold along the midrib, in radians: each half rises by this angle.
    pub fold: f32,
    /// Droop of the tip below the base plane, as a fraction of the length.
    pub curl: f32,
    /// Stations along the midrib for meshing; at least 3.
    pub stations: u32,
    /// Forward sweep of the blade toward the tip, in `[0, 2)`: a point at
    /// distance `x` from the midrib moves `lobe_skew * |x|` toward the tip,
    /// tapering to zero there. 0 keeps lobes square to the midrib.
    pub lobe_skew: f32,
    /// Half-width of the basal ears, as a fraction of the length; 0 for
    /// none.
    pub auricle: f32,
    /// Leaflets along each side of the midrib; 0 for a simple blade.
    pub leaflets: u32,
    /// Width of each leaflet (and of the midrib carrying them), as a
    /// fraction of the length.
    pub leaflet_width: f32,
    /// Midrib parameter of the lowest leaflet, in `[0, 1)`; below it the
    /// midrib is bare, a stalk.
    pub leaflet_span: f32,
}

impl Default for LeafShape {
    /// A pedunculate-oak-like blade: obovate, five rounded lobes a side.
    fn default() -> Self {
        Self {
            length: 0.1,
            width: 0.6,
            widest_at: 0.65,
            tip: 0.6,
            lobes: 5,
            lobe_depth: 0.4,
            petiole: 0.006,
            fold: 0.15,
            curl: 0.08,
            stations: 32,
            lobe_skew: 0.5,
            auricle: 0.04,
            leaflets: 0,
            leaflet_width: 0.02,
            leaflet_span: 0.0,
        }
    }
}

impl LeafShape {
    /// Half-width of the blade at midrib parameter `t`, in metres; zero
    /// outside `[0, 1]`.
    #[must_use]
    pub fn half_width(&self, t: f32) -> f32 {
        if !(0.0..=1.0).contains(&t) {
            return 0.0;
        }
        let blade = 0.5 * self.width * self.length * self.envelope(t) * self.lobing(t);
        blade + self.auricle * self.length * Self::ear(t)
    }

    /// The basal ear profile: one rounded bump over the first 15% of the
    /// blade.
    fn ear(t: f32) -> f32 {
        const SPAN: f32 = 0.15;
        if t >= SPAN {
            return 0.0;
        }
        let s = libm::sinf(PI * t / SPAN);
        s * s
    }

    /// Moves a point of the unswept blade (`y = t * length`) to the swept
    /// blade: `y + lobe_skew * |x| * (1 - y / length)`.
    #[must_use]
    pub fn skewed(&self, p: Vec2) -> Vec2 {
        let a = self.lobe_skew * p.x.abs();
        Vec2::new(p.x, p.y + a * (1.0 - p.y / self.length))
    }

    /// The inverse of [`Self::skewed`].
    #[must_use]
    pub fn unskewed(&self, p: Vec2) -> Vec2 {
        let a = self.lobe_skew * p.x.abs();
        Vec2::new(p.x, (p.y - a) / (1.0 - a / self.length))
    }

    /// True when leaf-plane point `p` lies on the (swept) blade.
    #[must_use]
    pub fn contains(&self, p: Vec2) -> bool {
        let flat = self.unskewed(p);
        let t = flat.y / self.length;
        (0.0..=1.0).contains(&t) && p.x.abs() <= self.half_width(t)
    }

    /// True when leaf-plane point `p` lies on leaf tissue: inside the
    /// outline, and for a compound blade on the midrib or a leaflet. This
    /// is what the coverage mask holds; for a simple blade it is
    /// [`Self::contains`].
    #[must_use]
    pub fn covers(&self, p: Vec2) -> bool {
        if !self.contains(p) {
            return false;
        }
        if self.leaflets == 0 {
            return true;
        }
        let flat = self.unskewed(p);
        let half = 0.5 * self.leaflet_width * self.length;
        if flat.x.abs() <= half {
            return true;
        }
        let t = flat.y / self.length;
        self.leaflet_ts().into_iter().any(|at| {
            // Each leaflet tapers to a point over its outer third.
            let reach = self.half_width(at).max(1e-9);
            let taper = (3.0 * (1.0 - flat.x.abs() / reach)).clamp(0.0, 1.0);
            libm::fabsf(t - at) * self.length <= half * taper
        })
    }

    /// Midrib parameters of the leaflets on each side, lowest first.
    #[must_use]
    pub fn leaflet_ts(&self) -> Vec<f32> {
        let n = self.leaflets;
        #[expect(clippy::cast_precision_loss, reason = "leaflet counts are small")]
        (0..n)
            .map(|k| self.leaflet_span + (1.0 - self.leaflet_span) * (k as f32 + 0.5) / n as f32)
            .collect()
    }

    /// The tissue as closed polygons in leaf space, counter-clockwise seen
    /// from `+Z`: the outline for a simple blade; the midrib and each
    /// leaflet for a compound one. Their union is [`Self::covers`], up to
    /// sampling. `stations` samples the outline and midrib as in
    /// [`Self::outline_at`].
    #[must_use]
    pub fn tissue_at(&self, stations: u32) -> Vec<Vec<Vec2>> {
        if self.leaflets == 0 {
            return alloc::vec![self.outline_at(stations)];
        }
        let half = 0.5 * self.leaflet_width * self.length;
        let ts = Self::ts(stations.max(2));
        // The midrib: a strip clipped to the outline near base and tip.
        let rib = |side: f32, t: f32| {
            self.skewed(Vec2::new(
                side * half.min(self.half_width(t).max(0.25 * half)),
                t * self.length,
            ))
        };
        let mut midrib: Vec<Vec2> = ts.iter().map(|&t| rib(1.0, t)).collect();
        midrib.extend(ts.iter().rev().map(|&t| rib(-1.0, t)));
        let mut polygons = alloc::vec![midrib];
        const SAMPLES: u32 = 8;
        for at in self.leaflet_ts() {
            let reach = self.half_width(at);
            let y = at * self.length;
            for side in [1.0_f32, -1.0] {
                // Along the leaflet's lower edge out to its tip, then back
                // along its upper edge.
                let mut leaflet = Vec::with_capacity(2 * SAMPLES as usize + 1);
                #[expect(clippy::cast_precision_loss, reason = "sample counts are small")]
                let edge = |k: u32, lower: f32| {
                    let x = reach * k as f32 / SAMPLES as f32;
                    let taper = (3.0 * (1.0 - x / reach.max(1e-9))).clamp(0.0, 1.0);
                    self.skewed(Vec2::new(side * x, y + lower * half * taper))
                };
                for k in 0..=SAMPLES {
                    leaflet.push(edge(k, -side));
                }
                for k in (0..SAMPLES).rev() {
                    leaflet.push(edge(k, side));
                }
                polygons.push(leaflet);
            }
        }
        polygons
    }

    /// Unimodal profile `t^p (1 - t)^q`, normalized to 1 at `widest_at`.
    fn envelope(&self, t: f32) -> f32 {
        let m = self.widest_at;
        let q = self.tip;
        let p = m * q / (1.0 - m);
        let peak = libm::powf(m, p) * libm::powf(1.0 - m, q);
        libm::powf(t, p) * libm::powf(1.0 - t, q) / peak
    }

    /// Broad rounded lobes separated by narrow sinuses (`sin^6`), deepest
    /// mid-blade and fading toward base and tip.
    fn lobing(&self, t: f32) -> f32 {
        if self.lobes == 0 {
            return 1.0;
        }
        #[expect(clippy::cast_precision_loss, reason = "lobe counts are small")]
        let s = libm::sinf(PI * (self.lobes as f32 + 0.5) * t);
        let s2 = s * s;
        let depth = self.lobe_depth * libm::sqrtf(libm::sinf(PI * t).max(0.0));
        1.0 - depth * s2 * s2 * s2
    }

    /// Largest half-width the outline can reach: the mask and card bounds.
    #[must_use]
    pub fn max_half_width(&self) -> f32 {
        (0.5 * self.width + self.auricle) * self.length
    }

    /// Midrib parameters of the meshing stations, from 0 to 1.
    pub(crate) fn station_ts(&self) -> Vec<f32> {
        Self::ts(self.stations)
    }

    /// `stations + 1` evenly spaced midrib parameters, from 0 to 1.
    fn ts(stations: u32) -> Vec<f32> {
        #[expect(clippy::cast_precision_loss, reason = "station counts are small")]
        (0..=stations).map(|i| i as f32 / stations as f32).collect()
    }

    /// The closed outline in leaf space at the meshing stations,
    /// counter-clockwise seen from `+Z`: up the right margin from the base,
    /// then down the left. The blade mesh's rim follows it exactly.
    #[must_use]
    pub fn outline(&self) -> Vec<Vec2> {
        self.outline_at(self.stations)
    }

    /// The same closed outline sampled at `stations` (at least 2) evenly
    /// spaced midrib parameters instead of the meshing stations: finer for
    /// masks, which resolve lobes a coarse mesh rim cuts across.
    #[must_use]
    pub fn outline_at(&self, stations: u32) -> Vec<Vec2> {
        let ts = Self::ts(stations.max(2));
        let mut points: Vec<Vec2> = ts
            .iter()
            .map(|&t| self.skewed(Vec2::new(self.half_width(t), t * self.length)))
            .collect();
        points.extend(
            ts.iter()
                .rev()
                .skip(1)
                .take(ts.len() - 2)
                .map(|&t| self.skewed(Vec2::new(-self.half_width(t), t * self.length))),
        );
        points
    }

    /// Texture coordinates of a leaf-space point: the mask and card cover
    /// `[-max_half_width, max_half_width] x [0, length]`. As for bark, `v = 0`
    /// is the first texel row (the blade base), and U, V run right-handed about
    /// the upper surface.
    #[must_use]
    pub fn uv(&self, p: Vec2) -> [f32; 2] {
        let half = self.max_half_width();
        [0.5 + 0.5 * p.x / half, p.y / self.length]
    }
}
