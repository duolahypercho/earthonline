"""Bake the Cycles quilt and the upstairs room's daylight response for the Rust game."""

import argparse
import gzip
import hashlib
import json
import math
import struct
import sys
import zlib
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from beaver_blender_scene import site_basis
from beaver_street_scene import load_surface

OUT = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo"
SIZE = 512
scene = bpy.context.scene
args = argparse.ArgumentParser()
args.add_argument("product", choices=("quilt", "light"))
product = args.parse_args(sys.argv[sys.argv.index("--") + 1 :]).product
scene.frame_set(4)
scene.render.engine = "CYCLES"
preferences = bpy.context.preferences.addons["cycles"].preferences
if sys.platform == "darwin":
    preferences.compute_device_type = "METAL"
    devices = preferences.get_devices_for_type("METAL")
    if any(device.type == "METAL" for device in devices):
        for device in devices:
            device.use = device.type == "METAL"
        scene.cycles.device = "GPU"
scene.cycles.seed = 73
scene.cycles.use_adaptive_sampling = False
scene.cycles.use_denoising = False


def publish(name, rgb, metadata):
    # Stored rows start at the top. Write the original eight-bit channel
    # values directly, including the non-color normal and transfer pages.
    def chunk(kind, blob):
        return (
            struct.pack(">I", len(blob))
            + kind
            + blob
            + struct.pack(">I", zlib.crc32(kind + blob))
        )

    height, width, _ = rgb.shape
    rgba = np.empty((height, width, 4), np.uint8)
    rgba[:, :, :3] = np.rint(np.clip(rgb, 0, 1) * 255).astype(np.uint8)
    rgba[:, :, 3] = 255
    raw = b"".join(b"\0" + row.tobytes() for row in rgba)
    png = b"\x89PNG\r\n\x1a\n" + chunk(
        b"IHDR", struct.pack(">2I5B", width, height, 8, 6, 0, 0, 0)
    )
    png += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
    path = OUT / (name + ".png")
    path.write_bytes(png)
    metadata["source_scene"] = {
        "file": Path(bpy.data.filepath).name,
        "sha256": hashlib.sha256(Path(bpy.data.filepath).read_bytes()).hexdigest(),
    }
    metadata.update(
        url="/" + path.relative_to(ROOT / "public").as_posix(),
        bytes=len(png),
        sha256=hashlib.sha256(png).hexdigest(),
        build="scripts/blender-buildings/bake_beaver_bedroom.py",
    )
    path.with_suffix(".provenance.json").write_text(
        json.dumps(metadata, indent=2) + "\n"
    )
    print("Baked", name, len(png), metadata["sha256"], flush=True)


def select_only(obj):
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj


