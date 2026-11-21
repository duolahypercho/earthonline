"""Measure the furnished room reflected in the four waxed dining boards."""

import argparse
import gzip
import hashlib
import json
import math
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

import bpy
import numpy as np
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from beaver_blender_scene import site_basis
from beaver_street_scene import load_surface
from cook_beaver_dining import DEST
from cook_beaver_mission_roof import png


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def material_inputs(boards):
    records = []
    for obj in boards:
        bs = obj.data.materials[0].node_tree.nodes.get("Principled BSDF")
        rough = bs.inputs["Roughness"].links[0].from_socket
        if rough.node.type != "SEPARATE_COLOR" or rough.name != "Green":
            raise ValueError(
                "The tabletop roughness is no longer the scanned green channel"
            )
        image = rough.node.inputs[0].links[0].from_node.image
        if hashlib.sha256(image.packed_file.data).hexdigest() != digest(
            DEST / "floorboards.png"
        ):
            raise ValueError("Packed oak scan differs from the installed board texture")
        normal = bs.inputs["Normal"].links[0].from_node
        if normal.type != "NORMAL_MAP":
            raise ValueError("Expected tangent-space oak relief")
        records.append(
            [
                normal.inputs["Strength"].default_value,
                bs.inputs["Coat Weight"].default_value,
                bs.inputs["Coat Roughness"].default_value,
                bs.inputs["Coat IOR"].default_value,
            ]
        )
    if len(records) != 4 or len({tuple(r) for r in records}) != 1:
        raise ValueError("Expected the same wax on four individual boards")
    width, height = image.size
    rgba = np.empty(width * height * 4, np.float32)
    image.pixels.foreach_get(rgba)
    data = rgba.reshape(height, width, 4)[: height // 3, :, 1][::-1]
    samples = np.concatenate(
        [
            data[
                round((row + 0.07) * len(data) / 8) : round(
                    (row + 0.93) * len(data) / 8
                ),
                round(0.025 * width) : round(0.975 * width),
            ].ravel()
            for row in (0, 3, 5, 7)
        ]
    )
    return records[0], float(np.median(samples))


def filter_radiance(panorama, roughness):
    # A split-sum GGX environment integral. Each output direction reflects
    # toward the zenith or the horizon of the real table, including the glazing.
    ny, nx = 30, 62
    az = ((np.arange(nx) + 0.5) / nx - 0.5) * 2 * math.pi
    el = (np.arange(ny) + 0.5) / ny * (math.pi / 2)
    angle, elevation = np.meshgrid(az, el)
    rays = np.stack(
        (
            np.sin(angle) * np.cos(elevation),
            np.cos(angle) * np.cos(elevation),
            np.sin(elevation),
        ),
        axis=-1,
    )
    east = np.stack(
        (np.cos(angle), -np.sin(angle), np.zeros_like(el[:, None] + az)), axis=-1
    )
    north = np.cross(east, rays)
    result = np.zeros_like(rays)
    normalizer = 0.0
    count = 512
    # Hammersley samples over the microfacet distribution, V = R = N.
    for i in range(count):
        u = (i + 0.5) / count
        v = int(f"{i:09b}"[::-1], 2) / count
        cos_h = math.sqrt((1 - u) / (1 + (roughness**4 - 1) * u))
        sin_h = math.sqrt(1 - cos_h * cos_h)
        ndotl = 2 * cos_h * cos_h - 1
        if ndotl <= 0:
            continue
        direction = ndotl * rays + 2 * cos_h * sin_h * (
            math.cos(v * 2 * math.pi) * east + math.sin(v * 2 * math.pi) * north
        )
        h, w = panorama.shape[:2]
        # Blender pixels are bottom-up. Longitude zero is camera-forward (+Y).
        x = (
            np.arctan2(direction[:, :, 0], direction[:, :, 1]) / (2 * math.pi) + 0.5
        ) * w - 0.5
        y = np.clip(
            (np.arcsin(np.clip(direction[:, :, 2], -1, 1)) / math.pi + 0.5) * h - 0.5,
            0,
            h - 1,
        )
        x0, y0 = np.floor(x).astype(int), np.floor(y).astype(int)
        fx, fy = (x - x0)[:, :, None], (y - y0)[:, :, None]
        low = (1 - fx) * panorama[y0, x0 % w] + fx * panorama[y0, (x0 + 1) % w]
        high = (1 - fx) * panorama[np.minimum(y0 + 1, h - 1), x0 % w] + fx * panorama[
            np.minimum(y0 + 1, h - 1), (x0 + 1) % w
        ]
        result += ndotl * ((1 - fy) * low + fy * high)
        normalizer += ndotl
    result /= normalizer
    return np.pad(
        np.pad(result, ((0, 0), (1, 1), (0, 0)), mode="wrap"),
        ((1, 1), (0, 0), (0, 0)),
        mode="edge",
    )


def daylight_receiver(scene, site, lo, hi, height):
    corners = [(lo[0], lo[1]), (lo[0], hi[1]), (hi[0], hi[1]), (hi[0], lo[1])]
    mesh = bpy.data.meshes.new("White meter above the four boards")
    mesh.from_pydata(
        [site @ Vector((x, -z, height)) for x, z in corners], [], [(0, 1, 2, 3)]
    )
    mesh.update()
    uv = mesh.uv_layers.new(name="Tabletop irradiance")
    for loop, coord in zip(
        mesh.polygons[0].loop_indices, ((0, 0), (0, 1), (1, 1), (1, 0))
    ):
        uv.data[loop].uv = coord
    meter = bpy.data.objects.new(mesh.name, mesh)
    scene.collection.objects.link(meter)
    for ray in (
        "diffuse",
        "glossy",
        "transmission",
        "volume_scatter",
        "shadow",
    ):
        setattr(meter, "visible_" + ray, False)
    mat = bpy.data.materials.new("Unit diffuse oak receiver")
    mat.use_nodes = True
    nodes = mat.node_tree.nodes
    white = nodes.new("ShaderNodeBsdfDiffuse")
    white.inputs["Color"].default_value = (1, 1, 1, 1)
    mat.node_tree.links.new(
        white.outputs[0], nodes.get("Material Output").inputs["Surface"]
    )
    mesh.materials.append(mat)
    image = bpy.data.images.new(
        "Board daylight response", 256, 64, float_buffer=True, alpha=False
    )
    image.colorspace_settings.name = "Non-Color"
    target = nodes.new("ShaderNodeTexImage")
    target.image = image
    nodes.active = target
    bpy.ops.object.select_all(action="DESELECT")
    meter.select_set(True)
    bpy.context.view_layer.objects.active = meter
    return image


def store_daylight(scene, image, atlas, row, source_key):
    # The native shadow map supplies direct sun. Capture diffuse return from
    # sunlit furnishings, along with direct and bounced sky above the actual oak.
    passes = {"INDIRECT"} if row >= 3 else {"DIRECT", "INDIRECT"}
    meter = bpy.context.view_layer.objects.active
    meter.visible_camera = True
    scene.cycles.samples = 1536
    bpy.ops.object.bake(
        type="DIFFUSE",
        pass_filter=passes,
        target="IMAGE_TEXTURES",
        margin=0,
        use_clear=True,
    )
    raw = np.empty(256 * 64 * 4, dtype=np.float32)
    image.pixels.foreach_get(raw)
    values = np.maximum(raw.reshape(64, 256, 4)[:, :, :3], 0)
    if row == 2:
        values = values / np.array(source_key)
    if not np.isfinite(values).all() or values.max() < 0.00001:
        raise ValueError("The daylight receiver returned no finite lighting signal")
    k = row - 1
    y = 224 + 64 * (k // 2)
    x = 256 * (k % 2)
    atlas[y : y + 64, x : x + 256, :3] = np.rint(
        255 * np.sqrt(values / (1 + values))
    ).astype(np.uint8)
    meter.visible_camera = False
    scene.cycles.samples = 192


def build(out):
    source = Path(bpy.data.filepath)
    floor = json.loads((DEST / "dining-floor-light.provenance.json").read_text())
    table = json.loads((DEST / "tabletop-light.provenance.json").read_text())
    if (
        digest(source) != floor["source_scene"]["sha256"]
        or digest(DEST / "terrace.glb") != floor["house_sha256"]
    ):
        raise ValueError("Use the packed furnished source and its registered house")
    scene = bpy.context.scene
    scene.frame_set(3)
    # A photo bookmark otherwise replaces the meter at render time.
    for marker in list(scene.timeline_markers):
        scene.timeline_markers.remove(marker)
    boards = [
        o
        for o in scene.objects["oak-dining-table"].children
        if "dining-board" in o.name
    ]
    finish, base_rough = material_inputs(boards)
    with gzip.open(ROOT / "public/data/sf/sf-city.json.gz", "rt") as file:
        house = next(
            h
            for h in json.load(file)["detailBuildings"]
            if h["addr"] == "21 Beaver Street"
        )
    ring = [Vector(p) for p in zip(house["points"][::2], house["points"][1::2])]
    site = site_basis(ring, load_surface(ROOT, house["centroid"])["plan_scale"])[3]
    lo, hi = (np.asarray(table[key])[[0, 2]] for key in ("minimum", "maximum"))
    lower, upper = lo + 0.12 * (hi - lo), hi - 0.12 * (hi - lo)
    locations = [
        site @ Vector((x, -z, table["tabletop_height_m"] + 0.006))
        for z in (lower[1], upper[1])
        for x in (lower[0], upper[0])
    ]
    camera = bpy.data.objects.new(
        "Tabletop incident radiance", bpy.data.cameras.new("Spherical radiance meter")
    )
    scene.collection.objects.link(camera)
    camera.data.type = "PANO"
    camera.data.panorama_type = "EQUIRECTANGULAR"
    camera.data.clip_start = 0.0005
    camera.rotation_euler = (math.pi / 2, 0, 0)
    scene.camera = camera
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 192
    scene.cycles.seed = 117
    scene.cycles.use_denoising = scene.cycles.use_adaptive_sampling = False
    scene.cycles.diffuse_bounces = 4
    scene.cycles.max_bounces = 10
    scene.render.resolution_x, scene.render.resolution_y = 256, 128
    scene.render.resolution_percentage = 100
    scene.render.use_border = scene.render.film_transparent = False
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.image_settings.color_depth = "32"
    sky = scene.world
    lights = [o for o in scene.objects if o.type == "LIGHT"]
    uniform = bpy.data.worlds.new("Achromatic calibration sky")
    uniform.use_nodes = True
    bg = uniform.node_tree.nodes["Background"]
    bg.inputs["Color"].default_value = (1, 1, 1, 1)
    sun = bpy.data.objects.new(
        "Solar transfer", bpy.data.lights.new("Unit source", "SUN")
    )
    sun.data.energy = 1
    sun.data.angle = math.radians(0.53)
    scene.collection.objects.link(sun)
    atlas = np.zeros((512, 512, 4), dtype=np.uint8)
    sources = (
        "fixtures",
        "uniform-sky",
        "clear-sky",
        "front-25",
        "front-57",
        "rear-25",
        "rear-57",
    )
    energy = []
    daylight = daylight_receiver(
        scene, site, lo, hi, table["tabletop_height_m"] + 0.004
    )
    bpy.context.view_layer.objects.active.visible_camera = False
    with TemporaryDirectory() as temp:
        path = Path(temp) / "incident.exr"
        for row, name in enumerate(sources):
            sun.hide_render = row < 3
            scene.world = sky if name == "clear-sky" else uniform
            bg.inputs["Strength"].default_value = float(name == "uniform-sky")
            if row == 0:
                for o in lights:
                    if o.data.type == "SUN":
                        o.hide_render = True
            elif row == 1:
                for o in lights:
                    o.hide_render = True
                for mat in bpy.data.materials:
                    if not mat.node_tree:
                        continue
                    for node in mat.node_tree.nodes:
                        slot = {
                            "BSDF_PRINCIPLED": "Emission Strength",
                            "EMISSION": "Strength",
                        }.get(node.type)
                        if slot:
                            for link in list(node.inputs[slot].links):
                                mat.node_tree.links.remove(link)
                            node.inputs[slot].default_value = 0
            if row >= 3:
                side, degrees = name.split("-")
                elevation = math.radians(int(degrees))
                incoming = Vector(
                    (
                        0,
                        (1 if side == "front" else -1) * math.cos(elevation),
                        math.sin(elevation),
                    )
                )
                sun.rotation_euler = (-incoming).to_track_quat("-Z", "Y").to_euler()
            if row > 0:
                store_daylight(scene, daylight, atlas, row, floor["source_key_rgb"])
            for index, point in enumerate(locations):
                camera.location = point
                scene.render.filepath = str(path)
                bpy.ops.render.render(write_still=True)
                image = bpy.data.images.load(str(path), check_existing=False)
                pixels = np.empty(256 * 128 * 4, np.float32)
                image.pixels.foreach_get(pixels)
                bpy.data.images.remove(image)
                raw = pixels.reshape(128, 256, 4)[:, :, :3]
                if not np.isfinite(raw).all() or raw.max() <= 0:
                    raise ValueError(f"Unlit reflection probe {name} {index}")
                if row == 0:
                    raw = raw * floor["fixture_units_per_source_unit"]
                elif row == 2:
                    raw = raw / np.array(floor["source_key_rgb"])
                for lobe, rough in enumerate((base_rough, finish[2])):
                    field = np.maximum(0, filter_radiance(raw, rough))
                    data = np.rint(255 * np.sqrt(field / (1 + field))).astype(np.uint8)
                    x = (index + 4 * lobe) * 64
                    atlas[row * 32 : (row + 1) * 32, x : x + 64, :3] = data
                energy.append(float(raw.mean()))
                print("Measured oak reflection", name, index, energy[-1], flush=True)
    if min(np.ptp(np.asarray(energy).reshape(len(sources), 4), axis=1)) < 1e-7:
        raise ValueError(
            "A source returned identical radiance at the four different positions"
        )
    path = out / "oak-reflection.png"
    png(path, atlas)
    meta = dict(
        source="Furnished 21 Beaver timber, installed lamps, sky and sun through the sashes.",
        attribution="Earth Online",
        license="Project-authored lighting data",
        build="scripts/blender-buildings/bake_beaver_oak.py",
        source_scene=floor["source_scene"],
        house_sha256=floor["house_sha256"],
        url="/" + (DEST / path.name).relative_to(ROOT / "public").as_posix(),
        sha256=digest(path),
        bytes=path.stat().st_size,
        probe_rectangle=[*lower, *upper],
        probe_height_m=table["tabletop_height_m"] + 0.006,
        source_rows=sources,
        tabletop_bounds=[*lo, *hi],
        daylight_map=[256, 64],
        daylight_first_row=224,
        tile_size=[64, 32],
        base_roughness=base_rough,
        normal_strength=finish[0],
        coat_weight=finish[1],
        coat_roughness=finish[2],
        coat_ior=finish[3],
        projection="azimuth and elevation above the oak, horizontal wrap and vertical clamp",
        transfer_encoding="sqrt(L/(1+L))",
        samples=scene.cycles.samples,
        filter_samples=512,
        source_means=[round(value, 7) for value in energy],
    )
    path.with_suffix(".provenance.json").write_text(json.dumps(meta, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args(
        sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    )
    out = args.output_dir.resolve() if args.output_dir else DEST
    if args.output_dir and out.is_relative_to(ROOT):
        parser.error("Keep verification bakes outside the checkout.")
    out.mkdir(parents=True, exist_ok=True)
    build(out)


if __name__ == "__main__":
    main()
