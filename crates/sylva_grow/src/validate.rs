// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Parameter checks run before growth.

use crate::{Arrangement, Count, GrowError, Hierarchy, Shape};

fn invalid(level: Option<usize>, name: &'static str) -> GrowError {
    GrowError::InvalidParameter { level, name }
}

fn finite(value: f32) -> bool {
    value.is_finite()
}

fn positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

fn fraction(value: f32) -> bool {
    (0.0..1.0).contains(&value)
}

fn span(span: [f32; 2]) -> bool {
    let [lo, hi] = span;
    (0.0..=1.0).contains(&lo) && (0.0..=1.0).contains(&hi) && lo <= hi
}

fn shape(level: Option<usize>, shape: &Shape) -> Result<(), GrowError> {
    let fields = [
        (shape.curve, "shape.curve"),
        (shape.curve_back, "shape.curve_back"),
        (shape.gnarl, "shape.gnarl"),
        (shape.up, "shape.up"),
        (shape.sag, "shape.sag"),
        (shape.light, "shape.light"),
        (shape.kink, "shape.kink"),
    ];
    for (value, name) in fields {
        if !finite(value) {
            return Err(invalid(level, name));
        }
    }
    if shape.gnarl != 0.0 && !positive(shape.gnarl_wavelength) {
        return Err(invalid(level, "shape.gnarl_wavelength"));
    }
    if shape.kink != 0.0 && !positive(shape.kink_interval) {
        return Err(invalid(level, "shape.kink_interval"));
    }
    if !(0.0..=1.0).contains(&shape.kink_jitter) {
        return Err(invalid(level, "shape.kink_jitter"));
    }
    Ok(())
}

pub(crate) fn check(h: &Hierarchy) -> Result<(), GrowError> {
    if !positive(h.segment_length) {
        return Err(invalid(None, "segment_length"));
    }
    if !positive(h.radii.tip_radius) {
        return Err(invalid(None, "radii.tip_radius"));
    }
    if !(h.radii.exponent.is_finite() && h.radii.exponent >= 1.0) {
        return Err(invalid(None, "radii.exponent"));
    }
    let trunk = &h.trunk;
    if !positive(trunk.length) {
        return Err(invalid(None, "trunk.length"));
    }
    if !fraction(trunk.length_jitter) {
        return Err(invalid(None, "trunk.length_jitter"));
    }
    if !finite(trunk.lean) {
        return Err(invalid(None, "trunk.lean"));
    }
    shape(None, &trunk.shape)?;
    for (index, level) in h.levels.iter().enumerate() {
        let at = Some(index);
        match level.count {
            Count::Fixed(_) => {}
            Count::PerMetre(density) => {
                if !(density.is_finite() && density >= 0.0) {
                    return Err(invalid(at, "count"));
                }
            }
        }
        if let Arrangement::Whorled { per_node: 0 } = level.arrangement {
            return Err(invalid(at, "arrangement.per_node"));
        }
        if let Arrangement::Spiral { divergence } = level.arrangement
            && !finite(divergence)
        {
            return Err(invalid(at, "arrangement.divergence"));
        }
        if !span(level.span) {
            return Err(invalid(at, "span"));
        }
        let jitters = [
            (level.roll_jitter, "roll_jitter"),
            (level.angle_jitter, "angle_jitter"),
        ];
        for (value, name) in jitters {
            if !finite(value) {
                return Err(invalid(at, name));
            }
        }
        if !(0.0..=1.0).contains(&level.position_jitter) {
            return Err(invalid(at, "position_jitter"));
        }
        if !fraction(level.length_jitter) {
            return Err(invalid(at, "length_jitter"));
        }
        if !(0.0..=1.0).contains(&level.balance) {
            return Err(invalid(at, "balance"));
        }
        if level.length.points().iter().any(|p| p[1] < 0.0) {
            return Err(invalid(at, "length"));
        }
        shape(at, &level.shape)?;
        if let Some(sites) = level.sites
            && !(sites.per_metre.is_finite() && sites.per_metre >= 0.0 && span(sites.span))
        {
            return Err(invalid(at, "sites"));
        }
    }
    if let Some(e) = &h.envelope {
        if !finite(e.base) {
            return Err(invalid(None, "envelope.base"));
        }
        if !positive(e.height) {
            return Err(invalid(None, "envelope.height"));
        }
        if !positive(e.radius) {
            return Err(invalid(None, "envelope.radius"));
        }
        if !(0.0..=1.0).contains(&e.min_fraction) {
            return Err(invalid(None, "envelope.min_fraction"));
        }
    }
    Ok(())
}
