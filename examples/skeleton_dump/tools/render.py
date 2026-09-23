# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render a skeleton_dump output directory with headless Blender.

    blender --background --python examples/skeleton_dump/tools/render.py -- .local/gallery/skeleton-dump

Reads `skeleton.json` and builds one curve per branch whose bevel radius
follows the pipe-model radii, so the render shows the IR exactly: centerlines,
radii and branch order (color). Leaf sites are small green spheres. Writes
`skeleton.png` (three-quarter view) and `skeleton-side.png` next to the input.
Coordinates are used unchanged (sylva is Z-up, like Blender).
"""

import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector

ORDER_COLORS = [
    (0.36, 0.22, 0.12, 1.0),
    (0.55, 0.35, 0.18, 1.0),
    (0.75, 0.55, 0.30, 1.0),
    (0.90, 0.78, 0.50, 1.0),
    (0.95, 0.90, 0.70, 1.0),
]
SITE_COLOR = (0.25, 0.65, 0.20, 1.0)


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if not argv:
        sys.exit("usage: render.py -- <skeleton_dump output dir>")
    return Path(argv[0])


def reset_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    shading = scene.display.shading
    shading.light = "STUDIO"
    shading.color_type = "OBJECT"
    shading.show_cavity = True
    shading.show_shadows = True
    scene.render.resolution_x = 1200
    scene.render.resolution_y = 1400
    world = bpy.data.worlds.new("world")
    world.color = (0.93, 0.93, 0.91)
    scene.world = world
    return scene


def branch_curve(branch, collection):
    curve = bpy.data.curves.new(f"branch_{branch['id']}", "CURVE")
    curve.dimensions = "3D"
    curve.bevel_depth = 1.0
    curve.bevel_resolution = 3
    curve.use_fill_caps = True
    spline = curve.splines.new("POLY")
    nodes = branch["nodes"]
    spline.points.add(len(nodes) - 1)
    for point, node in zip(spline.points, nodes):
        x, y, z = node["p"]
        point.co = (x, y, z, 1.0)
        point.radius = node["r"]
    obj = bpy.data.objects.new(curve.name, curve)
    obj.color = ORDER_COLORS[min(branch["order"], len(ORDER_COLORS) - 1)]
    collection.objects.link(obj)
    return obj


def site_spheres(sites, collection):
    if not sites:
        return
    mesh = bpy.data.meshes.new("sites")
    verts = [tuple(site["p"]) for site in sites]
    mesh.from_pydata(verts, [], [])
    carrier = bpy.data.objects.new("sites", mesh)
    # Workbench's object color for vertex instances comes from the instancer.
    carrier.color = SITE_COLOR
    collection.objects.link(carrier)
    bpy.ops.mesh.primitive_ico_sphere_add(subdivisions=2, radius=0.06)
    sphere = bpy.context.active_object
    sphere.color = SITE_COLOR
    sphere.parent = carrier
    carrier.instance_type = "VERTS"
    for c in sphere.users_collection:
        c.objects.unlink(sphere)
    collection.objects.link(sphere)


def bounds(objects):
    lo = Vector((math.inf,) * 3)
    hi = Vector((-math.inf,) * 3)
    for obj in objects:
        for corner in obj.bound_box:
            world = obj.matrix_world @ Vector(corner)
            lo = Vector(map(min, lo, world))
            hi = Vector(map(max, hi, world))
    return lo, hi


def add_camera(scene, target, distance, azimuth, elevation, name):
    direction = Vector(
        (
            math.cos(elevation) * math.cos(azimuth),
            math.cos(elevation) * math.sin(azimuth),
            math.sin(elevation),
        )
    )
    cam_data = bpy.data.cameras.new(name)
    cam_data.lens = 50
    cam = bpy.data.objects.new(name, cam_data)
    scene.collection.objects.link(cam)
    cam.location = target + direction * distance
    # Never exactly horizontal: elevation is always nonzero, so no roll.
    cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
    return cam


def main():
    out_dir = args()
    data = json.loads((out_dir / "skeleton.json").read_text())
    scene = reset_scene()
    collection = bpy.data.collections.new("skeleton")
    scene.collection.children.link(collection)

    curves = [branch_curve(b, collection) for b in data["branches"]]
    site_spheres(data["sites"], collection)
    bpy.context.view_layer.update()

    lo, hi = bounds(curves)
    target = (lo + hi) * 0.5
    size = max((hi - lo).length, 1.0)
    distance = size * 1.35

    views = [
        ("skeleton.png", math.radians(-60), math.radians(12)),
        ("skeleton-side.png", math.radians(-90), math.radians(3)),
    ]
    for filename, azimuth, elevation in views:
        cam = add_camera(scene, target, distance, azimuth, elevation, filename)
        scene.camera = cam
        scene.render.filepath = str(out_dir / filename)
        bpy.ops.render.render(write_still=True)
        print(f"wrote {scene.render.filepath}")


main()
