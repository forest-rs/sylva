// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Leaf contours and their coverage masks.

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
        #[expect(clippy::cast_precision_loss, reason = "station counts are small")]
        (0..=self.stations)
            .map(|i| i as f32 / self.stations as f32)
            .collect()
    }

    /// The closed outline in leaf space, counter-clockwise seen from `+Z`:
    /// up the right margin from the base, then down the left.
    #[must_use]
    pub fn outline(&self) -> Vec<Vec2> {
        let ts = self.station_ts();
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

/// An 8-bit coverage mask of a leaf blade, row-major from `v = 0`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeafMask {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Coverage per texel, 0 (empty) to 255 (inside).
    pub coverage: Vec<u8>,
}

impl LeafMask {
    /// Fraction of the mask area covered by the blade.
    #[must_use]
    pub fn coverage_fraction(&self) -> f32 {
        let sum: u64 = self.coverage.iter().map(|&c| u64::from(c)).sum();
        #[expect(clippy::cast_precision_loss, reason = "a coverage ratio")]
        let ratio = sum as f32 / (255.0 * self.coverage.len().max(1) as f32);
        ratio
    }
}

/// Rasterizes `shape` into a `size x size` coverage mask in its UV frame.
///
/// Each texel's coverage is the fraction of 4 x 4 stratified samples inside
/// the outline, tested exactly with [`LeafShape::contains`].
#[must_use]
pub fn leaf_mask(shape: &LeafShape, size: u32) -> LeafMask {
    const SUB: u32 = 4;
    let half = shape.max_half_width();
    let mut coverage = Vec::with_capacity((size * size) as usize);
    #[expect(clippy::cast_precision_loss, reason = "mask sizes are small")]
    let (n, sub) = (size as f32, SUB as f32);
    for row in 0..size {
        for col in 0..size {
            let mut inside = 0_u32;
            for sy in 0..SUB {
                for sx in 0..SUB {
                    #[expect(clippy::cast_precision_loss, reason = "mask sizes are small")]
                    let (u, v) = (
                        (col as f32 + (sx as f32 + 0.5) / sub) / n,
                        (row as f32 + (sy as f32 + 0.5) / sub) / n,
                    );
                    let x = (u - 0.5) * 2.0 * half;
                    if shape.contains(Vec2::new(x, v * shape.length)) {
                        inside += 1;
                    }
                }
            }
            #[expect(
                clippy::cast_possible_truncation,
                reason = "at most 16 samples scale into 0..=255"
            )]
            coverage.push(((inside * 255 + SUB * SUB / 2) / (SUB * SUB)) as u8);
        }
    }
    LeafMask {
        width: size,
        height: size,
        coverage,
    }
}
