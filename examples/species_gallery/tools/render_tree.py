# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Render a species_gallery tree, bark and leaves, in headless Blender.

    blender --background --python examples/species_gallery/tools/render_tree.py -- .local/gallery/species-gallery/oak-seed1

Imports `bark.obj` and `leaves.obj` unchanged (sylva is Z-up, like Blender)
and renders them two-sided. When the species' generated textures exist
(`../textures/<species>-bark/gltf` and `-leaf/gltf`), bark and leaves show
their base colour; otherwise they are flat brown and green. Writes `tree.png`
(whole tree), `tree-leaves.png` (the crown edge) and `tree-bark.png` (the
trunk base) next to the input, all in Workbench for geometry review, then
lit renders of the whole tree in EEVEE under a sun and sky: `tree-lit.png`
(sun behind the camera) and `tree-backlit.png` (sun behind the tree), each
with translucent leaves, and `tree-lit-opaque.png` and
`tree-backlit-opaque.png` without, for comparison. `tree-canopy.png` and
`tree-canopy-opaque.png` look up into the crown from beneath its edge
toward a low sun under a dim sky, where translucency matters most: lit
leaves glow against the shaded ones in front. Translucency comes from
the leaf set's `diffuse_transmission.png` (tint RGB, weight A), the texture
the GLBs bind through `KHR_materials_diffuse_transmission`; without it,
leaves transmit 35% tinted by their base colour.
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
    transmission = leaf_tex.with_name("diffuse_transmission.png")
    render_lit(scene, objects, out_dir, center, size, transmission)


def render_lit(scene, objects, out_dir, center, size, transmission):
    """The whole tree in EEVEE, front- and back-lit, with and without leaf
    translucency."""
    scene.render.engine = "BLENDER_EEVEE"
    world = scene.world
    world.use_nodes = True
    background = world.node_tree.nodes["Background"]
    background.inputs[0].default_value = (0.62, 0.72, 0.85, 1.0)
    background.inputs[1].default_value = 0.8
    sun = bpy.data.lights.new("sun", "SUN")
    sun.energy = 4.0
    sun_obj = bpy.data.objects.new("sun", sun)
    scene.collection.objects.link(sun_obj)
    mixes = []
    if len(objects) > 1:
        for material in objects[1].data.materials:
            mix = translucent(material, transmission)
            if mix is not None:
                mixes.append(mix)
    cam = add_camera(scene, center, size * 1.3, math.radians(-60), math.radians(10), "lit")
    scene.camera = cam
    view = (cam.location - center).normalized()
    front = (math.radians(40), 0.0, math.radians(-30))
    # Light travelling from behind the tree toward the camera, from above.
    travel = Vector((view.x, view.y, view.z - 0.8)).normalized()
    back = (-travel).to_track_quat("Z", "Y").to_euler()
    shots = [("tree-lit", cam, front, 0.8, 4.0), ("tree-backlit", cam, back, 0.8, 4.0)]
    # Beneath the crown's edge, looking up through it into a low sun whose
    # light passes through the leaves toward the camera; a dim sky keeps
    # ambient light from filling the shaded leaves.
    canopy = add_camera(
        scene, center + Vector((0.0, 0.0, size * 0.1)), size * 0.55,
        math.radians(-60), math.radians(-25), "canopy",
    )
    canopy.data.lens = 28
    up = (center - canopy.location).normalized()
    sun_travel = Vector((-up.x, -up.y, -0.35)).normalized()
    low = (-sun_travel).to_track_quat("Z", "Y").to_euler()
    # A stronger sun: only light passing through the outer leaves reaches us.
    shots.append(("tree-canopy", canopy, low, 0.25, 10.0))
    for name, camera, rotation, sky, energy in shots:
        scene.camera = camera
        sun_obj.rotation_euler = rotation
        sun.energy = energy
        background.inputs[1].default_value = sky
        for suffix, on in (("", True), ("-opaque", False)):
            for mix, weight in mixes:
                set_translucency(mix, weight, on)
            scene.render.filepath = str(out_dir / f"{name}{suffix}.png")
            bpy.ops.render.render(write_still=True)
            print(f"wrote {scene.render.filepath}")


def translucent(material, transmission):
    """Mixes a translucent lobe into a leaf: tint and weight from the
    diffuse-transmission texture when it exists, else 35% tinted by the base
    colour. Returns the mix node and its weight source."""
    if material is None or not material.use_nodes:
        return None
    nodes = material.node_tree.nodes
    links = material.node_tree.links
    bsdf = nodes["Principled BSDF"]
    # Leaves are waxy but not glossy; the default 0.5 roughness and full
    # specular turn sunlit leaves white at grazing angles.
    bsdf.inputs["Roughness"].default_value = 0.65
    bsdf.inputs["Specular IOR Level"].default_value = 0.25
    output = nodes["Material Output"]
    back = nodes.new("ShaderNodeBsdfTranslucent")
    mix = nodes.new("ShaderNodeMixShader")
    weight = None
    if transmission.exists():
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = bpy.data.images.load(str(transmission))
        # The weight is linear data in alpha; keep it unassociated.
        tex.image.alpha_mode = "CHANNEL_PACKED"
        links.new(tex.outputs["Color"], back.inputs["Color"])
        weight = tex.outputs["Alpha"]
    else:
        color = bsdf.inputs["Base Color"]
        if color.links:
            links.new(color.links[0].from_socket, back.inputs["Color"])
        else:
            back.inputs["Color"].default_value = color.default_value
        mix.inputs["Fac"].default_value = 0.35
    links.new(bsdf.outputs["BSDF"], mix.inputs[1])
    links.new(back.outputs["BSDF"], mix.inputs[2])
    links.new(mix.outputs["Shader"], output.inputs["Surface"])
    return mix, weight


def set_translucency(mix, weight, on):
    """Drives the mix by the texture's weight, or turns the lobe off."""
    links = mix.id_data.links
    for link in list(mix.inputs["Fac"].links):
        links.remove(link)
    if on and weight is not None:
        links.new(weight, mix.inputs["Fac"])
    elif not on:
        mix.inputs["Fac"].default_value = 0.0
    else:
        mix.inputs["Fac"].default_value = 0.35


main()
