// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Baking cluster and impostor atlases with `sylva_bake`.

use alloc::vec;
use alloc::vec::Vec;

use dapple_encode::Image;
use exedra_mesh::{AttributeBuffer, ExtractParams, TriMesh};
use glam::{Affine3A, Vec3};
use sylva_bake::{BakeMaterial, BakeMesh, BakeReport, BakeSettings, Baked, CardView, bake};
use sylva_foliage::Foliage;
use sylva_mesh::BRANCH_LAYER;
use sylva_skeleton::Skeleton;

use crate::LodError;
use crate::clusters::{AtlasLayout, Clusters, cluster_roots};
use crate::impostor::Impostor;
use dapple_raster::Edge;

/// Materials the atlases are baked with.
#[derive(Copy, Clone, Debug)]
pub struct CardMaterials<'a> {
    /// Bark, sampled by the bark UVs.
    pub bark: BakeMaterial<'a>,
    /// Leaves, sampled by the leaf UVs.
    pub leaf: BakeMaterial<'a>,
}

/// Size and sampling of every atlas cell.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AtlasSettings {
    /// Texels across and up one cell.
    pub cell: [u32; 2],
    /// Samples per texel along each axis.
    pub samples: u32,
}

impl Default for AtlasSettings {
    fn default() -> Self {
        Self {
            cell: [256, 256],
            samples: 4,
        }
    }
}

/// A baked atlas: its grid and its maps, cells laid out per [`AtlasLayout`].
#[derive(Clone, Debug)]
pub struct Atlas {
    /// The cell grid.
    pub layout: AtlasLayout,
    /// The maps over the whole atlas; the report sums every cell's.
    pub baked: Baked,
}

/// Bakes each of a level's cluster exemplars into its atlas cell: the
/// exemplar's bark (triangles of `bark` whose [`BRANCH_LAYER`] value lies in
/// the cluster) and its leaves at full detail, seen through the exemplar's
/// card view.
///
/// `bark` is the full-detail bark extracted with the branch stream carried.
///
/// # Errors
///
/// [`LodError::Bake`] when `bark` lacks the branch stream or a bake fails.
pub fn bake_clusters(
    skeleton: &Skeleton,
    bark: &TriMesh,
    foliage: &Foliage,
    clusters: &Clusters,
    materials: &CardMaterials<'_>,
    settings: &AtlasSettings,
) -> Result<Atlas, LodError> {
    let Some(AttributeBuffer::U32(owner)) = bark.attribute(BRANCH_LAYER) else {
        return Err(LodError::Bake("bark must carry the branch layer"));
    };
    let roots = cluster_roots(skeleton, clusters.params.root_order);
    let templates = leaf_templates(foliage);
    let cells = clusters
        .variants
        .iter()
        .map(|variant| {
            let card = &clusters.cards[variant.exemplar as usize];
            let root = Some(card.root as usize);
            let part = select(bark, |t| {
                roots.get(owner[t[0] as usize] as usize).copied().flatten() == root
            });
            let leaves = &clusters.members[variant.exemplar as usize];
            bake_cell(
                &part,
                leaves.iter().map(|&l| l as usize),
                foliage,
                &templates,
                materials,
                &variant.view,
                settings,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    compose(&cells, clusters.layout(), settings)
}

/// Bakes an impostor's planes, each from its own side, over the whole tree:
/// all of `bark` and every leaf at full detail.
///
/// # Errors
///
/// [`LodError::Bake`] when a bake fails.
pub fn bake_impostor(
    bark: &TriMesh,
    foliage: &Foliage,
    impostor: &Impostor,
    materials: &CardMaterials<'_>,
    settings: &AtlasSettings,
) -> Result<Atlas, LodError> {
    let templates = leaf_templates(foliage);
    let cells = impostor
        .views
        .iter()
        .map(|view| {
            bake_cell(
                bark,
                0..foliage.instances.len(),
                foliage,
                &templates,
                materials,
                view,
                settings,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    compose(&cells, impostor.layout(), settings)
}

fn leaf_templates(foliage: &Foliage) -> Vec<TriMesh> {
    foliage
        .templates
        .iter()
        .map(|t| t.mesh.to_trimesh(&ExtractParams::default()).0)
        .collect()
}

/// The triangles of `mesh` that `keep` accepts, with their vertices.
fn select(mesh: &TriMesh, keep: impl Fn(&[u32; 3]) -> bool) -> TriMesh {
    let mut remap = vec![u32::MAX; mesh.positions.len()];
    let mut out = TriMesh::default();
    for tri in mesh.indices.as_chunks::<3>().0 {
        if !keep(tri) {
            continue;
        }
        for &v in tri {
            let v = v as usize;
            if remap[v] == u32::MAX {
                remap[v] = u32::try_from(out.positions.len()).expect("fits the source");
                out.positions.push(mesh.positions[v]);
                out.normals.push(mesh.normals[v]);
                out.uvs.push(mesh.uvs[v]);
            }
            out.indices.push(remap[v]);
        }
    }
    out
}

fn bake_cell(
    bark: &TriMesh,
    leaves: impl Iterator<Item = usize>,
    foliage: &Foliage,
    templates: &[TriMesh],
    materials: &CardMaterials<'_>,
    view: &CardView,
    settings: &AtlasSettings,
) -> Result<Baked, LodError> {
    let mut meshes = vec![BakeMesh {
        positions: &bark.positions,
        normals: &bark.normals,
        uvs: &bark.uvs,
        indices: &bark.indices,
        transform: Affine3A::IDENTITY,
        material: materials.bark,
    }];
    for index in leaves {
        let leaf = &foliage.instances[index];
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
            material: materials.leaf,
        });
    }
    bake(
        &meshes,
        view,
        &BakeSettings {
            size: settings.cell,
            samples: settings.samples,
        },
    )
    .map_err(|_| LodError::Bake("a card bake failed"))
}

/// Copies equally sized cells into one atlas per map.
fn compose(
    cells: &[Baked],
    layout: AtlasLayout,
    settings: &AtlasSettings,
) -> Result<Atlas, LodError> {
    let [cw, ch] = settings.cell;
    let (w, h) = (cw * layout.columns, ch * layout.rows);
    let map = |pick: fn(&Baked) -> &Image, channels: usize| -> Result<Image, LodError> {
        let mut values = vec![0.0; w as usize * h as usize * channels];
        for (index, cell) in cells.iter().enumerate() {
            let index = u32::try_from(index).expect("few cells");
            let (x0, y0) = ((index % layout.columns) * cw, (index / layout.columns) * ch);
            let image = pick(cell);
            for y in 0..ch {
                for x in 0..cw {
                    let at = ((y0 + y) as usize * w as usize + (x0 + x) as usize) * channels;
                    values[at..at + channels].copy_from_slice(image.texel(x, y));
                }
            }
        }
        let edge = cells.first().map_or(Edge::Clamp, |c| pick(c).edge());
        Image::new(w, h, channels, edge, values).map_err(|_| LodError::Bake("atlas image"))
    };
    let report = cells.iter().fold(BakeReport::default(), |mut sum, c| {
        let r = c.report;
        sum.triangles += r.triangles;
        sum.covered_samples += r.covered_samples;
        sum.covered_texels += r.covered_texels;
        sum.alpha_discards += r.alpha_discards;
        sum
    });
    Ok(Atlas {
        layout,
        baked: Baked {
            base_color: map(|b| &b.base_color, 3)?,
            opacity: map(|b| &b.opacity, 1)?,
            normal: map(|b| &b.normal, 3)?,
            depth: map(|b| &b.depth, 1)?,
            report,
        },
    })
}
