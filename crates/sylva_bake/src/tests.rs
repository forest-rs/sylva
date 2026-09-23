// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::vec;

use dapple_encode::Image;
use dapple_raster::Edge;
use glam::{Affine3A, Vec2, Vec3};

use crate::{BakeError, BakeMaterial, BakeMesh, BakeSettings, CardView, bake};

const QUAD: [[f32; 3]; 4] = [
    [-1.0, -1.0, 0.0],
    [1.0, -1.0, 0.0],
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
];
const UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
const UP: [[f32; 3]; 4] = [[0.0, 0.0, 1.0]; 4];
const INDICES: [u32; 6] = [0, 1, 2, 0, 2, 3];

fn quad<'a>(transform: Affine3A, material: BakeMaterial<'a>) -> BakeMesh<'a> {
    BakeMesh {
        positions: &QUAD,
        normals: &UP,
        uvs: &UVS,
        indices: &INDICES,
        transform,
        material,
    }
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

fn view(half: f32) -> CardView {
    CardView::facing(Vec3::ZERO, Vec3::X, Vec3::Y, Vec2::splat(half), 2.0)
}

const SETTINGS: BakeSettings = BakeSettings {
    size: [32, 32],
    samples: 4,
};

#[test]
fn a_half_card_quad_covers_the_middle_quarter_exactly() {
    let baked = bake(
        &[quad(
            Affine3A::from_scale(Vec3::splat(0.5)),
            BakeMaterial::default(),
        )],
        &view(1.0),
        &SETTINGS,
    )
    .expect("bake");
    // The quad spans [-0.5, 0.5]: the middle 16 x 16 texels.
    assert_eq!(baked.report.covered_texels, 16 * 16);
    let opacity = baked.opacity.values();
    assert_eq!(opacity[16 * 32 + 16], 1.0);
    assert_eq!(opacity[0], 0.0);
    let n = baked.normal.texel(16, 16);
    assert_eq!(n, [0.0, 0.0, 1.0], "facing the viewer");
}

#[test]
fn nearer_fragments_win_and_back_faces_flip() {
    let red = BakeMaterial {
        color: [1.0, 0.0, 0.0],
        ..BakeMaterial::default()
    };
    let blue = BakeMaterial {
        color: [0.0, 0.0, 1.0],
        ..BakeMaterial::default()
    };
    // Blue is nearer (+Z toward the viewer) and flipped to face away.
    let near = Affine3A::from_translation(Vec3::new(0.0, 0.0, 0.5))
        * Affine3A::from_rotation_y(core::f32::consts::PI);
    let baked = bake(
        &[quad(Affine3A::IDENTITY, red), quad(near, blue)],
        &view(1.0),
        &SETTINGS,
    )
    .expect("bake");
    assert!(close(baked.base_color.texel(10, 10), &[0.0, 0.0, 1.0]));
    assert!(
        close(baked.normal.texel(10, 10), &[0.0, 0.0, 1.0]),
        "two-sided"
    );
    let depth = baked.depth.texel(10, 10)[0];
    assert!((depth - (0.5 + 0.5 * 0.5 / 2.0)).abs() < 1e-6, "{depth}");
    // Order does not matter.
    let swapped = bake(
        &[quad(near, blue), quad(Affine3A::IDENTITY, red)],
        &view(1.0),
        &SETTINGS,
    )
    .expect("swapped");
    assert_eq!(swapped.base_color.values(), baked.base_color.values());
}

#[test]
fn alpha_tested_textures_cut_the_coverage() {
    // A 2 x 2 opacity texture: only the top-right texel is opaque.
    let opacity = Image::new(2, 2, 1, Edge::Clamp, vec![0.0, 0.0, 0.0, 1.0]).expect("image");
    let color = Image::new(2, 2, 3, Edge::Clamp, vec![0.2; 12]).expect("image");
    let material = BakeMaterial {
        base_color: Some(&color),
        opacity: Some(&opacity),
        ..BakeMaterial::default()
    };
    let baked = bake(&[quad(Affine3A::IDENTITY, material)], &view(1.0), &SETTINGS).expect("bake");
    assert_eq!(baked.report.covered_texels, 16 * 16, "one quarter survives");
    assert!(baked.report.alpha_discards > 0);
    // Row 0 is v = 0, the card's bottom: the top-right quarter is rows 16+.
    assert_eq!(baked.opacity.texel(24, 24), [1.0]);
    assert_eq!(baked.opacity.texel(24, 8), [0.0]);
    assert!(close(baked.base_color.texel(24, 24), &[0.2; 3]));
}

#[test]
fn bakes_are_deterministic_and_input_is_validated() {
    let mesh = [quad(
        Affine3A::from_rotation_x(0.4),
        BakeMaterial::default(),
    )];
    let a = bake(&mesh, &view(1.2), &SETTINGS).expect("a");
    let b = bake(&mesh, &view(1.2), &SETTINGS).expect("b");
    assert_eq!(a.base_color.values(), b.base_color.values());
    assert_eq!(a.normal.values(), b.normal.values());
    assert_eq!(a.depth.values(), b.depth.values());
    let skewed = CardView {
        up: Vec3::new(0.3, 1.0, 0.0),
        ..view(1.0)
    };
    assert_eq!(bake(&mesh, &skewed, &SETTINGS).err(), Some(BakeError::View));
    let broken = BakeMesh {
        indices: &[0, 1, 9],
        ..mesh[0]
    };
    assert_eq!(
        bake(&[broken], &view(1.0), &SETTINGS).err(),
        Some(BakeError::Mesh { mesh: 0 })
    );
}
