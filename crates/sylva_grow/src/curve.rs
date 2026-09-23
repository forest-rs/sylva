// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Piecewise-linear curves over a normalized parameter.

use alloc::vec;
use alloc::vec::Vec;

/// A piecewise-linear function of a parameter in `[0, 1]`.
///
/// Points are `(x, y)` pairs with strictly increasing `x`. Evaluation clamps
/// outside the first and last point. Growth parameters use curves for values
/// that vary along a parent branch (angles, length ratios) or up the crown
/// (envelope profiles).
///
/// # Example
/// ```rust
/// use sylva_grow::Curve;
///
/// let ratio = Curve::new(vec![[0.0, 1.0], [0.5, 0.8], [1.0, 0.2]]).unwrap();
/// assert_eq!(ratio.eval(0.25), 0.9);
/// assert_eq!(ratio.eval(2.0), 0.2);
/// assert_eq!(Curve::constant(3.0).eval(0.7), 3.0);
/// ```
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(try_from = "Vec<[f32; 2]>", into = "Vec<[f32; 2]>")
)]
pub struct Curve {
    points: Vec<[f32; 2]>,
}

/// Why a point list is not a [`Curve`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CurveError {
    /// The list is empty.
    Empty,
    /// A coordinate is NaN or infinite.
    NonFinite,
    /// `x` does not strictly increase.
    Unordered,
}

impl core::fmt::Display for CurveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Empty => "curve has no points",
            Self::NonFinite => "curve has a non-finite coordinate",
            Self::Unordered => "curve x values must strictly increase",
        })
    }
}

impl core::error::Error for CurveError {}

impl Curve {
    /// Builds a curve from `(x, y)` points with strictly increasing `x`.
    ///
    /// # Errors
    ///
    /// [`CurveError`] when the list is empty, non-finite, or unordered.
    pub fn new(points: Vec<[f32; 2]>) -> Result<Self, CurveError> {
        if points.is_empty() {
            return Err(CurveError::Empty);
        }
        if points.iter().flatten().any(|v| !v.is_finite()) {
            return Err(CurveError::NonFinite);
        }
        if points.windows(2).any(|pair| pair[0][0] >= pair[1][0]) {
            return Err(CurveError::Unordered);
        }
        Ok(Self { points })
    }

    /// A curve with value `y` everywhere.
    #[must_use]
    pub fn constant(y: f32) -> Self {
        Self {
            points: vec![[0.0, y]],
        }
    }

    /// A straight line from `a` at `x = 0` to `b` at `x = 1`.
    #[must_use]
    pub fn linear(a: f32, b: f32) -> Self {
        Self {
            points: vec![[0.0, a], [1.0, b]],
        }
    }

    /// The curve's points.
    #[must_use]
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    /// Evaluates the curve at `x`, clamping outside its points.
    #[must_use]
    pub fn eval(&self, x: f32) -> f32 {
        let first = self.points[0];
        if x <= first[0] {
            return first[1];
        }
        for pair in self.points.windows(2) {
            let ([x0, y0], [x1, y1]) = (pair[0], pair[1]);
            if x <= x1 {
                return y0 + (y1 - y0) * ((x - x0) / (x1 - x0));
            }
        }
        self.points[self.points.len() - 1][1]
    }

    /// The largest `y` of any point (the curve's maximum).
    #[must_use]
    pub fn max(&self) -> f32 {
        self.points.iter().map(|p| p[1]).fold(f32::MIN, f32::max)
    }
}

impl TryFrom<Vec<[f32; 2]>> for Curve {
    type Error = CurveError;

    fn try_from(points: Vec<[f32; 2]>) -> Result<Self, Self::Error> {
        Self::new(points)
    }
}

impl From<Curve> for Vec<[f32; 2]> {
    fn from(curve: Curve) -> Self {
        curve.points
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::{Curve, CurveError};

    #[test]
    fn evaluates_piecewise_and_clamps() {
        let c = Curve::new(vec![[0.2, 1.0], [0.6, 3.0]]).expect("curve");
        assert_eq!(c.eval(0.0), 1.0);
        assert_eq!(c.eval(0.4), 2.0);
        assert_eq!(c.eval(0.6), 3.0);
        assert_eq!(c.eval(1.0), 3.0);
        assert_eq!(c.max(), 3.0);
    }

    #[test]
    fn rejects_bad_points() {
        assert_eq!(Curve::new(vec![]), Err(CurveError::Empty));
        assert_eq!(
            Curve::new(vec![[0.0, 1.0], [0.0, 2.0]]),
            Err(CurveError::Unordered)
        );
        assert_eq!(
            Curve::new(vec![[0.0, f32::NAN]]),
            Err(CurveError::NonFinite)
        );
    }
}
