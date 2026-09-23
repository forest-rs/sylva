// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use dapple_encode::{PackSettings, Profile, pack};
use dapple_raster::Edge;
use sylva_foliage::LeafShape;

use crate::{BarkRecipe, LeafRecipe, TextureError, bark, leaf, leaf_mask};

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
    let mask = leaf_mask(&shape, 64).expect("mask");
    let opacity = set.maps.opacity.as_ref().expect("opacity").values();
    assert_eq!(opacity, mask.values());
    // Veins are lighter than the blade along the midrib.
    let color = set.maps.base_color.as_ref().unwrap().values();
    let at = |col: usize, row: usize| color[(row * 64 + col) * 3 + 1];
    assert!(
        at(32, 32) > at(20, 32),
        "midrib is lighter than the blade beside it"
    );
    // Thin-walled translucency: the blade transmits, veins half as much.
    let weight = set
        .maps
        .subsurface_weight
        .as_ref()
        .expect("weight")
        .values();
    assert!(
        weight
            .iter()
            .all(|w| (0.0..=recipe.translucency).contains(w))
    );
    assert!((weight[32 * 64 + 20] - recipe.translucency).abs() < 0.05);
    assert!(
        weight[32 * 64 + 32] < weight[32 * 64 + 20],
        "the midrib transmits less"
    );
    assert_eq!(
        set.maps.subsurface_color.as_ref().expect("tint").channels(),
        3
    );
    // glTF packs it as one diffuse-transmission texture.
    let gltf = pack(&set.maps, Profile::Gltf, &PackSettings::default()).expect("pack");
    assert!(gltf.texture("diffuse_transmission").is_some());
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
    assert_eq!(
        leaf(&LeafRecipe {
            translucency: 1.5,
            ..LeafRecipe::default()
        })
        .err(),
        Some(TextureError::Params {
            name: "translucency"
        })
    );
}

/// The area enclosed by a closed polygon.
fn polygon_area(outline: &[glam::Vec2]) -> f32 {
    0.5 * outline
        .iter()
        .zip(outline.iter().cycle().skip(1))
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f32>()
}

#[test]
fn the_mask_covers_the_outline_area() {
    let shape = LeafShape::default();
    let frame_area = 2.0 * shape.max_half_width() * shape.length;
    let expected = polygon_area(&shape.outline_at(1024)) / frame_area;
    let mask = leaf_mask(&shape, 128).expect("mask");
    assert_eq!((mask.width(), mask.height()), (128, 128));
    let covered = mask.values().iter().sum::<f32>() / (128.0 * 128.0);
    assert!((covered - expected).abs() < 0.01, "{covered} vs {expected}");
    assert!(mask.values().iter().all(|v| (0.0..=1.0).contains(v)));
    // The base row sits on the blade base; the midrib column is covered
    // along the blade.
    assert!(mask.values()[64 * 128 + 64] > 0.99);
    assert_eq!(
        mask.values(),
        leaf_mask(&shape, 128).expect("mask").values(),
        "deterministic"
    );
    assert!(matches!(
        leaf_mask(&shape, 0),
        Err(TextureError::Params { name: "size" })
    ));
}
