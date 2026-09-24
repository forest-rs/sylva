# Copyright 2026 the Sylva Authors
# SPDX-License-Identifier: Apache-2.0 OR MIT

"""Beauty renders of species_gallery GLBs in Cycles, the standard review shot.

    cargo run --release -p species_gallery -- --glb-only
    blender --background --python examples/species_gallery/tools/render_beauty.py -- \
        hero .local/gallery/beauty/oak-hero.png

Arguments after `--`: the shot, the output PNG, then options:

- `--gallery DIR`: the species_gallery output (default
  `.local/gallery/species-gallery`);
- `--species NAME` (default `oak`) and `--seed N` for the single-tree shots
  (default 3);
- `--samples N` (default 128, adaptive) and `--scale S`, a factor on the
  1600 x 1200 resolution.

Shots:

- `hero`: the tree three-quarter lit by a low sun behind the camera, over a
  hair-grass meadow with haze and clumps of distant trees;
- `backlit`: the same view with the sun behind the tree, where leaf
  translucency carries the crown;
- `grove`: seeds 3, 5, 2, 1 and 4 together, for variation between seeds;
- `bark`: the trunk base and lower limbs close up.

Everything renders on the Metal GPU with adaptive sampling and OIDN, under
a minute per image at the defaults on an M-series Mac (about 40 s for the
hero). The grove and the background trees need every seed's GLBs
(`--glb-only`); seeds without them are skipped.

Blender's glTF importer drops `KHR_materials_diffuse_transmission`, so leaf
materials are rebuilt: the base colour's Principled BSDF mixed with a
Translucent BSDF by the leaf set's `diffuse_transmission.png` (tint RGB,
weight A), all under an outer alpha cutout so the translucent lobe is cut
too.
"""

import argparse
import math
import random
import sys
import time
from pathlib import Path

import bpy
from mathutils import Vector

parser = argparse.ArgumentParser(prog="render_beauty.py")
parser.add_argument("shot", choices=("hero", "backlit", "grove", "bark"))
parser.add_argument("out", type=Path)
parser.add_argument("--gallery", type=Path, default=Path(".local/gallery/species-gallery"))
parser.add_argument("--species", default="oak")
parser.add_argument("--seed", type=int, default=3)
parser.add_argument("--samples", type=int, default=128)
parser.add_argument("--scale", type=float, default=1.0)
ARGS = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
SHOT, OUT, GEN = ARGS.shot, ARGS.out.resolve(), ARGS.gallery
SAMPLES = ARGS.samples
RES = (int(1600 * ARGS.scale), int(1200 * ARGS.scale))
TRANS = GEN / f"textures/{ARGS.species}-leaf/gltf/diffuse_transmission.png"


def setup_render():
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    prefs = bpy.context.preferences.addons["cycles"].preferences
    prefs.compute_device_type = "METAL"
    prefs.refresh_devices()
    for d in prefs.devices:
        d.use = d.type == "METAL"
    c = scene.cycles
    c.device = "GPU"
    c.samples = SAMPLES
    c.use_adaptive_sampling = True
    c.adaptive_threshold = 0.02
    c.use_denoising = True
    c.denoiser = "OPENIMAGEDENOISE"
    c.max_bounces = 8
    c.diffuse_bounces = 3
    c.glossy_bounces = 2
    c.transmission_bounces = 4
    c.volume_bounces = 1
    c.transparent_max_bounces = 64
    c.sample_clamp_indirect = 8.0
    c.light_sampling_threshold = 0.01
    scene.render.resolution_x, scene.render.resolution_y = RES
    scene.render.resolution_percentage = 100
    scene.view_settings.view_transform = "AgX"
    scene.view_settings.look = "AgX - Medium High Contrast"
    scene.render.image_settings.file_format = "PNG"
    return scene


