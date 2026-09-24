// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The reference harness: measured reports per species and seed against
//! the species' reference ranges, a contact sheet, and an allometry fit.

use std::path::Path;

use dapple_lab::Report;
use dapple_lab::fit::{Cmaes, Dimension, Space, Target, fit, loss};
use sylva_grow::{Curve, Hierarchy};
use sylva_measure::{Reference, TreeMeasures, measure};
use sylva_species::{Growth, Species};
use sylva_texture::{LeafRecipe, bark, leaf};

use crate::{LeafLook, Preset};

/// Measurement names the fit and the regression test hold to the
/// reference: every reference range except texture colour.
pub(crate) fn form_targets(reference: &Reference) -> Vec<Target> {
    reference
        .targets("")
        .into_iter()
        .filter(|t| !t.name.starts_with("colour."))
        .collect()
}

/// The radius, in metres, one foliage site covers in silhouettes: half a
/// leaf's length.
pub(crate) fn leaf_radius(species: &Species) -> f32 {
    species
        .foliage
        .as_ref()
        .map_or(0.05, |f| 0.5 * f.shape.length)
}

/// Grows `species` from `seed` and measures it.
pub(crate) fn measure_seed(
    species: &Species,
    seed: u64,
) -> Result<TreeMeasures, Box<dyn std::error::Error>> {
    let grown = species.grow(seed)?;
    Ok(measure(&grown.skeleton, leaf_radius(species)))
}

/// Linear to sRGB-encoded, in `[0, 255]`.
fn srgb(linear: f32) -> f64 {
    let c = linear.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    f64::from(v) * 255.0
}

/// Colour measurements of a species' texture sets: the leaf's mean
/// sRGB green over its coverage, and the bark's mean sRGB luma.
fn colour(
    species: &Species,
    preset: &Preset,
) -> Result<Vec<(String, f64)>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    if let Some(foliage) = &species.foliage {
        let look: LeafLook = match &preset.leaf {
            Some(source) => ron::from_str(source)?,
            None => LeafLook::default(),
        };
        let set = leaf(&LeafRecipe {
            shape: foliage.shape,
            green: look.green,
            vein: look.vein,
            translucent: look.translucent,
            translucency: look.translucency,
            mottle: look.mottle,
            roughness: look.roughness,
            ..LeafRecipe::default()
        })?;
        let (Some(color), Some(opacity)) = (&set.maps.base_color, &set.maps.opacity) else {
            return Err("leaf sets have colour and opacity".into());
        };
        let (mut green, mut covered) = (0.0, 0.0);
        for (texel, &alpha) in color.values().chunks(3).zip(opacity.values()) {
            if alpha >= 0.5 {
                green += srgb(texel[1]);
                covered += 1.0;
            }
        }
        out.push((
            "colour.leaf.srgb_green".into(),
            green / f64::max(covered, 1.0),
        ));
    }
    let recipe: dapple_graph::Recipe = toml::from_str(&preset.bark)?;
    let set = bark(&recipe)?;
    let color = set.maps.base_color.ok_or("bark sets have colour")?;
    let texels = color.values().chunks(3);
    let n = texels.len();
    let luma: f64 = texels
        .map(|t| 0.2126 * srgb(t[0]) + 0.7152 * srgb(t[1]) + 0.0722 * srgb(t[2]))
        .sum();
    #[expect(clippy::cast_precision_loss, reason = "a mean over texels")]
    out.push(("colour.bark.srgb_luma".into(), luma / n as f64));
    Ok(out)
}

/// A report of `measures` and `colours` against `reference`.
fn report(
    subject: &str,
    measures: &TreeMeasures,
    colours: &[(String, f64)],
    reference: &Reference,
) -> Report {
    let mut report = measures.report(subject, reference);
    for (name, value) in colours {
        let bounds = reference
            .ranges
            .iter()
            .find(|r| &r.name == name)
            .map(|r| [r.lo, r.hi]);
        report.measure(name, *value, "", bounds);
    }
    report
}

