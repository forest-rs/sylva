# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render major forks close up, embedded against welded, in headless Blender.

    cargo run --release -p species_gallery -- --welded .local/gallery/species-gallery
    blender --background --python examples/species_gallery/tools/render_forks.py -- .local/gallery/species-gallery/oak-seed1

Reads `forks.json`, `bark.obj` and `bark-welded.obj` from a species_gallery
seed directory written with `--welded`. For the largest welded forks (up to
four) and the largest refused one, writes `forks/fork-<branch>.png`: four
panels, the embedded bark and the welded bark (left and right), each
textured under a sun and as a wireframe (top and bottom). The camera looks
across the plane of the fork and clips geometry in front of it. Textures come from `../textures/<species>-bark/gltf`
when present.
"""

import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector

BARK = (0.33, 0.24, 0.16, 1.0)
PANEL = 700


def args():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    if not argv:
        sys.exit("usage: render_forks.py -- <species_gallery seed dir>")
    return Path(argv[0])


def reset_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x = PANEL
    scene.render.resolution_y = PANEL
    scene.view_settings.view_transform = "AgX"
    world = bpy.data.worlds.new("world")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.55, 0.62, 0.72, 1.0)
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.6
    scene.world = world
    sun = bpy.data.lights.new("sun", "SUN")
    sun.energy = 4.0
    sun.angle = math.radians(3.0)
    sun_obj = bpy.data.objects.new("sun", sun)
    sun_obj.rotation_euler = (math.radians(40), 0.0, math.radians(-30))
    scene.collection.objects.link(sun_obj)
    return scene


def bark_material(textures):
    material = bpy.data.materials.new("bark")
    material.use_nodes = True
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    bsdf = nodes["Principled BSDF"]
    bsdf.inputs["Roughness"].default_value = 0.85
    base = textures / "base_color.png"
    if base.exists():
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = bpy.data.images.load(str(base))
        links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    else:
        bsdf.inputs["Base Color"].default_value = BARK
    normal = textures / "normal.png"
    if normal.exists():
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = bpy.data.images.load(str(normal))
        tex.image.colorspace_settings.name = "Non-Color"
        # `bark.obj` stores `1 - v`, so the map's green channel is flipped.
        split = nodes.new("ShaderNodeSeparateColor")
        invert = nodes.new("ShaderNodeMath")
        invert.operation = "SUBTRACT"
        invert.inputs[0].default_value = 1.0
        join = nodes.new("ShaderNodeCombineColor")
        links.new(tex.outputs["Color"], split.inputs["Color"])
        links.new(split.outputs["Red"], join.inputs["Red"])
        links.new(split.outputs["Green"], invert.inputs[1])
        links.new(invert.outputs["Value"], join.inputs["Green"])
        links.new(split.outputs["Blue"], join.inputs["Blue"])
        map_node = nodes.new("ShaderNodeNormalMap")
        links.new(join.outputs["Color"], map_node.inputs["Color"])
        links.new(map_node.outputs["Normal"], bsdf.inputs["Normal"])
    return material


def wire_material():
    material = bpy.data.materials.new("wire")
    material.use_nodes = True
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    bsdf = nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (0.75, 0.72, 0.68, 1.0)
    wire = nodes.new("ShaderNodeWireframe")
    wire.use_pixel_size = True
    wire.inputs["Size"].default_value = 1.2
    mix = nodes.new("ShaderNodeMixRGB")
    mix.inputs["Color1"].default_value = (0.75, 0.72, 0.68, 1.0)
    mix.inputs["Color2"].default_value = (0.05, 0.05, 0.05, 1.0)
    links.new(wire.outputs["Fac"], mix.inputs["Fac"])
    links.new(mix.outputs["Color"], bsdf.inputs["Base Color"])
    return material


def load(path, material):
    bpy.ops.wm.obj_import(filepath=str(path), forward_axis="Y", up_axis="Z")
    obj = bpy.context.selected_objects[0]
    obj.data.materials.clear()
    obj.data.materials.append(material)
    return obj


def camera(scene, fork):
    center = Vector(fork["center"])
    along = Vector(fork["parent_tangent"]).normalized()
    child = Vector(fork["child_tangent"]).normalized()
    # Look across the fork's plane, from the side where the crotch between
    # the parent's upper piece and the child opens, a little above it.
    side = along.cross(child)
    if side.length < 1e-3:
        side = along.orthogonal()
    side.normalize()
    direction = (side + 0.35 * (along + child).normalized()).normalized()
    distance = 7.0 * fork["parent_radius"]
    data = bpy.data.cameras.new("fork")
    data.lens = 50
    # Clip what lies between the camera and the fork, such as other limbs.
    data.clip_start = 0.6 * distance
    data.clip_end = 3.0 * distance
    cam = bpy.data.objects.new("fork", data)
    scene.collection.objects.link(cam)
    cam.location = center + direction * distance
    cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
    return cam


def compose(panels, path):
    """Tiles 2x2 panels (row-major) into one image."""
    width = PANEL * 2
    out = bpy.data.images.new(path.name, width, width)
    pixels = [0.0] * (width * width * 4)
    for index, panel in enumerate(panels):
        image = bpy.data.images.load(str(panel))
        src = list(image.pixels)
        col, row = index % 2, 1 - index // 2
        for y in range(PANEL):
            start = ((row * PANEL + y) * width + col * PANEL) * 4
            pixels[start : start + PANEL * 4] = src[y * PANEL * 4 : (y + 1) * PANEL * 4]
        bpy.data.images.remove(image)
        panel.unlink()
    out.pixels = pixels
    out.filepath_raw = str(path)
    out.file_format = "PNG"
    out.save()


def main():
    out_dir = args()
    species = out_dir.name.split("-seed")[0]
    textures = out_dir.parent / "textures" / f"{species}-bark" / "gltf"
    report = json.loads((out_dir / "forks.json").read_text())
    forks = sorted(report["forks"], key=lambda f: -f["parent_radius"])
    chosen = [f for f in forks if f["welded"]][:4] + [f for f in forks if not f["welded"]][:1]
    scene = reset_scene()
    lit = bark_material(textures)
    wire = wire_material()
    meshes = {
        "embedded": load(out_dir / "bark.obj", lit),
        "welded": load(out_dir / "bark-welded.obj", lit),
    }
    target = out_dir / "forks"
    target.mkdir(exist_ok=True)
    for fork in chosen:
        scene.camera = camera(scene, fork)
        panels = []
        for material in (lit, wire):
            for name, obj in meshes.items():
                for other in meshes.values():
                    other.hide_render = other is not obj
                obj.data.materials[0] = material
                panel = target / f"panel-{len(panels)}.png"
                scene.render.filepath = str(panel)
                bpy.ops.render.render(write_still=True)
                panels.append(panel)
        path = target / f"fork-{fork['branch']}.png"
        compose(panels, path)
        state = "welded" if fork["welded"] else "refused"
        print(f"wrote {path} ({state}, parent radius {fork['parent_radius']:.3f} m)")


main()