def fix_materials(objs, done):
    for o in objs:
        for slot in o.material_slots:
            m = slot.material
            if m is None or m.name in done:
                continue
            done.add(m.name)
            nt = m.node_tree
            nodes, links = nt.nodes, nt.links
            bsdf = nodes["Principled BSDF"]
            out = nodes["Material Output"]
            if hasattr(m, "surface_render_method"):
                m.surface_render_method = "DITHERED"
            if not bsdf.inputs["Alpha"].is_linked:
                # Bark: keep as imported.
                bsdf.inputs["Specular IOR Level"].default_value = 0.35
                continue
            # Leaf: alpha cutout via an outer Transparent mix (so the
            # translucent lobe is cut out too), translucency from the
            # diffuse transmission texture (KHR_materials_diffuse_transmission,
            # which the importer drops).
            alpha_src = bsdf.inputs["Alpha"].links[0].from_socket
            links.remove(bsdf.inputs["Alpha"].links[0])
            bsdf.inputs["Alpha"].default_value = 1.0
            bsdf.inputs["Roughness"].default_value = 0.6
            if bsdf.inputs["Roughness"].is_linked:
                links.remove(bsdf.inputs["Roughness"].links[0])
            bsdf.inputs["Specular IOR Level"].default_value = 0.3
            back = nodes.new("ShaderNodeBsdfTranslucent")
            mix = nodes.new("ShaderNodeMixShader")
            tex = nodes.new("ShaderNodeTexImage")
            tex.image = bpy.data.images.get(TRANS.name) or bpy.data.images.load(str(TRANS))
            tex.image.alpha_mode = "CHANNEL_PACKED"
            links.new(tex.outputs["Color"], back.inputs["Color"])
            links.new(tex.outputs["Alpha"], mix.inputs["Fac"])
            links.new(bsdf.outputs["BSDF"], mix.inputs[1])
            links.new(back.outputs["BSDF"], mix.inputs[2])
            cut = nodes.new("ShaderNodeMixShader")
            clear = nodes.new("ShaderNodeBsdfTransparent")
            links.new(alpha_src, cut.inputs["Fac"])
            links.new(clear.outputs["BSDF"], cut.inputs[1])
            links.new(mix.outputs["Shader"], cut.inputs[2])
            links.new(cut.outputs["Shader"], out.inputs["Surface"])


def import_tree(seed, loc=(0, 0, 0), rot=0.0, scale=1.0, done=None):
    path = GEN / f"{ARGS.species}-seed{seed}/glb/lod0.glb"
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(path))
    new = [o for o in bpy.data.objects if o not in before]
    root = bpy.data.objects.new(f"tree{seed}", None)
    bpy.context.scene.collection.objects.link(root)
    for o in new:
        if o.parent is None:
            o.parent = root
    root.location = loc
    root.rotation_euler = (0, 0, rot)
    root.scale = (scale,) * 3
    fix_materials([o for o in new if o.type == "MESH"], done if done is not None else set())
    bpy.context.view_layer.update()
    meshes = [o for o in new if o.type == "MESH"]
    return root, meshes


def bounds(meshes):
    lo = Vector((1e9,) * 3)
    hi = Vector((-1e9,) * 3)
    for o in meshes:
        for c in o.bound_box:
            w = o.matrix_world @ Vector(c)
            lo = Vector(map(min, lo, w))
            hi = Vector(map(max, hi, w))
    return lo, hi


def world_and_sun(scene, elevation, azimuth, sun_energy=4.5, sky_strength=0.35):
    """Sun direction: elevation above horizon, azimuth = direction the light
    comes FROM, degrees counter-clockwise from +X."""
    world = bpy.data.worlds.new("sky")
    world.use_nodes = True
    nt = world.node_tree
    bg = nt.nodes["Background"]
    sky = nt.nodes.new("ShaderNodeTexSky")
    sky.sky_type = "MULTIPLE_SCATTERING"
    sky.sun_disc = True
    sky.sun_size = math.radians(1.2)
    sky.sun_elevation = math.radians(elevation)
    sky.sun_rotation = math.radians(90.0 - azimuth)
    sky.air_density = 1.0
    sky.aerosol_density = 1.5
    sky.altitude = 0.0
    nt.links.new(sky.outputs["Color"], bg.inputs["Color"])
    bg.inputs["Strength"].default_value = sky_strength
    scene.world = world
    # A matching sun lamp for crisp, controllable shadows; the sky's own disc
    # is left in for glints but the lamp does the lighting work.
    sky.sun_intensity = 0.25
    light = bpy.data.lights.new("sun", "SUN")
    light.energy = sun_energy
    light.angle = math.radians(1.5)
    light.color = (1.0, 0.8, 0.6)
    sun = bpy.data.objects.new("sun", light)
    el, az = math.radians(elevation), math.radians(azimuth)
    d = Vector((math.cos(el) * math.cos(az), math.cos(el) * math.sin(az), math.sin(el)))
    sun.rotation_euler = d.to_track_quat("Z", "Y").to_euler()
    scene.collection.objects.link(sun)
    return sky