/// Writes a report per species and seed as `<dir>/<species>-seed<n>.json`,
/// `summary.json` with each report's pass and failures, and `contact.png`,
/// every tree's side silhouette in a species by seed grid, framed green
/// when its report passes and red when it does not.
pub(crate) fn run(
    dir: &Path,
    presets: &[Preset],
    seeds: &[u64],
    only: Option<&[String]>,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let mut summary = Vec::new();
    let mut sheet: Vec<Vec<Cell>> = Vec::new();
    for preset in presets {
        let species: Species = ron::from_str(&preset.species)?;
        if only.is_some_and(|names| !names.contains(&species.name)) {
            continue;
        }
        let Some(reference) = &preset.reference else {
            continue;
        };
        let reference = ron::from_str::<Reference>(reference)?.for_condition(species.grown_in);
        let colours = colour(&species, preset)?;
        let mut row = Vec::new();
        for &seed in seeds {
            let grown = species.grow(seed)?;
            let measures = measure(&grown.skeleton, leaf_radius(&species));
            let subject = format!("{}-seed{seed}", species.name);
            let report = report(&subject, &measures, &colours, &reference);
            std::fs::write(dir.join(format!("{subject}.json")), report.to_json())?;
            let failures: Vec<String> = report
                .failures()
                .map(|e| match e {
                    dapple_lab::Entry::Measurement(m) => {
                        format!("{} = {:.3} outside {:?}", m.name, m.value, m.bounds)
                    }
                    dapple_lab::Entry::Check(c) => format!("{}: {}", c.name, c.detail),
                })
                .collect();
            let loss = loss(
                &form_targets(&reference),
                &form_targets(&reference)
                    .iter()
                    .map(|t| measures.value(&t.name).unwrap_or(f64::NAN))
                    .collect::<Vec<_>>(),
            );
            println!(
                "{subject}: {} ({} failures), form loss {loss:.3}",
                if report.passed() { "pass" } else { "FAIL" },
                failures.len()
            );
            for failure in &failures {
                println!("  {failure}");
            }
            summary.push(format!(
                "{{\"subject\":\"{subject}\",\"passed\":{},\"form_loss\":{loss},\"failures\":{:?}}}",
                report.passed(),
                failures
            ));
            let (image, w, h) = silhouette(&grown.skeleton, leaf_radius(&species));
            row.push((image, w, h, report.passed()));
        }
        sheet.push(row);
    }
    std::fs::write(
        dir.join("summary.json"),
        format!("[{}]\n", summary.join(",")),
    )?;
    write_sheet(&dir.join("contact.png"), &sheet)?;
    println!("wrote {}", dir.display());
    Ok(())
}

/// One contact sheet cell: a square silhouette, its width and height, and
/// whether the tree's report passed.
type Cell = (Vec<bool>, usize, usize, bool);

/// Metres the contact sheet's cells cover, and pixels per metre.
const CELL_METRES: f32 = 36.0;
const CELL_SCALE: f32 = 8.0;

/// A front view of a tree: leaf sites as disks and branches of at least
/// 2 cm radius, in a square cell `CELL_METRES` across, ground at the
/// bottom.
fn silhouette(skeleton: &sylva_skeleton::Skeleton, leaf_radius: f32) -> (Vec<bool>, usize, usize) {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a fixed small cell"
    )]
    let size = (CELL_METRES * CELL_SCALE) as usize;
    let mut image = vec![false; size * size];
    let mut disk = |x: f32, z: f32, r: f32| {
        let cx = (x + 0.5 * CELL_METRES) * CELL_SCALE;
        let cy = (CELL_METRES - z) * CELL_SCALE;
        let rp = (r * CELL_SCALE).max(0.5);
        let (x0, x1) = ((cx - rp).floor(), (cx + rp).ceil());
        let (y0, y1) = ((cy - rp).floor(), (cy + rp).ceil());
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "clamped to the small cell"
        )]
        for py in (y0.max(0.0) as usize)..(y1.min(size as f32) as usize) {
            for px in (x0.max(0.0) as usize)..(x1.min(size as f32) as usize) {
                let (dx, dy) = (px as f32 + 0.5 - cx, py as f32 + 0.5 - cy);
                if dx * dx + dy * dy <= rp * rp {
                    image[py * size + px] = true;
                }
            }
        }
    };
    for site in skeleton.sites() {
        if let Some(branch) = skeleton.branch(site.branch) {
            let p = branch.sample(site.t).position;
            disk(p.x, p.z, leaf_radius);
        }
    }
    for branch in skeleton.branches() {
        for pair in branch.nodes.windows(2) {
            if pair[0].radius < 0.02 {
                continue;
            }
            for i in 0..=8 {
                let p = pair[0].position.lerp(pair[1].position, i as f32 / 8.0);
                disk(p.x, p.z, pair[0].radius);
            }
        }
    }
    (image, size, size)
}

