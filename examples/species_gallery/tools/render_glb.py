# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render a species_gallery tree's exported GLB levels in headless Blender.

    blender --background --python examples/species_gallery/tools/render_glb.py -- .local/gallery/species-gallery/oak-seed1

Imports every `glb/lod<n>.glb` with Blender's glTF importer, unchanged, so
materials, textures, alpha modes and normals are exactly what the files
carry. Places the levels in a row and renders them in EEVEE under a sun and
sky to `glb.png` next to the input. With `--instanced` after the directory,
uses each level's `lod<n>-instanced.glb` where one exists and writes
`glb-instanced.png`, for comparing instanced leaves with merged ones.
"""

import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if not argv:
        sys.exit("usage: render_glb.py -- <species_gallery seed dir>")
    return Path(argv[0]), "--instanced" in argv[1:]


def main():
    out_dir, instanced = args()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x = 3000
    scene.render.resolution_y = 800
    world = bpy.data.worlds.new("world")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.62, 0.72, 0.85, 1.0)
    world.node_tree.nodes["Background"].inputs[1].default_value = 0.8
    scene.world = world
    sun = bpy.data.lights.new("sun", "SUN")
    sun.energy = 4.0
    sun_obj = bpy.data.objects.new("sun", sun)
    sun_obj.rotation_euler = (math.radians(40), 0.0, math.radians(-30))
    scene.collection.objects.link(sun_obj)

    levels = sorted(
        (out_dir / "glb").glob("lod*.glb"), key=lambda p: int(p.stem[3:].split("-")[0])
    )
    levels = [p for p in levels if not p.stem.endswith("-instanced")]
    if instanced:
        levels = [
            p.with_name(f"{p.stem}-instanced.glb")
            if p.with_name(f"{p.stem}-instanced.glb").exists()
            else p
            for p in levels
        ]
    spacing = 16.0
    for n, path in enumerate(levels):
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=str(path))
        for obj in set(bpy.data.objects) - before:
            if obj.parent is None:
                obj.location.x += n * spacing
    width = (len(levels) - 1) * spacing
    target = Vector((width / 2, 0.0, 6.0))
    cam_data = bpy.data.cameras.new("cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = width + spacing
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    cam.location = target + Vector((0.0, -60.0, 4.0))
    cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.camera = cam
    scene.render.filepath = str(out_dir / ("glb-instanced.png" if instanced else "glb.png"))
    bpy.ops.render.render(write_still=True)
    print(f"wrote {scene.render.filepath}")


main()