def ground(scene, size=6000.0):
    bpy.ops.mesh.primitive_plane_add(size=size, location=(0, 0, 0))
    plane = bpy.context.active_object
    plane.name = "ground"
    m = bpy.data.materials.new("meadow")
    m.use_nodes = True
    nt = m.node_tree
    n, l = nt.nodes, nt.links
    bsdf = n["Principled BSDF"]
    bsdf.inputs["Roughness"].default_value = 0.9
    bsdf.inputs["Specular IOR Level"].default_value = 0.2
    coord = n.new("ShaderNodeTexCoord")
    big = n.new("ShaderNodeTexNoise")
    big.inputs["Scale"].default_value = 0.04
    big.inputs["Detail"].default_value = 6
    big.inputs["Roughness"].default_value = 0.6
    mid = n.new("ShaderNodeTexNoise")
    mid.inputs["Scale"].default_value = 0.6
    mid.inputs["Detail"].default_value = 8
    fine = n.new("ShaderNodeTexNoise")
    fine.inputs["Scale"].default_value = 40.0
    fine.inputs["Detail"].default_value = 4
    for t in (big, mid, fine):
        l.new(coord.outputs["Object"], t.inputs["Vector"])
    ramp = n.new("ShaderNodeValToRGB")
    cr = ramp.color_ramp
    cr.elements[0].position = 0.35
    cr.elements[0].color = (0.03, 0.06, 0.014, 1)
    cr.elements[1].position = 0.7
    cr.elements[1].color = (0.11, 0.12, 0.035, 1)
    e = cr.elements.new(0.52)
    e.color = (0.05, 0.09, 0.022, 1)
    mixf = n.new("ShaderNodeMath")
    mixf.operation = "MULTIPLY_ADD"
    l.new(big.outputs["Fac"], mixf.inputs[0])
    mixf.inputs[1].default_value = 0.7
    add2 = n.new("ShaderNodeMath")
    add2.operation = "MULTIPLY_ADD"
    l.new(mid.outputs["Fac"], add2.inputs[0])
    add2.inputs[1].default_value = 0.3
    l.new(add2.outputs[0], mixf.inputs[2])
    l.new(mixf.outputs[0], ramp.inputs["Fac"])
    # Fine speckle darkening, like grass blades in shadow.
    dark = n.new("ShaderNodeMix")
    dark.data_type = "RGBA"
    dark.blend_type = "MULTIPLY"
    l.new(ramp.outputs["Color"], dark.inputs["A"])
    sp = n.new("ShaderNodeMapRange")
    sp.inputs["From Min"].default_value = 0.3
    sp.inputs["From Max"].default_value = 0.7
    sp.inputs["To Min"].default_value = 0.6
    sp.inputs["To Max"].default_value = 1.15
    l.new(fine.outputs["Fac"], sp.inputs["Value"])
    comb = n.new("ShaderNodeCombineColor")
    for k in ("Red", "Green", "Blue"):
        l.new(sp.outputs["Result"], comb.inputs[k])
    l.new(comb.outputs["Color"], dark.inputs["B"])
    dark.inputs["Factor"].default_value = 1.0
    l.new(dark.outputs["Result"], bsdf.inputs["Base Color"])
    bump = n.new("ShaderNodeBump")
    bump.inputs["Strength"].default_value = 0.4
    l.new(fine.outputs["Fac"], bump.inputs["Height"])
    l.new(bump.outputs["Normal"], bsdf.inputs["Normal"])
    plane.data.materials.append(m)
    return plane


