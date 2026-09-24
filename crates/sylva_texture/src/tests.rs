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
    // Off the veins, the blade transmits nearly the full weight.
    assert!(weight[32 * 64 + 20] > 0.9 * recipe.translucency);
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

#[test]
fn bark_modules_follow_the_stem() {
    use dapple_library::modules::Birch;

    // Birch's base darkens and fissures with girth; the same module at two
    // girths gives two barks, each with colour, normals and roughness.
    let young = crate::bark_module(&Birch, 0.3, 1.3, 32).expect("young birch");
    let old = crate::bark_module(&Birch, 2.5, 0.3, 32).expect("old birch base");
    for set in [&young, &old] {
        assert!(set.maps.base_color.is_some() && set.maps.normal.is_some());
        assert!(set.fingerprint.is_none());
    }
    let mean = |set: &crate::BarkSet| {
        let values = set.maps.base_color.as_ref().expect("colour").values();
        values.iter().sum::<f32>() / values.len() as f32
    };
    assert!(mean(&young) > mean(&old), "the old base is darker");
    assert!(matches!(
        crate::bark_module(&Birch, 1.0, 1.3, 0),
        Err(TextureError::Params { name: "size" })
    ));
}

#[test]
fn bark_stages_blend_linearly_between_bracketing_heights() {
    use crate::{BarkStage, bark_stage_blend, bark_stages};
    use dapple_library::modules::Birch;

    let heights = [0.3, 1.3, 6.0];
    assert_eq!(bark_stage_blend(&heights, 0.0), (0, 0, 0.0));
    assert_eq!(bark_stage_blend(&heights, 9.0), (2, 2, 0.0));
    let (lo, hi, t) = bark_stage_blend(&heights, 0.8);
    assert_eq!((lo, hi), (0, 1));
    assert!((t - 0.5).abs() < 1e-6);
    let (lo, hi, t) = bark_stage_blend(&heights, 1.3);
    assert!((lo, hi) == (0, 1) && (t - 1.0).abs() < 1e-6 || (lo, hi, t) == (1, 2, 0.0));

    let stages = [
        BarkStage {
            height: 0.3,
            girth: 1.2,
        },
        BarkStage {
            height: 6.0,
            girth: 0.6,
        },
    ];
    let sets = bark_stages(&Birch, &stages, 32).expect("stages");
    assert_eq!(sets.len(), 2);
    let mean = |set: &crate::BarkSet| {
        let v = set.maps.base_color.as_ref().expect("colour").values();
        v.iter().sum::<f32>() / v.len() as f32
    };
    assert!(mean(&sets[1]) > mean(&sets[0]), "birch whitens up the stem");
    assert!(matches!(
        bark_stages(&Birch, &[stages[1], stages[0]], 32),
        Err(TextureError::Params { name: "stages" })
    ));
}

#[test]
fn pine_bark_turns_orange_up_the_stem() {
    use crate::{BarkStage, bark_stages};
    use dapple_library::modules::ScotsPine;

    let sets = bark_stages(
        &ScotsPine,
        &[
            BarkStage {
                height: 1.3,
                girth: 1.4,
            },
            BarkStage {
                height: 14.0,
                girth: 0.5,
            },
        ],
        32,
    )
    .expect("pine stages");
    // Red over blue: grey plates low, orange flakes high.
    let warmth = |set: &crate::BarkSet| {
        let v = set.maps.base_color.as_ref().expect("colour").values();
        let (mut r, mut b) = (0.0, 0.0);
        for texel in v.chunks(3) {
            r += texel[0];
            b += texel[2];
        }
        r / b.max(1e-6)
    };
    assert!(
        warmth(&sets[1]) > warmth(&sets[0]) * 1.2,
        "{} vs {}",
        warmth(&sets[1]),
        warmth(&sets[0])
    );
}
