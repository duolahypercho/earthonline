"""Measure and filter the furnished ground floor for the real-time reflection field.

Run Blender on the packed source named by dining-floor-light.provenance.json.
Capture with --capture-to outside the checkout. Rebuild the installed atlas from
the pinned incident panoramas with --measurements. Use --output-dir to verify.
"""

import argparse
import gzip
import hashlib
import json
import math
import re
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

import bpy
import numpy as np
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from beaver_blender_scene import site_basis
from beaver_floor_gloss import (
    GRID,
    PLAN_METRES,
    SOURCES,
    TILE,
    angular_filter,
    store_panorama,
)
from beaver_street_scene import load_surface
from cook_beaver_dining import DEST
from cook_beaver_mission_roof import png


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def floor_roughness(scene):
    oak = scene.objects["ground-storey"].children
    materials = {
        ob.data.materials[0] for ob in oak if ob.type == "MESH" and ob.data.materials
    }
    fitted = [
        o
        for o in oak
        if o.type == "MESH"
        and o.data.materials
        and o.data.materials[0].name.startswith("Worn oiled floor")
    ]
    if fitted:
        if len(fitted) != 4 or len({o.data.materials[0] for o in fitted}) != 1:
            raise ValueError(
                "Expected four fitted floor meshes with the same photograph"
            )
        rough_socket = (
            fitted[0]
            .data.materials[0]
            .node_tree.nodes["Principled BSDF"]
            .inputs["Roughness"]
        )
        image = rough_socket.links[0].from_node.image
        recipe = json.loads(
            (Path(bpy.data.filepath).parent / "oak/provenance.json").read_text()
        )
        source = next(
            p for p in recipe["packed_maps"] if p["file"] == "worn-oak-rough.png"
        )
        if hashlib.sha256(image.packed_file.data).hexdigest() != source["sha256"]:
            raise ValueError("The fitted roughness scan does not match its source")
        data = np.empty(len(image.pixels), np.float32)
        image.pixels.foreach_get(data)
        return [float(func(data[::4])) for func in (np.min, np.median, np.max)]
    values = []
    for mat in materials:
        if not mat.name.startswith("oiled-oak@"):
            continue
        shader = mat.node_tree.nodes["Principled BSDF"]
        socket = shader.inputs["Roughness"].links[0].from_socket
        if socket.node.type != "SEPARATE_COLOR" or socket.name != "Green":
            raise ValueError(
                "The oak roughness no longer uses the photographic green channel"
            )
        image = socket.node.inputs[0].links[0].from_node.image
        if hashlib.sha256(image.packed_file.data).hexdigest() != digest(
            DEST / "floorboards.png"
        ):
            raise ValueError("The packed floor no longer matches the runtime scan")
        values.append(image)
    if len(values) != 4 or len(set(values)) != 1:
        raise ValueError(
            "Expected four floor materials sharing the board roughness scan"
        )
    image = values[0]
    w, h = image.size
    data = np.empty(w * h * 4, np.float32)
    image.pixels.foreach_get(data)
    rough = data.reshape(h, w, 4)[: h // 3, :, 1]
    return [float(func(rough)) for func in (np.min, np.median, np.max)]


def survey(
    scene, site, calibration, store, *, points=None, source_ready=None, probe_ready=None
):
    for marker in list(scene.timeline_markers):
        scene.timeline_markers.remove(marker)
    camera = bpy.data.objects.new(
        "Ground-storey reflection survey", bpy.data.cameras.new("Incident radiance")
    )
    scene.collection.objects.link(camera)
    scene.camera = camera
    camera.data.type = "PANO"
    camera.data.panorama_type = "EQUIRECTANGULAR"
    camera.data.clip_start = 0.0005
    camera.rotation_euler = (math.pi / 2, 0, 0)
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 64
    scene.cycles.seed = 371
    scene.cycles.use_denoising = scene.cycles.use_adaptive_sampling = False
    scene.cycles.diffuse_bounces = 4
    scene.cycles.max_bounces = 10
    scene.render.resolution_x, scene.render.resolution_y = 128, 64
    scene.render.resolution_percentage = 100
    scene.render.use_border = scene.render.film_transparent = False
    scene.render.use_persistent_data = True
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.image_settings.color_depth = "32"
    sky = scene.world
    lights = [ob for ob in scene.objects if ob.type == "LIGHT"]
    uniform = bpy.data.worlds.new("White sky")
    uniform.use_nodes = True
    background = uniform.node_tree.nodes["Background"]
    background.inputs["Color"].default_value = (1, 1, 1, 1)
    sun = bpy.data.objects.new(
        "Sunlight transfer", bpy.data.lights.new("Unit irradiance", "SUN")
    )
    sun.data.energy = 1
    sun.data.angle = math.radians(0.53)
    scene.collection.objects.link(sun)
    if points is None:
        points = [
            site
            @ Vector(
                (
                    (x + 0.5) * PLAN_METRES[0] / GRID[0],
                    -(z + 0.5) * PLAN_METRES[1] / GRID[1],
                    0.015,
                )
            )
            for z in range(GRID[1])
            for x in range(GRID[0])
        ]
    raw = np.empty(128 * 64 * 4, np.float32)
    with TemporaryDirectory() as temp:
        scene.render.filepath = str(Path(temp) / "incident.exr")
        for row, name in enumerate(SOURCES):
            sun.hide_render = row < 3
            scene.world = sky if name == "clear-sky" else uniform
            background.inputs["Strength"].default_value = float(name == "uniform-sky")
            if row == 0:
                for ob in lights:
                    if ob.data.type == "SUN":
                        ob.hide_render = True
            if row == 1:
                for ob in lights:
                    ob.hide_render = True
                for mat in bpy.data.materials:
                    if mat.node_tree:
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
            if source_ready:
                source_ready(row)
            for index, point in enumerate(points):
                if probe_ready:
                    probe_ready(index)
                camera.location = point
                bpy.ops.render.render(write_still=True)
                image = bpy.data.images.load(
                    scene.render.filepath, check_existing=False
                )
                image.pixels.foreach_get(raw)
                bpy.data.images.remove(image)
                rgb = raw.reshape(64, 128, 4)[:, :, :3].copy()
                if row == 0:
                    rgb *= calibration["fixture_units_per_source_unit"]
                elif row == 2:
                    rgb /= np.asarray(calibration["source_key_rgb"])
                if not np.isfinite(rgb).all() or rgb.max() <= 0:
                    raise ValueError(
                        f"Invalid {name} radiance at floor position {index}"
                    )
                store(row, index, rgb)
            if points:
                print("Measured room reflections:", name, flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--floor-calibration", type=Path)
    parser.add_argument(
        "--scene", help="Use a named lighting scene from a fitted house"
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--capture-to",
        type=Path,
        help="Save the incident panoramas as an external .npy source",
    )
    mode.add_argument(
        "--measurements",
        type=Path,
        help="Cook a previously saved, hash-verified survey",
    )
    args = parser.parse_args(
        sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    )
    dest = args.output_dir.resolve() if args.output_dir else DEST
    if args.output_dir and dest.is_relative_to(ROOT):
        parser.error("Keep validation bakes outside the checkout")
    dest.mkdir(parents=True, exist_ok=True)
    calibration = json.loads(
        (
            args.floor_calibration or DEST / "dining-floor-light.provenance.json"
        ).read_text()
    )
    if (
        digest(Path(bpy.data.filepath)) != calibration["source_scene"]["sha256"]
        or digest(DEST / "terrace.glb") != calibration["house_sha256"]
    ):
        raise ValueError(
            "Use the registered furnished house and the matching runtime mesh"
        )
    with gzip.open(ROOT / "public/data/sf/sf-city.json.gz", "rt") as stream:
        building = next(
            b
            for b in json.load(stream)["detailBuildings"]
            if b["addr"] == "21 Beaver Street"
        )
    ring = [Vector(pt) for pt in zip(building["points"][::2], building["points"][1::2])]
    site = site_basis(ring, load_surface(ROOT, building["centroid"])["plan_scale"])[3]
    if args.scene:
        bpy.context.window.scene = bpy.data.scenes[args.scene]
    scene = bpy.context.scene
    scene.frame_set(3)
    response = ROOT / "native/eo-render/src/photo_camera_response.wgsl"
    gain = float(
        re.search(r"const PHOTO_METER_GAIN: f32 = ([0-9.]+);", response.read_text())[1]
    )
    camera_ev = scene.view_settings.exposure
    native_exposure = 2**camera_ev / (
        gain * calibration["fixture_units_per_source_unit"]
    )
    rough = floor_roughness(scene)
    kernel = angular_filter(rough[1])
    atlas = np.zeros((512, 512, 4), np.uint8)
    capture = (args.capture_to or args.measurements).resolve()
    if capture.is_relative_to(ROOT) or capture.suffix != ".npy":
        parser.error("Choose an external .npy measurement source")
    source_record = capture.with_suffix(".provenance.json")
    shape = (len(SOURCES), GRID[0] * GRID[1], 64, 128, 3)
    if args.capture_to:
        if capture.exists() or source_record.exists():
            parser.error("Capture to a new path to preserve the previous measurement")
        capture.parent.mkdir(parents=True, exist_ok=True)
        readings = np.empty(shape, dtype=np.float32)

        def record(row, index, rgb):
            readings[row, index] = rgb

        survey(scene, site, calibration, record)
        np.save(capture, readings, allow_pickle=False)
        source_record.write_text(
            json.dumps(
                {
                    "file": capture.name,
                    "sha256": digest(capture),
                    "shape": shape,
                    "source_scene": calibration["source_scene"],
                    "house_sha256": calibration["house_sha256"],
                    "sources": SOURCES,
                    "attribution": "Earth Online",
                    "license": "Project-authored lighting data",
                },
                indent=2,
            )
            + "\n"
        )
    original = json.loads(source_record.read_text())
    if (
        original["file"] != capture.name
        or original["sha256"] != digest(capture)
        or original["source_scene"] != calibration["source_scene"]
        or original["house_sha256"] != calibration["house_sha256"]
        or original["sources"] != list(SOURCES)
    ):
        raise ValueError("Unregistered ground-floor measurement source")
    readings = np.load(capture, mmap_mode="r", allow_pickle=False)
    if (
        readings.dtype != np.float32
        or readings.shape != shape
        or not np.isfinite(readings).all()
    ):
        raise ValueError("Invalid floor measurement array")
    for row in range(len(SOURCES)):
        for index in range(GRID[0] * GRID[1]):
            store_panorama(atlas, readings[row, index], row, index, kernel)
    path = dest / "floor-reflections.png"
    png(path, atlas)
    metadata = {
        "source": "Incident radiance above the floorboards of the furnished 21 Beaver Street house.",
        "attribution": "Earth Online",
        "license": "Project-authored lighting data",
        "build": "scripts/blender-buildings/bake_beaver_floor_gloss.py",
        "incident_panoramas": {"file": capture.name, "sha256": original["sha256"]},
        "source_scene": calibration["source_scene"],
        "house_sha256": calibration["house_sha256"],
        "url": "/" + (DEST / path.name).relative_to(ROOT / "public").as_posix(),
        "sha256": digest(path),
        "bytes": path.stat().st_size,
        "grid": GRID,
        "plan_metres": PLAN_METRES,
        "survey_height_m": 0.015,
        "tile_size": TILE,
        "source_rows": SOURCES,
        "transfer_encoding": "sqrt(L/(1+L))",
        "roughness_range_and_median": rough,
        "dining_camera": {
            "source_exposure_ev": camera_ev,
            "linear_game_units_per_source_unit": calibration[
                "fixture_units_per_source_unit"
            ],
            "display_response_sha256": digest(response),
            "display_meter_gain": gain,
            "reference_game_exposure": native_exposure,
        },
        "samples": 64,
        "hemisphere_filter_samples": 256,
    }
    path.with_suffix(".provenance.json").write_text(
        json.dumps(metadata, indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
