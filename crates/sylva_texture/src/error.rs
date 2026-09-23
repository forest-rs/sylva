// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Texture recipe failures.

use alloc::string::String;
use core::fmt;

use dapple_encode::EncodeError;
use dapple_field::program::ProgramError;
use dapple_raster::RasterError;

/// Why a texture recipe failed.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TextureError {
    /// A recipe parameter is out of range.
    Params {
        /// The offending parameter.
        name: &'static str,
    },
    /// Dapple rejected the field program.
    Program(ProgramError),
    /// Dapple failed to realize or process a raster.
    Raster(RasterError),
    /// Dapple rejected an image.
    Encode(EncodeError),
    /// `dapple_imaging` could not rasterize a mask; the message is its
    /// error's.
    Mask(String),
}

impl fmt::Display for TextureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Params { name } => write!(f, "texture parameter {name} is out of range"),
            Self::Program(error) => write!(f, "field program: {error}"),
            Self::Raster(error) => write!(f, "raster: {error}"),
            Self::Encode(error) => write!(f, "image: {error}"),
            Self::Mask(error) => write!(f, "mask: {error}"),
        }
    }
}

impl core::error::Error for TextureError {}

impl From<ProgramError> for TextureError {
    fn from(error: ProgramError) -> Self {
        Self::Program(error)
    }
}

impl From<RasterError> for TextureError {
    fn from(error: RasterError) -> Self {
        Self::Raster(error)
    }
}