def quilt():
    # Bake on the actual-size flat cutting pattern. The wrinkle Bump node
    # expresses its height in metres, so a unit square would change the relief.
    garment = next(
        m for m in bpy.data.materials if m.name.startswith("cotton-coverlet@")
    )
    source = next(
        o for o in scene.objects if o.type == "MESH" and o.active_material == garment
    )
    tile = json.loads((OUT.parents[1] / "linen.provenance.json").read_text())
    source_uv = source.data.uv_layers["Photographic finish"]
    extent = [max(p.uv[i] for p in source_uv.data) for i in (0, 1)]
    width, height = (value * tile["metres_per_tile"] for value in extent)
    mesh = bpy.data.meshes.new("Cotton pattern receiver")
    mesh.from_pydata(
        [(0, 0, 0), (width, 0, 0), (width, height, 0), (0, height, 0)],
        [],
        [(0, 1, 2, 3)],
    )
    scan = mesh.uv_layers.new(name="Photographic finish")
    dst = mesh.uv_layers.new(name="Cut fabric")
    for i, (u, v) in enumerate(((0, 0), (1, 0), (1, 1), (0, 1))):
        scan.data[i].uv = (u * extent[0], v * extent[1])
        dst.data[i].uv = (u, v)
    mesh.uv_layers.active = dst
    dst.active_render = True
    obj = bpy.data.objects.new(mesh.name, mesh)
    scene.collection.objects.link(obj)
    obj.location.z = 500
    mat = garment.copy()
    mesh.materials.append(mat)
    select_only(obj)
    tree = mat.node_tree
    bs = tree.nodes.get("Principled BSDF")
    surf = next(n for n in tree.nodes if n.type == "OUTPUT_MATERIAL").inputs["Surface"]
    # The portable material is already tinted by the authored dye.
    target = bpy.data.images.new(
        "Portable cotton", SIZE, SIZE, alpha=False, float_buffer=True
    )
    target.colorspace_settings.name = "Non-Color"
    node = tree.nodes.new("ShaderNodeTexImage")
    node.image = target
    tree.nodes.active = node
    emit = tree.nodes.new("ShaderNodeEmission")
    scene.cycles.samples = 1

    def page(kind):
        bpy.ops.object.bake(
            type=kind,
            normal_space="TANGENT",
            uv_layer=dst.name,
            margin=0,
            use_clear=True,
            target="IMAGE_TEXTURES",
        )
        data = np.empty(SIZE * SIZE * 4, np.float32)
        target.pixels.foreach_get(data)
        return data.reshape(SIZE, SIZE, 4)[::-1, :, :3].copy()

    normal = page("NORMAL")
    tree.links.new(emit.outputs[0], surf)
    tree.links.new(bs.inputs["Base Color"].links[0].from_socket, emit.inputs[0])
    color = page("EMIT")
    color = np.where(
        color <= 0.0031308,
        12.92 * color,
        1.055 * np.maximum(color, 0) ** (1 / 2.4) - 0.055,
    )
    tree.links.new(bs.inputs["Roughness"].links[0].from_socket, emit.inputs[0])
    rough = page("EMIT")
    orm = np.stack(
        (np.ones_like(rough[:, :, 0]), rough[:, :, 0], np.zeros_like(rough[:, :, 0])),
        axis=2,
    )
    assert normal[:, :, 2].mean() > 0.65 and normal[:, :, 0].std() > 0.02
    assert 0.08 < color.mean() < 0.76 and rough[:, :, 0].mean() > 0.48
    publish(
        "washed-cotton",
        np.concatenate((color, normal, orm)),
        dict(
            source=tile["source"],
            license=tile["license"],
            attribution=tile["attribution"],
            inputs=tile["inputs"],
            families=["cotton-coverlet"],
            metres_per_tile=1.0,
            normal_strength=1.0,
            paint_mix=0.0,
            cut_pattern=True,
            cut_metres=[round(width, 4), round(height, 4)],
            process="Cycles tangent normal, dyed diffuse and roughness of the Blender coverlet on its flat cutting pattern.",
        ),
    )


