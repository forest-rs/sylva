# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Reproducible, unclipped branch join review. Run with --help for selection.

cargo run --release -p species_gallery -- --bark-only --welded --species oak --seeds 1 OUTPUT
blender --background --python examples/species_gallery/tools/render_forks.py -- OUTPUT/oak-seed1 --list

Each image compares embedded (left) and welded (right), clay (top) and wire
(bottom). Only the selected child and its parent are shown. No near-plane
cutaways: open bases at the ends of these isolated branches are intentional.
The JSON sidecar records identities, outcomes, camera, input hashes and Blender
version. Use the same branch ID, azimuth and framing for before/after captures.
"""

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Quaternion, Vector


def args():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--list", action="store_true", help="Print fork records as JSON without rendering")
    parser.add_argument("--branch", action="append", default=[], help="Stable hexadecimal branch ID (repeatable)")
    parser.add_argument("--azimuth", type=float, default=0, help="Degrees around the parent axis (default: 0)")
    parser.add_argument("--scale", type=float, default=8, help="Image width in parent radii (default: 8)")
    parser.add_argument("--size", type=int, default=512, help="Pixels per panel (default: 512)")
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    result = parser.parse_args(argv)
    if not math.isfinite(result.azimuth) or not math.isfinite(result.scale) or result.scale <= 0 or result.size < 64:
        parser.error("azimuth must be finite, scale positive and finite, and size at least 64")
    return result


def material(wire=False):
    mat = bpy.data.materials.new("wire" if wire else "clay")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (0.45, 0.32, 0.20, 1)
    bsdf.inputs["Roughness"].default_value = 0.8
    if wire:
        node = mat.node_tree.nodes.new("ShaderNodeWireframe")
        node.use_pixel_size = True
        node.inputs["Size"].default_value = 1
        mix = mat.node_tree.nodes.new("ShaderNodeMixRGB")
        mix.inputs[1].default_value = (0.6, 0.6, 0.6, 1)
        mix.inputs[2].default_value = (0.03, 0.03, 0.03, 1)
        mat.node_tree.links.new(node.outputs["Fac"], mix.inputs[0])
        mat.node_tree.links.new(mix.outputs[0], bsdf.inputs["Base Color"])
    return mat


def read_bark(path):
    """Read the gallery's indexed positions/normals and branch face groups."""
    vertices, normals, groups = [], [], {}
    group = None
    with path.open() as source:
        for line in source:
            fields = line.split()
            if not fields:
                continue
            if fields[0] == "v":
                vertices.append(tuple(map(float, fields[1:])))
            elif fields[0] == "vn":
                normals.append(tuple(map(float, fields[1:])))
            elif fields[0] == "g":
                group = int(fields[1].removeprefix("branch-"))
            elif fields[0] == "f":
                if group is None:
                    raise ValueError(f"{path}: missing branch groups; regenerate with species_gallery --bark-only --welded")
                groups.setdefault(group, []).append(tuple(int(f.split('/')[0]) - 1 for f in fields[1:]))
    return vertices, normals, groups


def isolate(name, data, fork, mat):
    positions, normals, groups = data
    faces = groups.get(fork["parent"], []) + groups.get(fork["branch"], [])
    indices = sorted({i for face in faces for i in face})
    local = {index: i for i, index in enumerate(indices)}
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata([positions[i] for i in indices], [], [tuple(local[i] for i in f) for f in faces])
    mesh.update()
    for polygon in mesh.polygons:
        polygon.use_smooth = True
    mesh.normals_split_custom_set_from_vertices([normals[i] for i in indices])
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    mesh.materials.append(mat)
    return obj


def compose(panels, path, size):
    width = size * 2
    out = bpy.data.images.new(path.name, width, width)
    pixels = [0.0] * (width * width * 4)
    for index, panel in enumerate(panels):
        image = bpy.data.images.load(str(panel))
        src = list(image.pixels)
        col, row = index % 2, 1 - index // 2
        for y in range(size):
            start = ((row * size + y) * width + col * size) * 4
            pixels[start:start + size * 4] = src[y * size * 4:(y + 1) * size * 4]
        bpy.data.images.remove(image)
        panel.unlink()
    out.pixels = pixels
    out.filepath_raw = str(path)
    out.file_format = "PNG"
    out.save()
    bpy.data.images.remove(out)


