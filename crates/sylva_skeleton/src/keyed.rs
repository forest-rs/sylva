// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Keyed, counter-based randomness.
//!
//! Sylva never draws from a sequential random stream shared across a tree.
//! Every decision hashes a seed together with the keys that identify it — the
//! branch, the purpose, an ordinal — so changing one decision cannot reshuffle
//! unrelated ones. This is what makes incremental regeneration and art
//! direction stable.
//!
//! The hash is `exedra_math::keyed`, version 1 of the cross-repository keyed
//! hash contract (`exedra_math` ADR-0001), shared bit for bit with dapple.
//! This module re-exports it and adds [`SignedUnit`], a sylva convenience
//! outside that contract. The golden vectors below are checked here too, so
//! a dependency bump that changed the contract would fail sylva's tests.
//!
//! ```text
//! hash(0, [])                  = 0x0000000000000000
//! hash(1, [2, 3])              = 0x614aeb9ed12ccf8d
//! hash(u64::MAX, [0])          = 0xb4d055fcf2cbbd7b
//! unit_f32(0x614aeb9ed12ccf8d) = 0.38004941   (bits 0x3ec295d6)
//! unit_f64(0x614aeb9ed12ccf8d) = 0.3800494444596324
//! tag("")                      = 0xcbf29ce484222325
//! tag("a")                     = 0xaf63dc4c8601ec8c
//! ```
//!
//! # Example
//! ```rust
//! use sylva_skeleton::keyed::{Key, SignedUnit, hash, tag};
//!
//! assert_eq!(hash(0, &[]), 0);
//! let angle = Key::new(7).with(tag("branch.angle")).with(3).unit_f32();
//! assert!((0.0..1.0).contains(&angle));
//! let jitter = Key::new(7).with(tag("branch.roll")).signed_unit_f32();
//! assert!((-1.0..1.0).contains(&jitter));
//! ```

pub use exedra_math::keyed::{Key, hash, mix, splitmix64_mix, tag, unit_f32, unit_f64};

/// Signed unit values from a [`Key`]: a sylva convenience, not part of the
/// keyed hash contract.
pub trait SignedUnit {
    /// A value in `[-1, 1)`: `2 * unit_f32 - 1`, exact because `unit_f32`
    /// has 24 significant bits.
    fn signed_unit_f32(self) -> f32;
}

impl SignedUnit for Key {
    fn signed_unit_f32(self) -> f32 {
        self.unit_f32() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Key, SignedUnit, hash, mix, splitmix64_mix, tag, unit_f32, unit_f64};

    #[test]
    fn golden_vectors_match_the_documented_contract() {
        assert_eq!(hash(0, &[]), 0);
        assert_eq!(hash(1, &[2, 3]), 0x614a_eb9e_d12c_cf8d);
        assert_eq!(hash(u64::MAX, &[0]), 0xb4d0_55fc_f2cb_bd7b);
        assert_eq!(unit_f32(0x614a_eb9e_d12c_cf8d).to_bits(), 0x3ec2_95d6);
        assert_eq!(unit_f64(0x614a_eb9e_d12c_cf8d), 0.380_049_444_459_632_4);
        assert_eq!(unit_f32(0xb4d0_55fc_f2cb_bd7b).to_bits(), 0x3f34_d055);
        assert_eq!(unit_f64(0xb4d0_55fc_f2cb_bd7b), 0.706_303_953_413_949_6);
        assert_eq!(unit_f32(u64::MAX).to_bits(), 0x3f7f_ffff);
        assert_eq!(unit_f64(u64::MAX), 0.999_999_999_999_999_9);
        assert_eq!(unit_f32(0), 0.0);
    }

    #[test]
    fn hash_is_a_left_fold_of_mix() {
        assert_eq!(hash(5, &[1, 2]), mix(mix(5, 1), 2));
        assert_eq!(mix(0, 0), splitmix64_mix(0));
        assert_eq!(Key::new(1).with(2).with(3).bits(), hash(1, &[2, 3]));
        assert_ne!(hash(1, &[2, 3]), hash(1, &[3, 2]), "key order matters");
    }

    #[test]
    fn tags_are_fnv1a() {
        assert_eq!(tag(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(tag("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn ranges_stay_in_bounds() {
        for i in 0..1000 {
            let k = Key::new(42).with(i);
            let v = k.range_f32(-2.0, 3.0);
            assert!((-2.0..=3.0).contains(&v), "value {v} out of range");
            let s = k.signed_unit_f32();
            assert!((-1.0..1.0).contains(&s), "signed value {s} out of range");
        }
    }
}
