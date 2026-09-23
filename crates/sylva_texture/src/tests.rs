// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dapple_encode::{PackSettings, Profile, pack};
use dapple_raster::Edge;
use sylva_foliage::{LeafShape, leaf_mask};

use crate::{BarkRecipe, LeafRecipe, TextureError, bark, leaf};

fn small_bark() -> BarkRecipe {
    BarkRecipe {
        size: 64,
        ..BarkRecipe::default()
    }
}

#[test]
fn bark_is_deterministic_tileable_and_complete() {
    let a = bark(&small_bark()).expect("bark");
    let b = bark(&small_bark()).expect("again");
    assert_eq!(a.fingerprint, b.fingerprint);
    assert_eq!(a.height.digest(), b.height.digest());
    assert_eq!(a.height.edge(), Edge::Wrap, "one repeating tile");
    let maps = &a.maps;
    for image in [
        &maps.base_color,
        &maps.normal,
        &maps.specular_roughness,
        &maps.occlusion,
    ] {
        let image = image.as_ref().expect("map present");
        assert_eq!(image.edge(), Edge::Wrap, "every map wraps");
    }
    // Fissures are darker and rougher than plate tops.
    let heights = a.height.values();
    let (lo, hi) = heights
        .iter()
        .enumerate()
        .fold((0, 0), |(lo, hi), (i, &h)| {
            (
                if h < heights[lo] { i } else { lo },
                if h > heights[hi] { i } else { hi },
            )
        });
    let color = maps.base_color.as_ref().unwrap().values();
    let rough = maps.specular_roughness.as_ref().unwrap().values();
    assert!(color[lo * 3] < color[hi * 3], "fissures are darker");
    assert!(rough[lo] > rough[hi], "fissures are rougher");
    let other = bark(&BarkRecipe {
        seed: 2,
        ..small_bark()
    })
    .expect("seed 2");
    assert_ne!(other.fingerprint, a.fingerprint, "the seed is content");
    let bundle = pack(&a.maps, Profile::Gltf, &PackSettings::default()).expect("pack");
    assert!(bundle.texture("normal").is_some());
}

#[test]
fn leaf_opacity_is_the_leaf_shapes_own_mask() {
    let shape = LeafShape::default();
    let recipe = LeafRecipe {
        shape,
        size: 64,
        ..LeafRecipe::default()
    };
    let set = leaf(&recipe).expect("leaf");
    let mask = leaf_mask(&shape, 64);
    let opacity = set.maps.opacity.as_ref().expect("opacity").values();
    for (o, &c) in opacity.iter().zip(&mask.coverage) {
        assert!((o * 255.0 - f32::from(c)).abs() < 1e-3);
    }
    // Veins are lighter than the blade along the midrib.
    let color = set.maps.base_color.as_ref().unwrap().values();
    let at = |col: usize, row: usize| color[(row * 64 + col) * 3 + 1];
    assert!(
        at(32, 32) > at(20, 32),
        "midrib is lighter than the blade beside it"
    );
    assert_eq!(set.translucency.channels(), 3);
    assert_eq!(leaf(&recipe).expect("again").fingerprint, set.fingerprint);
    let bundle = pack(
        &set.maps,
        Profile::Lightweald,
        &PackSettings {
            alpha_cutoff: Some(0.5),
            ..PackSettings::default()
        },
    )
    .expect("pack");
    assert!(bundle.texture("base_color").is_some());
}

#[test]
fn invalid_recipes_are_refused() {
    assert_eq!(
        bark(&BarkRecipe {
            fissure: 0.6,
            ..small_bark()
        })
        .err(),
        Some(TextureError::Params { name: "fissure" })
    );
    assert_eq!(
        leaf(&LeafRecipe {
            size: 4,
            ..LeafRecipe::default()
        })
        .err(),
        Some(TextureError::Params { name: "size" })
    );
}