def grass(scene, center, radius, count, height=0.22, aim=(0, 0)):
    """Hair-particle grass on a disc around `center` for foreground texture."""
    import bmesh
    cx, cy = center[0], center[1]
    fwd = Vector((aim[0] - cx, aim[1] - cy, 0)).normalized()
    side = Vector((fwd.y, -fwd.x, 0))
    depth, width = radius
    nx, ny = 120, 160
    me = bpy.data.meshes.new("grass")
    bm = bmesh.new()
    verts = []
    for j in range(ny + 1):
        row = []
        for i in range(nx + 1):
            u = i / nx - 0.5
            v = j / ny
            p = Vector((cx, cy, 0.002)) + side * (u * width) + fwd * (-2.0 + v * depth)
            row.append(bm.verts.new(p))
        verts.append(row)
    for j in range(ny):
        for i in range(nx):
            bm.faces.new((verts[j][i], verts[j][i + 1], verts[j + 1][i + 1], verts[j + 1][i]))
    bm.to_mesh(me)
    bm.free()
    disc = bpy.data.objects.new("grass", me)
    scene.collection.objects.link(disc)
    vg = disc.vertex_groups.new(name="density")
    for vi, vert in enumerate(me.vertices):
        j, i = divmod(vi, nx + 1)
        u = abs(i / nx - 0.5) * 2
        v = j / ny
        w = min(1.0, (1 - u) / 0.25) * min(1.0, (1 - v) / 0.35)
        w = max(0.0, w) ** 1.5
        vg.add([vi], w, "REPLACE")
    disc.data.materials.append(bpy.data.materials["meadow"])
    mod = disc.modifiers.new("grass", "PARTICLE_SYSTEM")
    ps = mod.particle_system.settings
    ps.type = "HAIR"
    mod.particle_system.vertex_group_density = "density"
    ps.use_modifier_stack = False
    ps.count = count
    ps.hair_length = height
    ps.use_advanced_hair = True
    ps.child_type = "INTERPOLATED"
    ps.child_percent = 20
    ps.rendered_child_count = 20
    ps.child_length = 1.0
    ps.child_length_threshold = 0.5
    ps.roughness_1 = 0.02
    ps.roughness_endpoint = 0.04
    ps.roughness_2 = 0.03
    ps.use_hair_bspline = False
    ps.display_step = 2
    ps.render_step = 3
    ps.root_radius = 1.0
    ps.tip_radius = 0.0
    ps.radius_scale = 0.003
    ps.factor_random = 0.01
    ps.normal_factor = 1.0
    ps.brownian_factor = 0.0
    ps.use_rotations = True
    ps.rotation_factor_random = 0.3
    ps.hair_length = height
    gm = bpy.data.materials.new("grass")
    gm.use_nodes = True
    nt = gm.node_tree
    n, l = nt.nodes, nt.links
    for x in list(n):
        if x.type == "BSDF_PRINCIPLED":
            n.remove(x)
    hb = n.new("ShaderNodeBsdfPrincipled")
    info = n.new("ShaderNodeHairInfo")
    ramp = n.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].color = (0.03, 0.06, 0.015, 1)
    ramp.color_ramp.elements[1].color = (0.16, 0.2, 0.05, 1)
    l.new(info.outputs["Intercept"], ramp.inputs["Fac"])
    rnd = n.new("ShaderNodeMix")
    rnd.data_type = "RGBA"
    rnd.blend_type = "MULTIPLY"
    rnd.inputs["Factor"].default_value = 0.5
    l.new(ramp.outputs["Color"], rnd.inputs["A"])
    hsv = n.new("ShaderNodeCombineColor")
    for k in ("Red", "Green", "Blue"):
        l.new(info.outputs["Random"], hsv.inputs[k])
    l.new(hsv.outputs["Color"], rnd.inputs["B"])
    l.new(rnd.outputs["Result"], hb.inputs["Base Color"])
    hb.inputs["Roughness"].default_value = 0.7
    hb.inputs["Specular IOR Level"].default_value = 0.2
    hb.inputs["Subsurface Weight"].default_value = 0.0
    l.new(hb.outputs["BSDF"], n["Material Output"].inputs["Surface"])
    disc.data.materials.append(gm)
    ps.material_slot = "grass"
    return disc