/// Writes the contact sheet: rows of species, columns of seeds.
fn write_sheet(path: &Path, sheet: &[Vec<Cell>]) -> Result<(), Box<dyn std::error::Error>> {
    const BORDER: usize = 4;
    let columns = sheet.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let cell = sheet
        .iter()
        .flatten()
        .map(|(_, w, _, _)| *w)
        .max()
        .unwrap_or(1);
    let stride = cell + 2 * BORDER;
    let (w, h) = (columns * stride, sheet.len().max(1) * stride);
    let mut rgb = vec![255_u8; w * h * 3];
    for (r, row) in sheet.iter().enumerate() {
        for (c, (image, iw, ih, passed)) in row.iter().enumerate() {
            let frame = if *passed {
                [40, 160, 60]
            } else {
                [200, 40, 40]
            };
            for y in 0..stride {
                for x in 0..stride {
                    let (gx, gy) = (c * stride + x, r * stride + y);
                    let inner =
                        (BORDER..BORDER + iw).contains(&x) && (BORDER..BORDER + ih).contains(&y);
                    let px = if !inner {
                        frame
                    } else if image[(y - BORDER) * iw + (x - BORDER)] {
                        [45, 75, 40]
                    } else {
                        [235, 240, 248]
                    };
                    rgb[(gy * w + gx) * 3..(gy * w + gx) * 3 + 3].copy_from_slice(&px);
                }
            }
        }
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        u32::try_from(w)?,
        u32::try_from(h)?,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&rgb)?;
    Ok(())
}

/// The oak parameters the fit searches, and the hand-tuned preset's values
/// of them.
fn oak_space(hierarchy: &Hierarchy) -> (Space, Vec<f64>) {
    let envelope = hierarchy.envelope.as_ref().map_or(12.0, |e| e.radius);
    (
        Space {
            dimensions: vec![
                Dimension::linear("trunk.length", 5.0, 14.0),
                Dimension::linear("scaffold.length_scale", 0.6, 1.5),
                Dimension::linear("scaffold.angle_scale", 0.6, 1.3),
                Dimension::linear("envelope.radius", 6.0, 18.0),
                Dimension::linear("radii.exponent", 2.2, 3.0),
            ],
        },
        vec![
            f64::from(hierarchy.trunk.length),
            1.0,
            1.0,
            f64::from(envelope),
            f64::from(hierarchy.radii.exponent),
        ],
    )
}

/// `species` with the fit's parameters applied.
fn with_params(species: &Species, params: &[f64]) -> Result<Species, Box<dyn std::error::Error>> {
    let mut out = species.clone();
    let Growth::Hierarchical(h) = &mut out.growth else {
        return Err("the oak grows hierarchically".into());
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "parameters in small ranges"
    )]
    let p: Vec<f32> = params.iter().map(|&v| v as f32).collect();
    h.trunk.length = p[0];
    let scale = |curve: &Curve, s: f32| {
        Curve::new(curve.points().iter().map(|&[x, y]| [x, y * s]).collect())
    };
    let scaffold = h.levels.first_mut().ok_or("the oak has scaffold limbs")?;
    scaffold.length = scale(&scaffold.length, p[1])?;
    scaffold.angle = scale(&scaffold.angle, p[2])?;
    if let Some(envelope) = &mut h.envelope {
        envelope.radius = p[3];
    }
    h.radii.exponent = p[4];
    Ok(out)
}

