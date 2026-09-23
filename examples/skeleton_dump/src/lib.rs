// Copyright 2026 the Sylva Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Debug dumps of a sylva skeleton, shared by the review examples.

use std::fmt::Write as _;

use glam::Vec3;
use sylva_skeleton::Skeleton;

fn vec3_json(v: Vec3) -> String {
    format!("[{},{},{}]", v.x, v.y, v.z)
}

/// The skeleton as JSON: branches (ID, order, parent attachment, nodes with
/// position, radius, tangent and normal) and sites with their positions.
/// `tools/render.py` reads this.
///
/// # Errors
///
/// Only if formatting into the string fails.
pub fn skeleton_json(skeleton: &Skeleton) -> Result<String, std::fmt::Error> {
    let mut out = String::from("{\"branches\":[");
    for (i, branch) in skeleton.branches().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let parent = branch.parent.map_or_else(
            || "null".to_owned(),
            |a| format!("{{\"id\":\"{}\",\"t\":{}}}", a.parent, a.t),
        );
        write!(
            out,
            "{{\"id\":\"{}\",\"order\":{},\"parent\":{parent},\"nodes\":[",
            branch.id, branch.order
        )?;
        for (j, node) in branch.nodes.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            write!(
                out,
                "{{\"p\":{},\"r\":{},\"tangent\":{},\"normal\":{}}}",
                vec3_json(node.position),
                node.radius,
                vec3_json(node.frame.tangent),
                vec3_json(node.frame.normal)
            )?;
        }
        out.push_str("]}");
    }
    out.push_str("],\"sites\":[");
    for (i, site) in skeleton.sites().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let branch = skeleton.branch(site.branch).expect("site branch exists");
        write!(
            out,
            "{{\"branch\":\"{}\",\"t\":{},\"p\":{},\"scale\":{}}}",
            site.branch,
            site.t,
            vec3_json(branch.sample(site.t).position),
            site.scale
        )?;
    }
    out.push_str("]}\n");
    Ok(out)
}

/// Branch centerlines as OBJ polylines, one object per branch order.
///
/// # Errors
///
/// Only if formatting into the string fails.
pub fn skeleton_obj(skeleton: &Skeleton) -> Result<String, std::fmt::Error> {
    let mut out = String::from("# sylva skeleton centerlines; Z up, metres\n");
    let mut next_vertex = 1;
    let max_order = skeleton
        .branches()
        .iter()
        .map(|b| b.order)
        .max()
        .unwrap_or(0);
    for order in 0..=max_order {
        writeln!(out, "o order_{order}")?;
        for branch in skeleton.branches().iter().filter(|b| b.order == order) {
            let first = next_vertex;
            for node in &branch.nodes {
                let p = node.position;
                writeln!(out, "v {} {} {}", p.x, p.y, p.z)?;
                next_vertex += 1;
            }
            out.push('l');
            for index in first..next_vertex {
                write!(out, " {index}")?;
            }
            out.push('\n');
        }
    }
    Ok(out)
}
