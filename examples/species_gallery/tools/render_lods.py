# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render a species_gallery LOD chain side by side in headless Blender.

    blender --background --python examples/species_gallery/tools/render_lods.py -- .local/gallery/species-gallery/oak-seed1

Loads `lods/lod<n>-bark.obj`, `lods/lod<n>-leaves.obj` and, where the level
has them, `lods/lod<n>-cards.obj` for every level, then `lods/impostor.obj`,
places them in a row, and textures them with the species' generated sets and
the baked atlases; leaf cards, cluster cards and the impostor are
alpha-tested with their texture's opacity. When `lods/octahedral.json`
exists, a last column draws the octahedral impostor the way a renderer
would: one quad facing the camera, sampling the frame nearest the view
direction. Writes
`lods.png` (full size) and `lods-small.png` (the same view at 1/6 scale,
roughly the screen sizes the coarse levels are meant for) next to the input.
"""

import json
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


def hemi_octahedral_encode(d):
    total = abs(d.x) + abs(d.y) + max(d.z, 0.0)
    px, py = (d.x / total, d.y / total) if total > 0 else (0.0, 0.0)
    return ((px + py + 1.0) * 0.5, (px - py + 1.0) * 0.5)


def add_octahedral(lods, octa, x, camera_offset):
    """One camera-facing quad over the frame nearest the orthographic view."""
    frames = octa["frames"]
    toward = camera_offset.normalized()
    toward.z = max(toward.z, 0.0)
    u, v = hemi_octahedral_encode(toward.normalized())
    col = min(int(u * frames), frames - 1)
    row = min(int(v * frames), frames - 1)
    u0, u1 = col / frames, (col + 1) / frames
    # Atlas rows fill from v = 0 at the image top; Blender's v is up.
    v0, v1 = 1.0 - (row + 1) / frames, 1.0 - row / frames
    r = octa["radius"]
    cx, cy, cz = octa["center"]
    # The frame's axes: up is +Z made perpendicular to the view.
    up = (Vector((0.0, 0.0, 1.0)) - toward * toward.z).normalized()
    right = up.cross(toward)
    center = Vector((cx + x, cy, cz))
    corners = [center + right * (sx * r) + up * (sy * r) for sx, sy in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
    mesh = bpy.data.meshes.new("octahedral")
    mesh.from_pydata(corners, [], [(0, 1, 2, 3)])
    uv = mesh.uv_layers.new()
    for loop, (lu, lv) in zip(uv.data, ((u0, v0), (u1, v0), (u1, v1), (u0, v1))):
        loop.uv = (lu, lv)
    obj = bpy.data.objects.new("octahedral", mesh)
    obj.data.materials.append(material("octahedral", lods / "octahedral" / "base_color.png", True))
    bpy.context.scene.collection.objects.link(obj)


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
    octahedral = lods / "octahedral.json"
    octa = json.loads(octahedral.read_text()) if octahedral.exists() else None
    count = len(columns) + (1 if octa else 0)
    for n, parts in enumerate(columns):
        for path, mat in parts:
            bpy.ops.wm.obj_import(filepath=str(path), forward_axis="Y", up_axis="Z")
            if not bpy.context.selected_objects:
                continue  # an empty part, such as a clustered level's leaves
            obj = bpy.context.selected_objects[0]
            obj.data.materials.clear()
            obj.data.materials.append(mat)
            obj.location.x = n * spacing
    width = (count - 1) * spacing
    target = Vector((width / 2, 0.0, 6.0))
    camera_offset = Vector((0.0, -60.0, 4.0))
    if octa:
        add_octahedral(lods, octa, len(columns) * spacing, camera_offset)
    cam_data = bpy.data.cameras.new("cam")
    cam_data.type = "ORTHO"
    cam_data.ortho_scale = width + spacing
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    cam.location = target + camera_offset
    cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.camera = cam
    for filename, scale in [("lods.png", 100), ("lods-small.png", 17)]:
        scene.render.resolution_percentage = scale
        scene.render.filepath = str(out_dir / filename)
        bpy.ops.render.render(write_still=True)
        print(f"wrote {scene.render.filepath}")


main()