def main():
    opts = args()
    directory = opts.directory.resolve()
    report = json.loads((directory / "forks.json").read_text())
    forks = sorted(report["forks"], key=lambda f: -f["parent_radius"])
    if opts.branch:
        missing = set(opts.branch) - {f["id"] for f in forks}
        if missing:
            raise ValueError(f"unknown branch IDs: {sorted(missing)}; use --list")
        chosen = [f for f in forks if f["id"] in opts.branch]
    elif opts.list:
        chosen = forks
    else:
        chosen = [f for f in forks if f["outcome"] == "welded"][:2]
        chosen += [f for f in forks if f["outcome"] == "refused"][:1]
        chosen += [f for f in forks if f["outcome"] == "excluded"][:2]
    if opts.list:
        print(json.dumps(chosen, indent=2))
        return
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_EEVEE"
    scene.render.resolution_x = scene.render.resolution_y = opts.size
    scene.view_settings.view_transform = "AgX"
    world = bpy.data.worlds.new("world")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.55, 0.62, 0.72, 1)
    world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.6
    scene.world = world
    sun = bpy.data.lights.new("sun", "SUN")
    sun.energy = 3
    sun_obj = bpy.data.objects.new("sun", sun)
    sun_obj.rotation_euler = (math.radians(40), 0, math.radians(-30))
    scene.collection.objects.link(sun_obj)
    lit, wire = material(), material(True)
    inputs = [directory / "forks.json", directory / "bark.obj", directory / "bark-welded.obj", Path(__file__)]
    inputs += [p for p in (directory / "capture.json", directory / "species.ron") if p.exists()]
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in inputs}
    data = {name: read_bark(directory / file) for name, file in [("embedded", "bark.obj"), ("welded", "bark-welded.obj")]}
    target = directory / "forks"
    target.mkdir(exist_ok=True)
    for fork in chosen:
        meshes = [isolate(name, mesh, fork, lit) for name, mesh in data.items()]
        center = Vector(fork["center"])
        along = Vector(fork["parent_tangent"]).normalized()
        child = Vector(fork["child_tangent"]).normalized()
        side = along.cross(child)
        if side.length < 1e-3:
            side = along.orthogonal()
        direction = Quaternion(along, math.radians(opts.azimuth)) @ (side.normalized() + 0.25 * along).normalized()
        radius = fork["parent_radius"]
        camera_data = bpy.data.cameras.new("fork")
        camera_data.type = "ORTHO"
        camera_data.ortho_scale = opts.scale * radius
        camera_data.clip_start = max(radius * 0.001, 0.00001)
        # Place the camera beyond every retained vertex: no foreground cuts.
        distance = max((Vector(v) - center).length for mesh in meshes for v in [vertex.co for vertex in mesh.data.vertices]) + radius
        camera_data.clip_end = 2 * distance + radius
        camera = bpy.data.objects.new("fork", camera_data)
        scene.collection.objects.link(camera)
        camera.location = center + direction * distance
        camera.rotation_euler = (center - camera.location).to_track_quat("-Z", "Y").to_euler()
        scene.camera = camera
        panels = []
        for mat in (lit, wire):
            for obj in meshes:
                for other in meshes:
                    other.hide_render = other != obj
                obj.data.materials[0] = mat
                panel = target / f"panel-{len(panels)}.png"
                scene.render.filepath = str(panel)
                bpy.ops.render.render(write_still=True)
                panels.append(panel)
        path = target / f"fork-{fork['id']}-az{opts.azimuth:g}.png"
        compose(panels, path, opts.size)
        metadata = {"fork": fork, "panels": ["embedded clay", "welded clay", "embedded wire", "welded wire"], "isolated_branches": [fork["parent_id"], fork["id"]], "azimuth_degrees": opts.azimuth, "width_parent_radii": opts.scale, "camera_position": list(camera.location), "camera_target": list(center), "clip_metres": [camera_data.clip_start, camera_data.clip_end], "panel_pixels": opts.size, "blender": bpy.app.version_string, "input_sha256": hashes}
        path.with_suffix(".json").write_text(json.dumps(metadata, indent=2) + "\n")
        print(f"wrote {path} ({fork['outcome']}: {fork['reason']})")
        for obj in meshes:
            mesh = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            bpy.data.meshes.remove(mesh)
        bpy.data.objects.remove(camera, do_unlink=True)
        bpy.data.cameras.remove(camera_data)


main()
