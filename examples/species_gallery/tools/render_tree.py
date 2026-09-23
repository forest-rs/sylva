# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render a species_gallery tree, bark and leaves, in headless Blender.

    blender --background --python examples/species_gallery/tools/render_tree.py -- target/species-gallery/oak-seed1

Imports `bark.obj` and `leaves.obj` unchanged (sylva is Z-up, like Blender)
and renders them two-sided. When the species' generated textures exist
(`../textures/<species>-bark/gltf` and `-leaf/gltf`), bark and leaves show
their base colour; otherwise they are flat brown and green. Writes `tree.png`
(whole tree), `tree-leaves.png` (the crown edge) and `tree-bark.png` (the
trunk base) next to the input.
"""

import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector

BARK = (0.33, 0.24, 0.16, 1.0)
LEAF = (0.22, 0.42, 0.14, 1.0)


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if not argv:
        sys.exit("usage: render_tree.py -- <species_gallery seed dir>")
    return Path(argv[0])


def reset_scene(textured):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    shading = scene.display.shading
    shading.light = "STUDIO"
    shading.color_type = "TEXTURE" if textured else "OBJECT"
    shading.show_cavity = True
    shading.show_shadows = True
    shading.show_backface_culling = False
    scene.render.resolution_x = 1200
    scene.render.resolution_y = 1400
    world = bpy.data.worlds.new("world")
    world.color = (0.93, 0.93, 0.91)
    scene.world = world
    return scene


def textured_material(name, image_path):
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    nodes = material.node_tree.nodes
    tex = nodes.new("ShaderNodeTexImage")
    tex.image = bpy.data.images.load(str(image_path))
    bsdf = nodes["Principled BSDF"]
    material.node_tree.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    nodes.active = tex
    return material


def load(path, color, texture):
    bpy.ops.wm.obj_import(filepath=str(path), forward_axis="Y", up_axis="Z")
    obj = bpy.context.selected_objects[0]
    obj.color = color
    if texture is not None and texture.exists():
        obj.data.materials.clear()
        obj.data.materials.append(textured_material(path.stem, texture))
    return obj


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
    cam_data.clip_start = 0.01
    cam = bpy.data.objects.new(name, cam_data)
    scene.collection.objects.link(cam)
    cam.location = target + direction * distance
    cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
    return cam


def main():
    out_dir = args()
    species = out_dir.name.split("-seed")[0]
    textures = out_dir.parent / "textures"
    bark_tex = textures / f"{species}-bark" / "gltf" / "base_color.png"
    leaf_tex = textures / f"{species}-leaf" / "gltf" / "base_color.png"
    scene = reset_scene(bark_tex.exists())
    objects = [load(out_dir / "bark.obj", BARK, bark_tex)]
    leaves_path = out_dir / "leaves.obj"
    if leaves_path.exists():
        objects.append(load(leaves_path, LEAF, leaf_tex))
    bpy.context.view_layer.update()
    lo, hi = bounds(objects)
    center = (lo + hi) * 0.5
    size = max((hi - lo).length, 1.0)
    edge = Vector((center.x + 0.35 * (hi.x - lo.x), center.y, center.z + 0.1 * (hi.z - lo.z)))
    views = [
        ("tree.png", center, size * 1.3, math.radians(-60), math.radians(10)),
        ("tree-leaves.png", edge, 1.2, math.radians(-20), math.radians(15)),
        ("tree-bark.png", Vector((0.0, 0.0, 1.2)), 2.2, math.radians(-60), math.radians(8)),
    ]
    for filename, target, distance, az, elevation in views:
        cam = add_camera(scene, target, distance, az, elevation, filename)
        scene.camera = cam
        scene.render.filepath = str(out_dir / filename)
        bpy.ops.render.render(write_still=True)
        print(f"wrote {scene.render.filepath}")


main()
