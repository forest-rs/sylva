# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render a species_gallery bark mesh with a UV grid in headless Blender.

    blender --background --python examples/species_gallery/tools/render_bark.py -- .local/gallery/species-gallery/oak-seed1

Imports `bark.obj` unchanged (sylva is Z-up, like Blender), keeps its
authored normals, and textures it with Blender's generated UV grid so bark
seams, wrap counts and texel density are visible. `bark.obj` stores `1 - v`
so that sylva's row-indexed UVs sample dapple's textures correctly in Blender;
Blender's own grid therefore reads mirrored, which is expected. Writes `bark.png` (whole
tree), `bark-base.png` (trunk base and root flare) and `bark-fork.png` (the
first limb junctions) next to the input.
"""

import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if not argv:
        sys.exit("usage: render_bark.py -- <species_gallery seed dir>")
    return Path(argv[0])


def reset_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    shading = scene.display.shading
    shading.light = "STUDIO"
    shading.color_type = "TEXTURE"
    shading.show_cavity = False
    shading.show_shadows = False
    scene.render.resolution_x = 1200
    scene.render.resolution_y = 1400
    world = bpy.data.worlds.new("world")
    world.color = (0.93, 0.93, 0.91)
    scene.world = world
    return scene


def grid_material():
    image = bpy.data.images.new("uv_grid", 1024, 1024)
    image.generated_type = "COLOR_GRID"
    material = bpy.data.materials.new("bark_grid")
    material.use_nodes = True
    nodes = material.node_tree.nodes
    tex = nodes.new("ShaderNodeTexImage")
    tex.image = image
    tex.interpolation = "Closest"
    bsdf = nodes["Principled BSDF"]
    material.node_tree.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    return material


def add_camera(scene, target, distance, azimuth, elevation, name, lens=50):
    direction = Vector(
        (
            math.cos(elevation) * math.cos(azimuth),
            math.cos(elevation) * math.sin(azimuth),
            math.sin(elevation),
        )
    )
    cam_data = bpy.data.cameras.new(name)
    cam_data.lens = lens
    cam_data.clip_start = 0.01
    cam = bpy.data.objects.new(name, cam_data)
    scene.collection.objects.link(cam)
    cam.location = target + direction * distance
    cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
    return cam


def main():
    out_dir = args()
    scene = reset_scene()
    bpy.ops.wm.obj_import(
        filepath=str(out_dir / "bark.obj"), forward_axis="Y", up_axis="Z"
    )
    bark = bpy.context.selected_objects[0]
    bark.data.materials.clear()
    bark.data.materials.append(grid_material())
    bpy.context.view_layer.update()

    lo = Vector((math.inf,) * 3)
    hi = Vector((-math.inf,) * 3)
    for corner in bark.bound_box:
        world = bark.matrix_world @ Vector(corner)
        lo = Vector(map(min, lo, world))
        hi = Vector(map(max, hi, world))
    center = (lo + hi) * 0.5
    size = max((hi - lo).length, 1.0)

    views = [
        ("bark.png", center, size * 1.35, math.radians(-60), math.radians(12), 50),
        ("bark-base.png", Vector((0.0, 0.0, 0.6)), 3.2, math.radians(-60), math.radians(15), 50),
        ("bark-fork.png", Vector((0.0, 0.0, 2.6)), 3.0, math.radians(-40), math.radians(20), 50),
    ]
    for filename, target, distance, azimuth, elevation, lens in views:
        cam = add_camera(scene, target, distance, azimuth, elevation, filename, lens)
        scene.camera = cam
        scene.render.filepath = str(out_dir / filename)
        bpy.ops.render.render(write_still=True)
        print(f"wrote {scene.render.filepath}")


main()
