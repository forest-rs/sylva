// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tree texture recipes for sylva, built on dapple.
//!
//! Species texture recipes turn a few botanical parameters into OpenPBR
//! material maps through dapple's field programs and raster operations:
//!
//! - [`bark()`] runs a dapple bark [`Recipe`](dapple_graph::Recipe), the same
//!   data dapple bakes, into a tileable set whose tile matches the world size
//!   of `sylva_mesh`'s bark UVs, and [`bark_module`] runs one of dapple's
//!   calibrated bark modules at a stem's girth and height;
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
//! use sylva_texture::{LeafRecipe, leaf};
//!
//! let set = leaf(&LeafRecipe { size: 64, ..LeafRecipe::default() })?;
//! let bundle = pack(&set.maps, Profile::Gltf, &PackSettings::default())?;
//! assert!(bundle.texture("base_color").is_some());
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

#![no_std]

extern crate alloc;

mod bark;
mod error;
mod leaf;

pub use bark::{BarkSet, bark, bark_module};
pub use error::TextureError;
pub use leaf::{LeafRecipe, LeafSet, leaf, leaf_mask};

#[cfg(test)]
mod tests;
