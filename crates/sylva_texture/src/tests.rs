// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;

use dapple_encode::{PackSettings, Profile, pack};
use dapple_field::program::Op;
use dapple_field::{CellOutput, Domain};
use dapple_graph::{RasterParams, Recipe, RecipeNode, RecipeOutput, Step};
use dapple_raster::Edge;
use dapple_raster::HeightToNormal;
use sylva_foliage::LeafShape;

use crate::{LeafRecipe, TextureError, bark, leaf, leaf_mask};

/// A small bark recipe: cellular fissures realized on a 32-texel tile, grey
/// scale colour, and normals from the same height.
fn small_bark(seed: u64) -> Recipe {
    let torus = Domain::periodic(1, 1).expect("domain");
    let node = |label: &str, step| RecipeNode {
        label: label.into(),
        step,
    };
    Recipe {
        nodes: vec![
            node(
                "height",
                Step::Field {
                    op: Op::Cellular {
                        domain: torus,
                        frequency: [7.0, 2.0],
                        jitter: 1.0,
                        seed,
                        output: CellOutput::Border,
                    },
                    inputs: vec![],
                },
            ),
            node(
                "height_map",
                Step::Realize {
                    input: "height".into(),
                    width: 32,
                    height: 32,
                },
            ),
            node(
                "normal_map",
                Step::Raster {
                    input: "height_map".into(),
                    params: RasterParams::HeightToNormal(HeightToNormal { scale: 0.04 }),
                },
            ),
        ],
        outputs: vec![
            RecipeOutput {
                role: "base_color".into(),
                channels: vec![
                    "height_map".into(),
                    "height_map".into(),
                    "height_map".into(),
                ],
            },
            RecipeOutput {
                role: "normal".into(),
                channels: vec!["normal_map".into()],
            },
        ],
        ..Recipe::default()
    }
}

#[test]
fn bark_runs_a_dapple_recipe_into_wrapping_maps() {
    let a = bark(&small_bark(1)).expect("bark");
    let b = bark(&small_bark(1)).expect("again");
    assert_eq!(a.fingerprint, b.fingerprint);
    let color = a.maps.base_color.as_ref().expect("colour");
    let normal = a.maps.normal.as_ref().expect("normal");
    assert_eq!(color.values(), b.maps.base_color.as_ref().unwrap().values());
    assert_eq!((color.width(), color.channels()), (32, 3));
    assert_eq!(normal.channels(), 3);
    for image in [color, normal] {
        assert_eq!(image.edge(), Edge::Wrap, "one repeating tile");
    }
    // Channels interleave per texel: all three came from one height.
    assert!(
        color
            .values()
            .chunks(3)
            .all(|t| t[0] == t[1] && t[1] == t[2])
    );
    assert_ne!(
        bark(&small_bark(2)).expect("seed 2").fingerprint,
        a.fingerprint,
        "the seed is content"
    );
    let bundle = pack(&a.maps, Profile::Gltf, &PackSettings::default()).expect("pack");
    assert!(bundle.texture("normal").is_some());
}

#[test]
fn bark_refuses_outputs_that_do_not_fit() {
    let mut unknown = small_bark(1);
    unknown.outputs[1].role = "sparkle".into();
    assert_eq!(
        bark(&unknown).err(),
        Some(TextureError::Output {
            role: "sparkle".into()
        })
    );
    let mut short = small_bark(1);
    short.outputs[0].channels.pop();
    assert_eq!(
        bark(&short).err(),
        Some(TextureError::Output {
            role: "base_color".into()
        })
    );
    let mut colourless = small_bark(1);
    colourless.outputs.remove(0);
    assert_eq!(
        bark(&colourless).err(),
        Some(TextureError::Output {
            role: "base_color".into()
        })
    );
    let mut dangling = small_bark(1);
    dangling.nodes[1] = RecipeNode {
        label: "height_map".into(),
        step: Step::Realize {
            input: "missing".into(),
            width: 32,
            height: 32,
        },
    };
    assert!(matches!(bark(&dangling), Err(TextureError::Recipe(_))));
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