def daylight():
    with gzip.open(ROOT / "public/data/sf/sf-city.json.gz", "rt") as f:
        house = next(
            b
            for b in json.load(f)["detailBuildings"]
            if b["addr"] == "21 Beaver Street"
        )
    ring = [Vector(p) for p in zip(house["points"][::2], house["points"][1::2])]
    scale = load_surface(ROOT, house["centroid"])["plan_scale"]
    site = site_basis(ring, scale)[3]
    points, faces = [], []
    # Each infinitesimal white receiver measures the irradiance incident from
    # its hemisphere. Ray visibility is disabled so probes cannot shade the room.
    axes = [
        Vector(v)
        for v in ((1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1), (0, -1, 0), (0, 1, 0))
    ]

    def receiver(p, n):
        u = n.orthogonal().normalized() * 0.001
        v = n.cross(u)
        first = len(points)
        points.extend((p - u - v, p + u - v, p + u + v, p - u + v))
        faces.append([first, first + 1, first + 2, first + 3])

    nx, ny, nz = 10, 6, 19
    low, high = (0.22, 10 / 3 + 0.15, 0.23), (6.78, 20 / 3 - 0.17, 12.77)
    for n in axes:
        for z in np.linspace(low[2], high[2], nz):
            for y in np.linspace(low[1], high[1], ny):
                for x in np.linspace(low[0], high[0], nx):
                    receiver(site @ Vector((x, -z, y)), n)
    receiver(Vector((0, 0, 550)), Vector((0, 0, 1)))
    mesh = bpy.data.meshes.new("Upstairs directional receivers")
    mesh.from_pydata(points, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(mesh.name, mesh)
    scene.collection.objects.link(obj)
    for ray in ("diffuse", "glossy", "transmission", "volume_scatter", "shadow"):
        setattr(obj, "visible_" + ray, False)
    mat = bpy.data.materials.new("Unit diffuse receiver")
    mat.use_nodes = True
    tree = mat.node_tree
    diffuse = tree.nodes.new("ShaderNodeBsdfDiffuse")
    diffuse.inputs[0].default_value = (1, 1, 1, 1)
    tree.links.new(diffuse.outputs[0], tree.nodes.get("Material Output").inputs[0])
    mesh.materials.append(mat)
    # One image texel per oriented receiver. The last row holds an unobstructed
    # reference facing the zenith, to check the unit-sky calibration.
    image_width, image_height = nx * nz, 6 * ny + 1
    uv = mesh.uv_layers.new(name="Probe texels")
    for i, face in enumerate(mesh.polygons):
        if i == len(faces) - 1:
            col, row = 0, 6 * ny
        else:
            axis, z, y, x = np.unravel_index(i, (6, nz, ny, nx))
            col, row = z * nx + x, axis * ny + y
        for j, (u, v) in zip(face.loop_indices, ((0, 0), (1, 0), (1, 1), (0, 1))):
            uv.data[j].uv = ((col + u) / image_width, (row + v) / image_height)
    target = bpy.data.images.new(
        "Directional light samples",
        image_width,
        image_height,
        alpha=False,
        float_buffer=True,
    )
    target.colorspace_settings.name = "Non-Color"
    node = tree.nodes.new("ShaderNodeTexImage")
    node.image = target
    tree.nodes.active = node
    select_only(obj)
    for light in scene.objects:
        if light.type == "LIGHT":
            light.hide_render = True
    for m in bpy.data.materials:
        if m.node_tree:
            for node in m.node_tree.nodes:
                if node.type == "BSDF_PRINCIPLED":
                    slot = node.inputs["Emission Strength"]
                    for link in list(slot.links):
                        m.node_tree.links.remove(link)
                    slot.default_value = 0
                elif node.type == "EMISSION":
                    for link in list(node.inputs["Strength"].links):
                        m.node_tree.links.remove(link)
                    node.inputs["Strength"].default_value = 0
    clear_sky = scene.world.copy()
    sky_node = next(n for n in clear_sky.node_tree.nodes if n.type == "TEX_SKY")
    sky_node.sun_disc = False
    reading_light = bpy.data.objects["Bedside reading light"]
    world = bpy.data.worlds.new("Unit outdoor radiance")
    world.use_nodes = True
    scene.world = world
    background = world.node_tree.nodes.get("Background")
    background.inputs["Color"].default_value = (1, 1, 1, 1)
    sun = bpy.data.lights.new("Unit directional irradiance", "SUN")
    sun.energy = 1
    sun.angle = math.radians(0.53)
    lamp = bpy.data.objects.new(sun.name, sun)
    scene.collection.objects.link(lamp)
    scene.cycles.samples = 1536
    scene.cycles.diffuse_bounces = 4
    scene.cycles.max_bounces = 10
    atlas = np.zeros((SIZE, SIZE, 3), np.float32)
    checks = {}
    for index, (label, side, altitude) in enumerate(
        (
            ("sky", 0, 0),
            ("front-low", 1, 25),
            ("front-high", 1, 57),
            ("rear-low", -1, 25),
            ("rear-high", -1, 57),
            ("day-sky", 0, 0),
            ("reading-light", 0, 0),
        )
    ):
        background.inputs["Strength"].default_value = float(index == 0)
        scene.world = clear_sky if label == "day-sky" else world
        lamp.hide_render = not 1 <= index <= 4
        reading_light.hide_render = label != "reading-light"
        angle = math.radians(altitude)
        incident = Vector((0, side * math.cos(angle), math.sin(angle)))
        lamp.rotation_euler = (-incident).to_track_quat("-Z", "Y").to_euler()
        passes = {"DIRECT", "INDIRECT"} if lamp.hide_render else {"INDIRECT"}
        bpy.ops.object.bake(
            type="DIFFUSE",
            pass_filter=passes,
            target="IMAGE_TEXTURES",
            margin=0,
            use_clear=True,
        )
        raw = np.empty(image_width * image_height * 4, np.float32)
        target.pixels.foreach_get(raw)
        pixels = raw.reshape(image_height, image_width, 4)[:, :, :3]
        calibration = pixels[-1, 0]
        print("Open receiver", label, calibration, "peak", pixels.max(), flush=True)
        if index == 0:
            assert 0.83 < calibration.mean() < 1.2, ("sky calibration", calibration)
        field = pixels[:-1].reshape(6, ny, nz, nx, 3).transpose(0, 2, 1, 3, 4)
        assert np.isfinite(field).all() and field.max() > 0.001, (label, field.max())
        # Express the reference sky in units of the source sun. The game and
        # Blender use different clear-sky-to-key ratios and scene light units.
        # Normalizing per channel retains the authored daylight white point.
        if label == "day-sky":
            key = bpy.data.objects["Morning sun"].data
            field = field / (np.asarray(key.color) * key.energy)
        if label == "reading-light":
            field = field / 2.0
        for axis in range(6):
            y0 = (index * 6 + axis) * ny
            for z in range(nz):
                atlas[y0 : y0 + ny, z * nx : (z + 1) * nx] = np.sqrt(
                    np.clip(field[axis, z], 0, 1)
                )
        checks[label] = dict(
            mean=[round(float(c), 5) for c in field.mean(axis=(0, 1, 2, 3))],
            maximum=round(float(field.max()), 4),
            open_receiver=[round(float(c), 5) for c in calibration],
        )
        print(label, checks[label], flush=True)
    key_light = bpy.data.objects["Morning sun"]
    sun_dir = key_light.matrix_world.to_quaternion() @ Vector((0, 0, 1))
    key_on_card = (
        key_light.data.energy
        * np.dot(key_light.data.color, [0.2126, 0.7152, 0.0722])
        * sun_dir.z
    )
    room_exposure = scene.view_settings.exposure
    scene.frame_set(1)
    outdoor_exposure = scene.view_settings.exposure
    sky_return = float(np.dot(checks["sky"]["mean"], [0.2126, 0.7152, 0.0722]))
    publish(
        "bedroom-daylight",
        atlas,
        dict(
            exposure_range_ev=round(room_exposure - outdoor_exposure, 4),
            exterior_exposed_beam=round(float(key_on_card * 2**outdoor_exposure), 5),
            mean_sky_return=round(sky_return, 6),
            source="Authored terrace and surrounding city geometry in the editable Blender scene.",
            attribution="Earth Online",
            license="Project-authored lighting data",
            grid=[nx, ny, nz],
            minimum=low,
            maximum=high,
            samples=scene.cycles.samples,
            optical_paths=[
                "unit white sky, direct and indirect",
                "unit white sun, indirect only",
                "clear reference sky without the sun disc, per channel relative to the source sun",
                "physical 5 W bedside fixture, direct and indirect",
            ],
            solar_elevation_degrees=[25, 57],
            diffuse_bounces=4,
            calibration=checks,
        ),
    )


if product == "quilt":
    quilt()
else:
    daylight()
