// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Orthonormal frames along branch centerlines.

use glam::Vec3;

/// Default tolerance for [`Frame::is_orthonormal`].
pub const FRAME_EPSILON: f32 = 1e-4;

/// A right-handed orthonormal frame: `tangent` along the centerline, `normal`
/// across it, and `binormal = tangent × normal`.
///
/// Branch meshing uses the normal as the ring's zero angle, so bark UV seams
/// follow it. Frames along a branch are rotation-minimizing (see
/// [`Frame::transport`]) so rings do not twist.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Frame {
    /// Unit direction along the centerline.
    pub tangent: Vec3,
    /// Unit direction perpendicular to the tangent.
    pub normal: Vec3,
}

impl Default for Frame {
    /// The frame with tangent `+Z` and normal `+X`.
    fn default() -> Self {
        Self {
            tangent: Vec3::Z,
            normal: Vec3::X,
        }
    }
}

impl Frame {
    /// The third axis, `tangent × normal`.
    #[must_use]
    pub fn binormal(&self) -> Vec3 {
        self.tangent.cross(self.normal)
    }

    /// True when both axes are unit length and perpendicular within `epsilon`.
    #[must_use]
    pub fn is_orthonormal(&self, epsilon: f32) -> bool {
        self.tangent.is_finite()
            && self.normal.is_finite()
            && (self.tangent.length_squared() - 1.0).abs() <= epsilon
            && (self.normal.length_squared() - 1.0).abs() <= epsilon
            && self.tangent.dot(self.normal).abs() <= epsilon
    }

    /// Builds a frame around `tangent` whose normal is `reference` made
    /// perpendicular to it.
    ///
    /// Returns `None` when `tangent` is degenerate or `reference` is parallel
    /// to it; callers choose their own fallback and count it.
    #[must_use]
    pub fn from_tangent(tangent: Vec3, reference: Vec3) -> Option<Self> {
        let tangent = tangent.try_normalize()?;
        let normal = (reference - tangent * reference.dot(tangent)).try_normalize()?;
        Some(Self { tangent, normal })
    }

    /// Builds a frame around `tangent`, trying `reference`, then `+X`, then
    /// `+Y`. The second value is true when `reference` could not be used.
    ///
    /// Returns `None` only when `tangent` is degenerate.
    #[must_use]
    pub fn from_tangent_or_axis(tangent: Vec3, reference: Vec3) -> Option<(Self, bool)> {
        if let Some(frame) = Self::from_tangent(tangent, reference) {
            return Some((frame, false));
        }
        Self::from_tangent(tangent, Vec3::X)
            .or_else(|| Self::from_tangent(tangent, Vec3::Y))
            .map(|frame| (frame, true))
    }

    /// Carries this frame from `from` to `to`, ending with `next_tangent`,
    /// by the double-reflection method (Wang et al., *Computation of Rotation
    /// Minimizing Frames*, 2008).
    ///
    /// The result is re-orthonormalized so error does not accumulate along long
    /// branches. A zero-length step keeps the normal and only re-projects it.
    #[must_use]
    pub fn transport(&self, from: Vec3, to: Vec3, next_tangent: Vec3) -> Self {
        let v1 = to - from;
        let c1 = v1.dot(v1);
        let (reflected_normal, reflected_tangent) = if c1 > f32::MIN_POSITIVE {
            (
                self.normal - v1 * (2.0 / c1 * v1.dot(self.normal)),
                self.tangent - v1 * (2.0 / c1 * v1.dot(self.tangent)),
            )
        } else {
            (self.normal, self.tangent)
        };
        let v2 = next_tangent - reflected_tangent;
        let c2 = v2.dot(v2);
        let normal = if c2 > f32::MIN_POSITIVE {
            reflected_normal - v2 * (2.0 / c2 * v2.dot(reflected_normal))
        } else {
            reflected_normal
        };
        Self::from_tangent(next_tangent, normal).unwrap_or(Self {
            tangent: next_tangent.normalize_or(self.tangent),
            normal: self.normal,
        })
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::{FRAME_EPSILON, Frame};

    #[test]
    fn transport_along_a_straight_line_keeps_the_frame() {
        let frame = Frame::default();
        let moved = frame.transport(Vec3::ZERO, Vec3::Z, Vec3::Z);
        assert!(moved.is_orthonormal(FRAME_EPSILON));
        assert!((moved.normal - Vec3::X).length() < 1e-6);
    }

    #[test]
    fn transport_around_a_planar_bend_does_not_twist() {
        // A quarter circle in the XZ plane: the binormal (+Y or -Y) of a
        // rotation-minimizing frame stays fixed for a planar curve.
        let mut frame = Frame::from_tangent(Vec3::Z, Vec3::X).expect("frame");
        let binormal = frame.binormal();
        let steps = 32;
        let point = |i: i32| {
            let a = core::f32::consts::FRAC_PI_2 * i as f32 / steps as f32;
            Vec3::new(1.0 - a.cos(), 0.0, a.sin())
        };
        let tangent = |i: i32| {
            let a = core::f32::consts::FRAC_PI_2 * i as f32 / steps as f32;
            Vec3::new(a.sin(), 0.0, a.cos())
        };
        for i in 0..steps {
            frame = frame.transport(point(i), point(i + 1), tangent(i + 1));
            assert!(frame.is_orthonormal(FRAME_EPSILON));
        }
        assert!((frame.binormal() - binormal).length() < 1e-4);
        assert!((frame.tangent - Vec3::X).length() < 1e-4);
    }

    #[test]
    fn degenerate_references_fall_back() {
        assert!(Frame::from_tangent(Vec3::X, Vec3::X).is_none());
        let (frame, fell_back) = Frame::from_tangent_or_axis(Vec3::X, Vec3::X).expect("frame");
        assert!(fell_back);
        assert!(frame.is_orthonormal(FRAME_EPSILON));
        assert!(Frame::from_tangent_or_axis(Vec3::ZERO, Vec3::X).is_none());
    }
}
