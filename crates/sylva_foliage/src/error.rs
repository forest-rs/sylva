// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Foliage failures and parameter validation.

use core::fmt;

use exedra_mesh::BuildError;

use crate::{FoliageParams, LeafShape};

/// Why a foliage operation failed.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FoliageError {
    /// A parameter is out of range.
    Params {
        /// The offending parameter.
        name: &'static str,
    },
    /// The mesh kernel rejected a leaf template.
    Build(BuildError),
    /// A mesh kernel operation on a freshly built element failed.
    Kernel,
    /// A site names a branch the skeleton does not contain.
    MissingBranch,
    /// The skeleton has more sites than a `u32` index can name.
    TooLarge,
}

impl fmt::Display for FoliageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Params { name } => write!(f, "foliage parameter {name} is out of range"),
            Self::Build(error) => write!(f, "mesh kernel rejected a leaf: {error}"),
            Self::Kernel => f.write_str("mesh kernel refused a leaf attribute write"),
            Self::MissingBranch => f.write_str("a site names a missing branch"),
            Self::TooLarge => f.write_str("too many sites to index"),
        }
    }
}

impl core::error::Error for FoliageError {}

fn check(ok: bool, name: &'static str) -> Result<(), FoliageError> {
    if ok {
        Ok(())
    } else {
        Err(FoliageError::Params { name })
    }
}

fn positive(v: f32) -> bool {
    v.is_finite() && v > 0.0
}

impl LeafShape {
    /// Checks every parameter range.
    ///
    /// # Errors
    ///
    /// [`FoliageError::Params`] naming the first parameter out of range.
    pub fn validate(&self) -> Result<(), FoliageError> {
        check(positive(self.length), "shape.length")?;
        check(positive(self.width), "shape.width")?;
        check(
            self.widest_at > 0.0 && self.widest_at < 1.0,
            "shape.widest_at",
        )?;
        check(self.tip > 0.0 && self.tip <= 1.0, "shape.tip")?;
        check(self.lobes <= 64, "shape.lobes")?;
        check((0.0..1.0).contains(&self.lobe_depth), "shape.lobe_depth")?;
        check(
            self.petiole.is_finite() && self.petiole >= 0.0,
            "shape.petiole",
        )?;
        check(self.fold.is_finite() && self.fold.abs() < 1.5, "shape.fold")?;
        check(self.curl.is_finite(), "shape.curl")?;
        check((3..=1024).contains(&self.stations), "shape.stations")?;
        check(
            self.auricle.is_finite() && (0.0..0.5).contains(&self.auricle),
            "shape.auricle",
        )?;
        check(
            self.lobe_skew.is_finite()
                && (0.0..2.0).contains(&self.lobe_skew)
                && self.lobe_skew * self.max_half_width() < self.length,
            "shape.lobe_skew",
        )?;
        check(self.leaflets <= 1024, "shape.leaflets")?;
        check(
            self.leaflet_width.is_finite() && self.leaflet_width > 0.0 && self.leaflet_width < 0.5,
            "shape.leaflet_width",
        )?;
        check(
            (0.0..1.0).contains(&self.leaflet_span),
            "shape.leaflet_span",
        )?;
        // The swept margin must still advance from station to station, or
        // the blade strip would fold over itself.
        let ts = self.station_ts();
        let margin = |t: f32| {
            self.skewed(glam::Vec2::new(self.half_width(t), t * self.length))
                .y
        };
        check(
            ts.windows(2).all(|w| margin(w[1]) > margin(w[0])),
            "shape.lobe_skew",
        )
    }
}

impl FoliageParams {
    /// Checks every parameter range.
    ///
    /// # Errors
    ///
    /// [`FoliageError::Params`] naming the first parameter out of range.
    pub fn validate(&self) -> Result<(), FoliageError> {
        self.shape.validate()?;
        check((1..=256).contains(&self.variants), "variants")?;
        let v = self.variation;
        for (value, name) in [
            (v.size, "variation.size"),
            (v.width, "variation.width"),
            (v.lobe_depth, "variation.lobe_depth"),
        ] {
            check((0.0..1.0).contains(&value), name)?;
        }
        check(self.droop.is_finite(), "droop")?;
        check((0.0..=1.0).contains(&self.light), "light")?;
        check(self.roll_jitter.is_finite(), "roll_jitter")?;
        check((1..=16).contains(&self.whorl), "whorl")
    }
}
