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
//! # Contract
//!
//! This is a cross-repository contract, shared bit-for-bit with dapple and
//! intended to move into `exedra_math`. Do not change it without changing
//! every implementation and its golden vectors.
//!
//! ```text
//! splitmix64_mix(z) = z ^= z >> 30; z *= 0xBF58476D1CE4E5B9;
//!                     z ^= z >> 27; z *= 0x94D049BB133111EB;
//!                     z ^ (z >> 31)                      (wrapping u64 arithmetic)
//! mix(h, k)         = splitmix64_mix(h ^ (k * 0x9E3779B97F4A7C15))
//! hash(seed, keys)  = keys.fold(seed, mix)               (so hash(seed, []) = seed)
//! unit_f32(h)       = (h >> 40) as f32 * 2^-24           (in [0, 1))
//! unit_f64(h)       = (h >> 11) as f64 * 2^-53           (in [0, 1))
//! ```
//!
//! Golden vectors:
//!
//! ```text
//! hash(0, [])                  = 0x0000000000000000
//! hash(1, [2, 3])              = 0x614aeb9ed12ccf8d
//! hash(u64::MAX, [0])          = 0xb4d055fcf2cbbd7b
//! unit_f32(0x614aeb9ed12ccf8d) = 0.38004941   (bits 0x3ec295d6)
//! unit_f64(0x614aeb9ed12ccf8d) = 0.3800494444596324
//! unit_f32(0xb4d055fcf2cbbd7b) = 0.70630390   (bits 0x3f34d055)
//! unit_f64(0xb4d055fcf2cbbd7b) = 0.7063039534139496
//! unit_f32(u64::MAX)           = 0.99999994   (bits 0x3f7fffff)
//! unit_f64(u64::MAX)           = 0.9999999999999999
//! ```
//!
//! # Example
//! ```rust
//! use sylva_skeleton::keyed::{Key, hash, tag};
//!
//! assert_eq!(hash(0, &[]), 0);
//! let angle = Key::new(7).with(tag("branch.angle")).with(3).unit_f32();
//! assert!((0.0..1.0).contains(&angle));
//! ```

/// SplitMix64's output finalizer.
#[must_use]
pub const fn splitmix64_mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Folds one key into a hash state.
#[must_use]
pub const fn mix(h: u64, k: u64) -> u64 {
    splitmix64_mix(h ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Folds `keys` left into `seed` with [`mix`].
#[must_use]
pub const fn hash(seed: u64, keys: &[u64]) -> u64 {
    let mut h = seed;
    let mut i = 0;
    while i < keys.len() {
        h = mix(h, keys[i]);
        i += 1;
    }
    h
}

/// Maps a hash to `[0, 1)` using its top 24 bits.
///
/// The shifted value has 24 bits, so the conversion to `f32` is exact.
#[must_use]
pub const fn unit_f32(h: u64) -> f32 {
    (h >> 40) as u32 as f32 * (1.0 / 16_777_216.0)
}

/// Maps a hash to `[0, 1)` using its top 53 bits.
///
/// The shifted value has 53 bits, so the conversion to `f64` is exact.
#[must_use]
pub const fn unit_f64(h: u64) -> f64 {
    (h >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0)
}

/// A purpose tag from a name: 64-bit FNV-1a over its UTF-8 bytes.
///
/// Tags make keys readable (`tag("branch.angle")`) while staying plain `u64`
/// keys. This helper is a sylva convenience, not part of the shared contract;
/// hashes built from tags are only as portable as the tag names.
#[must_use]
pub const fn tag(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    h
}

/// A hash state that accumulates keys, then yields values.
///
/// `Key::new(seed).with(a).with(b)` equals `hash(seed, &[a, b])`.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct Key(u64);

impl Key {
    /// Starts from a seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Folds in one more key.
    #[must_use]
    pub const fn with(self, key: u64) -> Self {
        Self(mix(self.0, key))
    }

    /// The raw 64-bit hash.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// A value in `[0, 1)`.
    #[must_use]
    pub const fn unit_f32(self) -> f32 {
        unit_f32(self.0)
    }

    /// A value in `[0, 1)`.
    #[must_use]
    pub const fn unit_f64(self) -> f64 {
        unit_f64(self.0)
    }

    /// A value in `[lo, hi)` (or `hi` itself when rounding lands there).
    #[must_use]
    pub fn range_f32(self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit_f32()
    }

    /// A value in `[-1, 1)`. Exact: `unit_f32` has 24 significant bits.
    #[must_use]
    pub fn signed_unit_f32(self) -> f32 {
        self.unit_f32() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Key, hash, mix, splitmix64_mix, tag, unit_f32, unit_f64};

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
