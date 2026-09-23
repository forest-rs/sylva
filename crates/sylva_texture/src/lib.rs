// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tree texture recipes for sylva, built on dapple.
//!
//! Species texture recipes turn a few botanical parameters into OpenPBR
//! material maps through dapple's field programs and raster operations:
//!
//! - [`bark()`] makes a tileable bark set ([`BarkRecipe`]) whose tile matches
//!   the world size of `sylva_mesh`'s bark UVs;
//! - [`leaf()`] makes a leaf set ([`LeafRecipe`]) in the UV frame of a
//!   `sylva_foliage` leaf shape: opacity is the leaf's own coverage mask
//!   ([`leaf_mask`], rasterized from its outline by `dapple_imaging`), and
//!   veins follow its lobes.
//!
//! Every map of a set derives from one height or one outline, so the maps
//! agree. Sets are [`MaterialMaps`](dapple_encode::MaterialMaps): pack them
//! with [`dapple_encode::pack`] for a consumer, which builds mip chains that
//! preserve alpha-tested coverage and fold normal variance into roughness.
//! Output is deterministic: equal recipes give equal bytes on every platform.
//!
//! # Example
//! ```rust
//! use dapple_encode::{PackSettings, Profile, pack};
//! use sylva_texture::{BarkRecipe, bark};
//!
//! let set = bark(&BarkRecipe { size: 64, ..BarkRecipe::default() })?;
//! let bundle = pack(&set.maps, Profile::Gltf, &PackSettings::default())?;
//! assert!(bundle.texture("base_color").is_some());
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

#![no_std]

extern crate alloc;

mod bark;
mod error;
mod leaf;

pub use bark::{BarkRecipe, BarkSet, bark};
pub use error::TextureError;
pub use leaf::{LeafRecipe, LeafSet, leaf, leaf_mask};

#[cfg(test)]
mod tests;
