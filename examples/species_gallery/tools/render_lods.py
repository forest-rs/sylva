# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render a species_gallery LOD chain side by side in headless Blender.

    blender --background --python examples/species_gallery/tools/render_lods.py -- target/species-gallery/oak-seed1

Loads `lods/lod<n>-bark.obj`, `lods/lod<n>-leaves.obj` and, where the level
has them, `lods/lod<n>-cards.obj` for every level, then `lods/impostor.obj`,
places them in a row, and textures them with the species' generated sets and
the baked atlases; leaf cards, cluster cards and the impostor are
alpha-tested with their texture's opacity. Writes
`lods.png` (full size) and `lods-small.png` (the same view at 1/6 scale,
roughly the screen sizes the coarse levels are meant for) next to the input.
"""

import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if not argv:
        sys.exit("usage: render_lods.py -- <species_gallery seed dir>")
    return Path(argv[0])


def material(name, image_path, alpha):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nodes = mat.node_tree.nodes
    tex = nodes.new("ShaderNodeTexImage")
    tex.image = bpy.data.images.load(str(image_path))
    bsdf = nodes["Principled BSDF"]
    mat.node_tree.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    if alpha:
        mat.node_tree.links.new(tex.outputs["Alpha"], bsdf.inputs["Alpha"])
        mat.surface_render_method = "DITHERED"
    nodes.active = tex
    return mat


def main():
    out_dir = args()
    species = out_dir.name.split("-seed")[0]
    textures = out_dir.parent / "textures"
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x = 3000
    scene.render.resolution_y = 800
    scene.render.film_transparent = False
    world = bpy.data.worlds.new("world")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs[0].default_value = (0.8, 0.84, 0.88, 1.0)
    world.node_tree.nodes["Background"].inputs[1].default_value = 1.0
    scene.world = world
    sun = bpy.data.lights.new("sun", "SUN")
    sun.energy = 3.0
    sun_obj = bpy.data.objects.new("sun", sun)
    sun_obj.rotation_euler = (math.radians(50), 0.0, math.radians(30))
    scene.collection.objects.link(sun_obj)

    bark_mat = material("bark", textures / f"{species}-bark" / "gltf" / "base_color.png", False)
    leaf_mat = material("leaf", textures / f"{species}-leaf" / "gltf" / "base_color.png", True)
    lods = out_dir / "lods"
    levels = sorted(lods.glob("lod*-bark.obj"))
    columns = []
    for n, bark_path in enumerate(levels):
        parts = [(bark_path, bark_mat), (bark_path.with_name(f"lod{n}-leaves.obj"), leaf_mat)]
        cards = lods / f"lod{n}-cards.obj"
        if cards.exists():
            parts.append((cards, material(f"cards{n}", lods / f"lod{n}-cards" / "base_color.png", True)))
        columns.append(parts)
    impostor = lods / "impostor.obj"
    if impostor.exists():
        columns.append([(impostor, material("impostor", lods / "impostor" / "base_color.png", True))])
    spacing = 16.0
    for n, parts in enumerate(columns):
        for path, mat in parts:
            bpy.ops.wm.obj_import(filepath=str(path), forward_axis="Y", up_axis="Z")
            if not bpy.context.selected_objects:
                continue  # an empty part, such as a clustered level's leaves
            obj = bpy.context.selected_objects[0]
            obj.data.materials.clear()
            obj.data.materials.append(mat)
            obj.location.x = n * spacing
    width = (len(columns) - 1) * spacing
    target = Vector((width / 2, 0.0, 6.0))
    cam_data = bpy.data.cameras.new("cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = width + spacing
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    cam.location = target + Vector((0.0, -60.0, 4.0))
    cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.camera = cam
    for filename, scale in [("lods.png", 100), ("lods-small.png", 17)]:
        scene.render.resolution_percentage = scale
        scene.render.filepath = str(out_dir / filename)
        bpy.ops.render.render(write_still=True)
        print(f"wrote {scene.render.filepath}")


main()
