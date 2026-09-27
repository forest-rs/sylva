// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Inspect generated crowns or branch subtrees using actual foliage geometry.
//! Run with `--help` for species, stable branch selection and camera controls.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use exedra_mesh::{AttributeBuffer, ExtractAttribute, ExtractParams, NormalsSource, TriMesh};
use serde::{Deserialize, Serialize};
use sylva_asset::{GeneratedTree, TreeMaterials};
use sylva_bake::glam::{Affine3A, Vec2, Vec3};
use sylva_bake::{BakeMaterial, BakeMesh, BakeSettings, CardView, bake};
use sylva_mesh::{BRANCH_LAYER, MeshParams, mesh_skeleton};
use sylva_skeleton::BranchId;
use sylva_species::Species;

type Error = Box<dyn std::error::Error>;

struct Options {
    source: String,
    seed: u64,
    branch: Option<BranchId>,
    list: bool,
    order: u32,
    size: u32,
    samples: u32,
    views: Vec<[f32; 2]>,
    output: PathBuf,
    obj: bool,
    frame: Option<PathBuf>,
}

fn options() -> Result<Option<Options>, Error> {
    let mut name = "beech".to_owned();
    let mut preset = None;
    let mut opts = Options {
        source: String::new(),
        seed: 1,
        branch: None,
        list: false,
        order: 2,
        size: 512,
        samples: 2,
        views: Vec::new(),
        output: ".local/gallery/review".into(),
        obj: false,
        frame: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!(
                "Usage: review [OPTIONS]\n\n\
                --species NAME     oak, spruce, beech, birch (default: beech)\n\
                --preset FILE      Read a saved/custom species RON instead\n\
                --seed NUMBER      Generation seed (default: 1)\n\
                --list             Print branch survey as JSON, without meshing\n\
                --order NUMBER     Branch order for --list (default: 2)\n\
                --branch HEX       Render this stable branch ID and descendants\n\
                --view AZ,EL       Camera angles in degrees; repeat for more views\n\
                --frame FILE       Reuse cameras from a previous review.ron\n\
                --size PIXELS      Square image width (default: 512)\n\
                --samples NUMBER   Samples per pixel axis, 1..8 (default: 2)\n\
                --obj              Also export selected subtree geometry; requires --branch\n\
                --out DIRECTORY    Output (default: .local/gallery/review)\n\n\
                Default views: 0,0; 90,0; 0,65. Outputs: species.ron, review.ron,\n\
                view-N.png (lit geometry), coverage-N.png (unscaled coverage).\n\
                Projected area measures the resolved union of bark and foliage,\n\
                not leaf area or a species realism score. Materials use plain colours."
            );
            return Ok(None);
        }
        if arg == "--list" {
            opts.list = true;
            continue;
        }
        if arg == "--obj" {
            opts.obj = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{arg} needs a value; use --help"))?;
        match arg.as_str() {
            "--species" => name = value,
            "--preset" => preset = Some(value),
            "--seed" => opts.seed = value.parse()?,
            "--branch" => opts.branch = Some(BranchId::from_bits(u64::from_str_radix(&value, 16)?)),
            "--order" => opts.order = value.parse()?,
            "--size" => opts.size = value.parse()?,
            "--samples" => opts.samples = value.parse()?,
            "--out" => opts.output = value.into(),
            "--frame" => opts.frame = Some(value.into()),
            "--view" => {
                let (az, el) = value
                    .split_once(',')
                    .ok_or("--view needs AZ,EL in degrees")?;
                let view = [az.parse::<f32>()?, el.parse::<f32>()?];
                if !view.iter().all(|v| v.is_finite()) || view[1].abs() > 90.0 {
                    return Err("view angles must be finite; elevation is -90..90".into());
                }
                opts.views.push(view);
            }
            _ => return Err(format!("unknown option {arg}; use --help").into()),
        }
    }
    if !(64..=4096).contains(&opts.size) || !(1..=8).contains(&opts.samples) {
        return Err("size must be 64..4096; samples must be 1..8".into());
    }
    if opts.obj && opts.branch.is_none() {
        return Err("--obj requires --branch to bound geometry expansion".into());
    }
    if opts.frame.is_some() && !opts.views.is_empty() {
        return Err("--frame reuses saved views; omit --view".into());
    }
    if opts.views.is_empty() {
        opts.views = vec![[0.0, 0.0], [90.0, 0.0], [0.0, 65.0]];
    }
    opts.source = match preset {
        Some(path) => std::fs::read_to_string(path)?,
        None => match name.as_str() {
            "oak" => include_str!("../../presets/oak.ron"),
            "spruce" => include_str!("../../presets/spruce.ron"),
            "beech" => include_str!("../../presets/beech.ron"),
            "birch" => include_str!("../../presets/birch.ron"),
            _ => {
                return Err(format!(
                    "unknown species {name:?}; expected oak, spruce, beech or birch"
                )
                .into());
            }
        }
        .into(),
    };
    Ok(Some(opts))
}

