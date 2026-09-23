// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Bakes a twig-cluster card: a branchlet's bark and leaves, with all its
//! twigs, rendered onto a quad, the stand-in real-time trees use for distant
//! foliage.

use std::path::Path;

use dapple_encode::{Filter, Image, PackSettings, Profile, pack};
use exedra_mesh::{AttributeBuffer, TriMesh};
use sylva_bake::glam::{Affine3A, Vec2, Vec3};
use sylva_bake::{BakeMaterial, BakeMesh, BakeSettings, CardView, bake};
use sylva_foliage::Foliage;
use sylva_mesh::BRANCH_LAYER;
use sylva_skeleton::Skeleton;

/// Colour textures the card samples.
pub(crate) struct Textures {
    /// The bark set's linear base colour.
    pub(crate) bark: Image,
    /// The leaf set's linear base colour, when the species has foliage.
    pub(crate) leaf: Option<Image>,
}

/// Bakes the order-2 branchlet whose subtree carries the most leaves into
/// `dir` and returns a stats fragment for `stats.json`.
///
/// The card's `up` runs along the branchlet; it faces its leaves' mean upper
/// surface. Textures are packed for glTF (PNG and KTX2 with coverage-
/// preserving mips), with the depth map beside them.
pub(crate) fn bake_twig_card(
    dir: &Path,
    skeleton: &Skeleton,
    bark: &TriMesh,
    foliage: &Foliage,
    textures: &Textures,
) -> Result<String, Box<dyn std::error::Error>> {
    // Each branch's order-2 ancestor (itself for order 2), in storage order
    // so parents resolve first.
    let branches = skeleton.branches();
    let mut cluster = vec![None; branches.len()];
    for (index, branch) in branches.iter().enumerate() {
        cluster[index] = match branch.order {
            2 => Some(index),
            o if o > 2 => branch
                .parent
                .and_then(|a| skeleton.index_of(a.parent))
                .and_then(|p| cluster[p]),
            _ => None,
        };
    }
    let mut counts = vec![0_u32; branches.len()];
    for leaf in &foliage.instances {
        let site = &skeleton.sites()[leaf.site as usize];
        if let Some(root) = skeleton.index_of(site.branch).and_then(|i| cluster[i]) {
            counts[root] += 1;
        }
    }
    let Some((root, _)) = counts
        .iter()
        .enumerate()
        .filter(|&(_, &c)| c > 0)
        .max_by_key(|&(i, &c)| (c, usize::MAX - i))
    else {
        return Ok(String::new());
    };
    let branch = &branches[root];
    let in_cluster = |index: usize| cluster[index] == Some(root);

    // The cluster's bark triangles, selected by the branch stream.
    let Some(AttributeBuffer::U32(owner)) = bark.attribute(BRANCH_LAYER) else {
        return Err("bark extraction must carry the branch layer".into());
    };
    let mut remap = vec![u32::MAX; bark.positions.len()];
    let (mut positions, mut normals, mut uvs, mut indices) = (vec![], vec![], vec![], vec![]);
    for tri in bark.indices.as_chunks::<3>().0 {
        if !in_cluster(owner[tri[0] as usize] as usize) {
            continue;
        }
        for &v in tri {
            let v = v as usize;
            if remap[v] == u32::MAX {
                remap[v] = u32::try_from(positions.len())?;
                positions.push(bark.positions[v]);
                normals.push(bark.normals[v]);
                uvs.push(bark.uvs[v]);
            }
            indices.push(remap[v]);
        }
    }

    // Leaves on the twig, and the card frame they suggest.
    let leaves: Vec<_> = foliage
        .instances
        .iter()
        .filter(|leaf| {
            skeleton
                .index_of(skeleton.sites()[leaf.site as usize].branch)
                .is_some_and(in_cluster)
        })
        .collect();
    let base = branch.nodes[0].position;
    let tip = branch.nodes[branch.nodes.len() - 1].position;
    let up = (tip - base).normalize();
    let facing: Vec3 = leaves.iter().map(|l| l.rotation * Vec3::Z).sum();
    let toward = (facing - up * facing.dot(up))
        .try_normalize()
        .or_else(|| up.any_orthonormal_vector().try_normalize())
        .expect("a unit vector");
    let right = up.cross(toward);

    let templates: Vec<TriMesh> = foliage
        .templates
        .iter()
        .map(|t| t.mesh.to_trimesh(&exedra_mesh::ExtractParams::default()).0)
        .collect();
    let bark_material = BakeMaterial {
        base_color: Some(&textures.bark),
        ..BakeMaterial::default()
    };
    let leaf_material = BakeMaterial {
        base_color: textures.leaf.as_ref(),
        ..BakeMaterial::default()
    };
    let mut meshes = vec![BakeMesh {
        positions: &positions,
        normals: &normals,
        uvs: &uvs,
        indices: &indices,
        transform: Affine3A::IDENTITY,
        material: bark_material,
    }];
    for leaf in &leaves {
        let t = &templates[leaf.template as usize];
        meshes.push(BakeMesh {
            positions: &t.positions,
            normals: &t.normals,
            uvs: &t.uvs,
            indices: &t.indices,
            transform: Affine3A::from_scale_rotation_translation(
                Vec3::splat(leaf.scale),
                leaf.rotation,
                leaf.position,
            ),
            material: leaf_material,
        });
    }

    // Fit the card around everything it draws.
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for mesh in &meshes {
        for p in mesh.positions {
            let w = mesh.transform.transform_point3(Vec3::from_array(*p)) - base;
            let q = Vec3::new(w.dot(right), w.dot(up), w.dot(toward));
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    let margin = 0.05;
    let center3 = (lo + hi) * 0.5;
    let center = base + right * center3.x + up * center3.y + toward * center3.z;
    let half = Vec2::new(hi.x - lo.x, hi.y - lo.y) * 0.5 + Vec2::splat(margin);
    let half_depth = (hi.z - lo.z) * 0.5 + margin;
    let long = 512.0_f32;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "card sizes are small and positive"
    )]
    let size = if half.y >= half.x {
        [((long * half.x / half.y).ceil() as u32).max(8), long as u32]
    } else {
        [long as u32, ((long * half.y / half.x).ceil() as u32).max(8)]
    };
    let view = CardView::facing(center, right, up, half, half_depth);
    let baked = bake(&meshes, &view, &BakeSettings { size, samples: 4 })?;

    std::fs::create_dir_all(dir)?;
    let bundle = pack(
        &baked.maps(),
        Profile::Gltf,
        &PackSettings {
            filter: Filter::Kaiser,
            alpha_cutoff: Some(0.5),
            ..PackSettings::default()
        },
    )?;
    for texture in &bundle.textures {
        std::fs::write(
            dir.join(format!("{}.png", texture.name)),
            dapple_encode::png::write(texture)?,
        )?;
        std::fs::write(
            dir.join(format!("{}.ktx2", texture.name)),
            dapple_encode::ktx2::write(texture),
        )?;
    }
    write_gray(&dir.join("depth.png"), &baked.depth)?;
    write_gray(&dir.join("opacity.png"), &baked.opacity)?;
    let r = baked.report;
    Ok(format!(
        ",\"twig_card\":{{\"leaves\":{},\"triangles\":{},\"size\":[{},{}],\"covered_texels\":{},\
         \"extent_m\":[{:.3},{:.3}]}}",
        leaves.len(),
        r.triangles,
        size[0],
        size[1],
        r.covered_texels,
        2.0 * half.x,
        2.0 * half.y
    ))
}

/// Writes a 1-channel image in `[0, 1]` as an 8-bit grayscale PNG, row 0 on
/// top as in dapple's output.
pub(crate) fn write_gray(path: &Path, image: &Image) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut encoder =
        png::Encoder::new(std::io::BufWriter::new(file), image.width(), image.height());
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a clamped unit value scaled to a byte"
    )]
    let data: Vec<u8> = image
        .values()
        .iter()
        .map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
        .collect();
    encoder.write_header()?.write_image_data(&data)?;
    Ok(())
}