/// The form loss of `species` over `seeds`: the mean of each seed's loss
/// against `targets`.
fn seeds_loss(
    species: &Species,
    targets: &[Target],
    seeds: &[u64],
) -> Result<f64, Box<dyn std::error::Error>> {
    let mut total = 0.0;
    for &seed in seeds {
        let m = measure_seed(species, seed)?;
        let values: Vec<f64> = targets
            .iter()
            .map(|t| m.value(&t.name).unwrap_or(f64::NAN))
            .collect();
        total += loss(targets, &values);
    }
    #[expect(clippy::cast_precision_loss, reason = "few seeds")]
    Ok(total / seeds.len() as f64)
}

/// Fits five oak parameters (trunk length, scaffold length and angle
/// scales, envelope radius, pipe-model exponent) to the oak reference's form ranges with
/// `dapple_lab::fit`, from the hand-tuned preset, on seed 1; then scores
/// the hand-tuned and fitted presets on seeds 1 to 4 (2 to 4 held out).
/// Writes `oak-fit.json` (the fit in `dapple_lab`'s report format) and
/// `oak-fitted.ron`.
pub(crate) fn fit_oak(dir: &Path, presets: &[Preset]) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let preset = presets
        .iter()
        .find(|p| ron::from_str::<Species>(&p.species).is_ok_and(|s| s.name == "oak"))
        .ok_or("no oak preset")?;
    let species: Species = ron::from_str(&preset.species)?;
    let reference = ron::from_str::<Reference>(preset.reference.as_deref().ok_or("no reference")?)?
        .for_condition(species.grown_in);
    let targets = form_targets(&reference);
    let Growth::Hierarchical(hierarchy) = &species.growth else {
        return Err("the oak grows hierarchically".into());
    };
    let (space, start) = oak_space(hierarchy);
    let started = std::time::Instant::now();
    let result = fit(
        &space,
        &targets,
        &Cmaes {
            sigma: 0.2,
            max_evaluations: 80,
            target_loss: 0.0,
            ..Cmaes::default()
        },
        Some(&start),
        |params| -> Result<Vec<f64>, Box<dyn std::error::Error>> {
            let m = measure_seed(&with_params(&species, params)?, 1)?;
            Ok(targets
                .iter()
                .map(|t| m.value(&t.name).unwrap_or(f64::NAN))
                .collect())
        },
    )?;
    let fitted = with_params(&species, &result.params)?;
    let seeds = [1, 2, 3, 4];
    let hand = seeds_loss(&species, &targets, &seeds)?;
    let tuned = seeds_loss(&fitted, &targets, &seeds)?;
    let held_hand = seeds_loss(&species, &targets, &seeds[1..])?;
    let held_tuned = seeds_loss(&fitted, &targets, &seeds[1..])?;
    std::fs::write(
        dir.join("oak-fit.json"),
        result.report("oak fit", &space, &targets).to_json(),
    )?;
    std::fs::write(
        dir.join("oak-fitted.ron"),
        ron::ser::to_string_pretty(&fitted, ron::ser::PrettyConfig::default())?,
    )?;
    println!(
        "oak fit: {} evaluations in {:.0} s; seed-1 loss {:.3}",
        result.evaluations,
        started.elapsed().as_secs_f64(),
        result.loss
    );
    for (dimension, (from, to)) in space
        .dimensions
        .iter()
        .zip(start.iter().zip(&result.params))
    {
        println!("  {}: {from:.3} -> {to:.3}", dimension.name);
    }
    println!(
        "mean form loss over seeds 1-4: hand-tuned {hand:.3}, fitted {tuned:.3}; \
         held-out seeds 2-4: hand-tuned {held_hand:.3}, fitted {held_tuned:.3}"
    );
    Ok(())
}