#[derive(Serialize, Deserialize)]
struct ViewReport {
    angles_degrees: [f32; 2],
    center_metres: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    half_extent_metres: f32,
    submitted_triangles: u64,
    covered_samples: u64,
    projected_area_m2: f64,
    render_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct Review {
    species: String,
    seed: u64,
    branch_id: Option<String>,
    realization: String,
    branches: usize,
    leaves: usize,
    templates: usize,
    size_pixels: u32,
    samples_per_axis: u32,
    views: Vec<ViewReport>,
}

fn placed<'a>(tri: &'a TriMesh, transform: Affine3A, color: [f32; 3]) -> BakeMesh<'a> {
    BakeMesh {
        positions: &tri.positions,
        normals: &tri.normals,
        uvs: &tri.uvs,
        indices: &tri.indices,
        transform,
        material: BakeMaterial {
            color,
            ..BakeMaterial::default()
        },
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "PNG channels are clamped to 0..255"
)]
fn write_images(
    out: &std::path::Path,
    index: usize,
    baked: &sylva_bake::Baked,
) -> Result<(), Error> {
    let width = baked.opacity.width() as usize;
    let height = baked.opacity.height() as usize;
    for coverage in [false, true] {
        let mut pixels = Vec::with_capacity(width * height * 3);
        // Baker row zero is the bottom of the card; PNG row zero is the top.
        for y in (0..height).rev() {
            for x in 0..width {
                let i = y * width + x;
                let alpha = baked.opacity.values()[i];
                let n = Vec3::from_slice(&baked.normal.values()[3 * i..3 * i + 3]);
                let light = 0.35 + 0.65 * n.dot(Vec3::new(-0.3, 0.6, 0.74).normalize()).max(0.0);
                for c in 0..3 {
                    let v = if coverage {
                        alpha
                    } else {
                        let color = baked.base_color.values()[3 * i + c] * light;
                        alpha * color.sqrt() + (1.0 - alpha) * 0.93
                    };
                    pixels.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
        }
        let name = if coverage { "coverage" } else { "view" };
        let file = std::fs::File::create(out.join(format!("{name}-{index}.png")))?;
        let mut encoder = png::Encoder::new(file, baked.opacity.width(), baked.opacity.height());
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
    }
    Ok(())
}

fn main() -> Result<(), Error> {
    let Some(opts) = options()? else {
        return Ok(());
    };
    let species: Species = ron::from_str(&opts.source)?;
    let reference = opts
        .frame
        .as_ref()
        .map(|path| -> Result<Review, Error> {
            Ok(ron::from_str(&std::fs::read_to_string(path)?)?)
        })
        .transpose()?;
    if let Some(reference) = &reference {
        let branch = opts.branch.map(|id| format!("{:016x}", id.bits()));
        if reference.species != species.name
            || reference.seed != opts.seed
            || reference.branch_id != branch
            || reference.views.is_empty()
        {
            return Err(
                "camera reference needs views of the same species, seed and branch selection"
                    .into(),
            );
        }
    }
    let grown = species.grow(opts.seed)?;
    let skeleton = &grown.skeleton;
    if let Some(id) = opts.branch
        && skeleton.index_of(id).is_none()
    {
        return Err(format!("unknown branch {id:?}; use --list --order N").into());
    }
    let mut site_counts = vec![0; skeleton.branches().len()];
    for site in skeleton.sites() {
        site_counts[skeleton.index_of(site.branch).expect("site owner")] += 1;
    }
    if opts.list {
        let rows: Vec<_> = skeleton.branches().iter().enumerate().filter(|(_, b)| b.order == opts.order).map(|(i,b)| format!("{{\"id\":\"{:016x}\",\"order\":{},\"length_m\":{},\"radius_m\":{},\"direct_sites\":{}}}", b.id.bits(), b.order, b.length(), b.nodes[0].radius, site_counts[i])).collect();
        println!("[{}]", rows.join(",\n"));
        return Ok(());
    }
    let mut selected = vec![false; skeleton.branches().len()];
    for (i, branch) in skeleton.branches().iter().enumerate() {
        selected[i] = opts.branch.is_none()
            || opts.branch == Some(branch.id)
            || branch
                .parent
                .is_some_and(|a| selected[skeleton.index_of(a.parent).expect("parent")]);
    }
    let tree = GeneratedTree::new(
        grown.skeleton,
        species.foliage.as_ref(),
        TreeMaterials::default(),
    )?;
    let bark = mesh_skeleton(tree.skeleton(), &MeshParams::default())?;
    let mut bark = bark
        .mesh
        .to_trimesh(&ExtractParams {
            normals: NormalsSource::CustomOnly,
            attributes: vec![ExtractAttribute::new(BRANCH_LAYER, u32::MAX)],
            ..ExtractParams::default()
        })
        .0;
    let Some(AttributeBuffer::U32(owners)) = bark.attribute(BRANCH_LAYER) else {
        return Err("bark has no branch provenance".into());
    };
    bark.indices = bark
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter(|face| selected[owners[face[0] as usize] as usize])
        .flatten()
        .copied()
        .collect();
    let foliage = tree.foliage();
    let templates: Vec<_> = foliage
        .templates
        .iter()
        .map(|t| {
            sylva_foliage::tissue_mesh(&t.shape)
                .map(|mesh| mesh.to_trimesh(&ExtractParams::default()).0)
        })
        .collect::<Result<_, _>>()?;
    let mut meshes = vec![placed(&bark, Affine3A::IDENTITY, [0.28, 0.17, 0.08])];
    let mut leaves = 0;
    for leaf in &foliage.instances {
        if !selected[tree
            .skeleton()
            .index_of(leaf.id.branch)
            .expect("leaf owner")]
        {
            continue;
        }
        meshes.push(placed(
            &templates[leaf.template as usize],
            Affine3A::from_scale_rotation_translation(
                Vec3::splat(leaf.scale),
                leaf.rotation,
                leaf.position,
            ),
            [0.12, 0.3, 0.045],
        ));
        leaves += 1;
    }
    // Reuse local prototype bounds; detailed conifer templates contain many
    // needles, but an instance only needs eight transformed bounds corners.
    let bounds = |mesh: &TriMesh| {
        mesh.indices.iter().fold(
            (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
            |(a, b), &i| {
                let p = Vec3::from_array(mesh.positions[i as usize]);
                (a.min(p), b.max(p))
            },
        )
    };
    let template_bounds: Vec<_> = templates.iter().map(bounds).collect();
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    let mut extend = |(a, b): (Vec3, Vec3), transform: Affine3A| {
        for x in [a.x, b.x] {
            for y in [a.y, b.y] {
                for z in [a.z, b.z] {
                    let p = transform.transform_point3(Vec3::new(x, y, z));
                    low = low.min(p);
                    high = high.max(p);
                }
            }
        }
    };
    if !bark.indices.is_empty() {
        extend(bounds(&bark), Affine3A::IDENTITY);
    }
    for leaf in &foliage.instances {
        if selected[tree
            .skeleton()
            .index_of(leaf.id.branch)
            .expect("leaf owner")]
        {
            extend(
                template_bounds[leaf.template as usize],
                Affine3A::from_scale_rotation_translation(
                    Vec3::splat(leaf.scale),
                    leaf.rotation,
                    leaf.position,
                ),
            );
        }
    }
    let center = (low + high) * 0.5;
    let half = (high - low).length() * 0.53;
    std::fs::create_dir_all(&opts.output)?;
    std::fs::write(opts.output.join("species.ron"), &opts.source)?;
    let mut report = Review {
        species: species.name,
        seed: opts.seed,
        branch_id: opts.branch.map(|id| format!("{:016x}", id.bits())),
        realization: "opaque_tissue_and_embedded_bark".into(),
        branches: selected.iter().filter(|&&s| s).count(),
        leaves,
        templates: templates.len(),
        size_pixels: opts.size,
        samples_per_axis: opts.samples,
        views: Vec::new(),
    };
    let cameras: Vec<([f32; 2], CardView)> = if let Some(reference) = &reference {
        reference
            .views
            .iter()
            .map(|v| {
                (
                    v.angles_degrees,
                    CardView::facing(
                        Vec3::from_array(v.center_metres),
                        Vec3::from_array(v.right),
                        Vec3::from_array(v.up),
                        Vec2::splat(v.half_extent_metres),
                        v.half_extent_metres,
                    ),
                )
            })
            .collect()
    } else {
        opts.views
            .iter()
            .map(|&angles| {
                let (az, el) = (angles[0].to_radians(), angles[1].to_radians());
                let right = Vec3::new(az.cos(), az.sin(), 0.0);
                let toward = Vec3::new(az.sin() * el.cos(), -az.cos() * el.cos(), el.sin());
                (
                    angles,
                    CardView::facing(center, right, toward.cross(right), Vec2::splat(half), half),
                )
            })
            .collect()
    };
    for (index, &(angles, view)) in cameras.iter().enumerate() {
        let started = Instant::now();
        let baked = bake(
            &meshes,
            &view,
            &BakeSettings {
                size: [opts.size; 2],
                samples: opts.samples,
            },
        )?;
        let render_ms = u64::try_from(started.elapsed().as_millis())?;
        let area = baked
            .opacity
            .values()
            .iter()
            .map(|&a| f64::from(a))
            .sum::<f64>()
            * (2.0 * f64::from(view.half.x) / f64::from(opts.size)).powi(2);
        write_images(&opts.output, index, &baked)?;
        report.views.push(ViewReport {
            angles_degrees: angles,
            center_metres: view.center.to_array(),
            right: view.right.to_array(),
            up: view.up.to_array(),
            half_extent_metres: view.half.x,
            submitted_triangles: baked.report.triangles,
            covered_samples: baked.report.covered_samples,
            projected_area_m2: area,
            render_ms,
        });
        println!("view {index}: {area:.5} m² projected coverage, {render_ms} ms");
    }
    if opts.obj {
        write_obj(&opts.output.join("branch.obj"), &meshes)?;
    }
    std::fs::write(
        opts.output.join("review.ron"),
        ron::ser::to_string_pretty(&report, ron::ser::PrettyConfig::default())?,
    )?;
    println!(
        "{} branches, {} leaves; wrote {}",
        report.branches,
        leaves,
        opts.output.display()
    );
    Ok(())
}

fn write_obj(path: &std::path::Path, meshes: &[BakeMesh<'_>]) -> Result<(), Error> {
    let mut out = String::from("# sylva tissue review; bark first, then foliage\n");
    let mut base = 1_usize;
    for (i, mesh) in meshes.iter().enumerate() {
        writeln!(out, "g {}", if i == 0 { "bark" } else { "foliage" })?;
        let mut indices = mesh.indices.to_vec();
        indices.sort_unstable();
        indices.dedup();
        let mut remap = vec![0; mesh.positions.len()];
        for (local, &index) in indices.iter().enumerate() {
            remap[index as usize] = base + local;
            let p = mesh
                .transform
                .transform_point3(Vec3::from_array(mesh.positions[index as usize]));
            writeln!(out, "v {} {} {}", p.x, p.y, p.z)?;
        }
        for face in mesh.indices.as_chunks::<3>().0 {
            writeln!(
                out,
                "f {} {} {}",
                remap[face[0] as usize], remap[face[1] as usize], remap[face[2] as usize]
            )?;
        }
        base += indices.len();
    }
    std::fs::write(path, out)?;
    Ok(())
}