def haze(scene, size, density=0.0015):
    bpy.ops.mesh.primitive_cube_add(size=1, location=(0, 0, size[2] / 2))
    box = bpy.context.active_object
    box.name = "haze"
    box.scale = size
    m = bpy.data.materials.new("haze")
    m.use_nodes = True
    n = m.node_tree.nodes
    n.remove(n["Principled BSDF"])
    vol = n.new("ShaderNodeVolumePrincipled")
    vol.inputs["Density"].default_value = density
    vol.inputs["Color"].default_value = (0.75, 0.82, 0.95, 1)
    vol.inputs["Anisotropy"].default_value = 0.5
    m.node_tree.links.new(vol.outputs["Volume"], n["Material Output"].inputs["Volume"])
    box.data.materials.append(m)
    box.visible_shadow = False
    return box


def far_trees(scene, cam_loc, aim, done, count=420, near=260.0, far=1100.0, spread=75.0):
    """Scatter linked copies of LOD2 oaks in a loose band behind the subject,
    to break the bare horizon."""
    rng = random.Random(11)
    protos = []
    for seed in (1, 2, 4, 5, 6, 3):
        path = GEN / f"{ARGS.species}-seed{seed}/glb/lod2.glb"
        if not path.exists():
            continue
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=str(path))
        new = [o for o in bpy.data.objects if o not in before]
        fix_materials([o for o in new if o.type == "MESH"], done)
        coll = bpy.data.collections.new(f"far{seed}")
        for o in new:
            for c in list(o.users_collection):
                c.objects.unlink(o)
            coll.objects.link(o)
        protos.append(coll)
    if not protos:
        return
    base = math.atan2(aim[1] - cam_loc[1], aim[0] - cam_loc[0])
    k = 0
    # Clumps (small woods and hedgerow fragments) rather than evenly spaced
    # parkland trees.
    for _ in range(count // 6):
        a0 = base + math.radians(rng.uniform(-spread, spread))
        r0 = rng.uniform(near, far)
        n = rng.randint(2, 12)
        for _ in range(n):
            a = a0 + rng.gauss(0, 12.0) / r0
            r = r0 + rng.gauss(0, 14.0)
            inst = bpy.data.objects.new(f"far{k}", None)
            k += 1
            inst.instance_type = "COLLECTION"
            inst.instance_collection = rng.choice(protos)
            inst.location = (cam_loc[0] + r * math.cos(a), cam_loc[1] + r * math.sin(a), 0)
            inst.rotation_euler = (0, 0, rng.uniform(0, 2 * math.pi))
            inst.scale = (rng.uniform(0.75, 1.25),) * 3
            scene.collection.objects.link(inst)


def camera(scene, loc, target, lens=35.0, dof=None):
    cam_data = bpy.data.cameras.new("cam")
    cam_data.lens = lens
    cam_data.sensor_width = 36.0
    cam_data.clip_end = 2000
    cam = bpy.data.objects.new("cam", cam_data)
    scene.collection.objects.link(cam)
    cam.location = loc
    cam.rotation_euler = (Vector(target) - Vector(loc)).to_track_quat("-Z", "Y").to_euler()
    if dof:
        cam_data.dof.use_dof = True
        cam_data.dof.focus_distance = dof[0]
        cam_data.dof.aperture_fstop = dof[1]
    scene.camera = cam
    return cam


def main():
    scene = setup_render()
    done = set()
    start = time.time()
    if SHOT in ("hero", "backlit", "bark"):
        root, meshes = import_tree(ARGS.seed, done=done)
        lo, hi = bounds(meshes)
        print("BOUNDS", lo, hi)
        cx, cy = (lo.x + hi.x) / 2, (lo.y + hi.y) / 2
        ground(scene)
        if SHOT == "hero":
            world_and_sun(scene, elevation=13, azimuth=-168, sun_energy=9.0, sky_strength=0.2)
            cam_loc = (cx + 26 * math.cos(math.radians(-110)), cy + 26 * math.sin(math.radians(-110)), 1.65)
            camera(scene, cam_loc, (cx, cy, 6.0), lens=30, dof=(26.0, 8.0))
            grass(scene, cam_loc, (80, 90), 300000, aim=(cx, cy))
            haze(scene, (3000, 3000, 80), 0.0009)
            far_trees(scene, cam_loc, (cx, cy), done)
        elif SHOT == "backlit":
            world_and_sun(scene, elevation=13, azimuth=87, sun_energy=14.0, sky_strength=0.16)
            scene.view_settings.exposure = 0.0
            cam_loc = (cx + 24 * math.cos(math.radians(-110)), cy + 24 * math.sin(math.radians(-110)), 1.65)
            camera(scene, cam_loc, (cx, cy, 6.0), lens=30, dof=(24.0, 8.0))
            grass(scene, cam_loc, (80, 90), 300000, aim=(cx, cy))
            haze(scene, (3000, 3000, 80), 0.0004)
            far_trees(scene, cam_loc, (cx, cy), done)
        else:
            world_and_sun(scene, elevation=28, azimuth=-150, sun_energy=4.0, sky_strength=0.45)
            # trunk base: find the lowest bark vertices' centroid
            bark = [o for o in meshes if o.name.startswith("bark")][0]
            mw = bark.matrix_world
            pts = [mw @ v.co for v in bark.data.vertices]
            low = [p for p in pts if p.z < 1.0]
            tx = sum(p.x for p in low) / len(low)
            ty = sum(p.y for p in low) / len(low)
            print("TRUNK", tx, ty)
            a = math.radians(-110)
            cam_loc = (tx + 8.5 * math.cos(a), ty + 8.5 * math.sin(a), 1.2)
            camera(scene, cam_loc, (tx, ty, 3.6), lens=22, dof=(8.3, 4.0))
            haze(scene, (3000, 3000, 80), 0.0008)
            far_trees(scene, cam_loc, (tx, ty), done)
            grass(scene, cam_loc, (45, 50), 200000, height=0.2, aim=(tx, ty))
    else:
        rng = random.Random(7)
        placements = [
            (3, (0, 0)),
            (5, (-15, 10)),
            (2, (14, 13)),
            (1, (-2, 26)),
            (4, (24, -6)),
        ]
        for seed, (x, y) in placements:
            rot, scale = rng.uniform(0, 2 * math.pi), rng.uniform(0.85, 1.15)
            if not (GEN / f"{ARGS.species}-seed{seed}/glb/lod0.glb").exists():
                print("SKIP seed", seed, "(no GLBs; run species_gallery --glb-only)")
                continue
            import_tree(
                seed,
                loc=(x, y, 0),
                rot=rot,
                scale=scale,
                done=done,
            )
        ground(scene)
        world_and_sun(scene, elevation=15, azimuth=-165, sun_energy=9.0, sky_strength=0.2)
        a = math.radians(-100)
        cam_loc = (4 + 50 * math.cos(a), 8 + 50 * math.sin(a), 2.0)
        camera(scene, cam_loc, (6.5, 8, 6.8), lens=27)
        grass(scene, cam_loc, (95, 120), 400000, aim=(4, 8))
        haze(scene, (3000, 3000, 80), 0.0008)
        far_trees(scene, cam_loc, (4, 8), done, near=300.0)
    print("SETUP", round(time.time() - start, 1))
    t = time.time()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    scene.render.filepath = str(OUT)
    bpy.ops.render.render(write_still=True)
    print("RENDER_SECONDS", round(time.time() - t, 1))


main()
