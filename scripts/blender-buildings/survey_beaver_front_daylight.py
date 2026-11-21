"""Fit the sky to the game sun and measure the light arriving at the front bed.

Run in Blender on the packed street-garden house. Supply the CSV produced by
the eo-look site_daylight example and an external directory for the new scene
and scene-linear panoramas. The direct solar disk is excluded from the probes.
"""

import argparse
import csv
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

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
PHOTO = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo"
HOURS = (8, 10, 12, 14, 16, 18)
ACROSS = (0.68, 2.62, 4.56)
DEPTH = (-1.78, -0.52)
HEIGHT = 0.50
SOURCE_SCENE = "Front garden · street daylight"
SITE_SCENE = "Front garden · solar day"
SUN = "Site sun · 21 Beaver"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_site():
    with gzip.open(ROOT / "public/data/sf/sf-city.json.gz", "rt") as stream:
        building = next(
            b
            for b in json.load(stream)["detailBuildings"]
            if b["addr"] == "21 Beaver Street"
        )
    a, b = [Vector(building["points"][i : i + 2]) for i in (2, 4)]
    across = (b - a).normalized()
    return across, Vector((across.y, -across.x))


def attach_daylight(source, records, calibration):
    original = next(
        o
        for o in source.objects
        if o.type == "LIGHT" and o.data.type == "SUN" and not o.hide_render
    )
    scene = source.copy()
    scene.name = SITE_SCENE
    scene.use_fake_user = True
    objects = list(source.objects)
    for coll in list(scene.collection.children):
        scene.collection.children.unlink(coll)
    for ob in list(scene.collection.objects):
        scene.collection.objects.unlink(ob)
    contents = bpy.data.collections.new(
        "Architecture and contents · shared source objects"
    )
    scene.collection.children.link(contents)
    for ob in objects:
        if ob != original:
            contents.objects.link(ob)
    sun = original.copy()
    sun.data = original.data.copy()
    sun.animation_data_clear()
    sun.data.animation_data_clear()
    sun.name = SUN
    scene.collection.objects.link(sun)
    scene.world = source.world.copy()
    scene.world.name = "Sky above Beaver Street"
    scene.world.node_tree.animation_data_clear()
    scene.animation_data_clear()
    for m in list(scene.timeline_markers):
        scene.timeline_markers.remove(m)
    scene.camera = source.camera
    bpy.context.window.scene = scene
    sky = next(n for n in scene.world.node_tree.nodes if n.type == "TEX_SKY")
    sky.sun_disc = False
    background = next(n for n in scene.world.node_tree.nodes if n.type == "BACKGROUND")
    strength = background.inputs["Strength"].default_value
    axis, front = read_site()
    reference = np.asarray(records[12]["key"])
    # Keep the established Blender wattage at noon. The game supplies the
    # relative solar energy and colour at every other hour.
    scale = np.asarray(calibration["source_key_rgb"]) / reference
    previous_rotation = None
    for hour, row in sorted(records.items()):
        east, up, south = row["sun"]
        horizontal = Vector((east, south))
        toward = Vector((axis.dot(horizontal), front.dot(horizontal), up)).normalized()
        rotation = (-toward).to_track_quat("-Z", "Y").to_euler()
        if previous_rotation:
            rotation.make_compatible(previous_rotation)
        previous_rotation = rotation.copy()
        sun.rotation_euler = rotation
        rgb = np.asarray(row["key"]) * scale if up > 0 else np.zeros(3)
        energy = float(rgb.max())
        sun.data.energy = energy
        sun.data.color = rgb / energy if energy > 0 else (1, 1, 1)
        sky.sun_elevation = math.asin(max(0.0, up))
        # In this panoramic camera +Y is zero azimuth, turning toward +X.
        sky.sun_rotation = math.atan2(toward.x, toward.y) % math.tau
        background.inputs["Strength"].default_value = strength * min(
            1.0, max(0.0, 1 + up / 0.105)
        )
        frame = round(2 * hour)
        sun.keyframe_insert("rotation_euler", frame=frame)
        for key in ("energy", "color"):
            sun.data.keyframe_insert(key, frame=frame)
        for key in ("sun_elevation", "sun_rotation"):
            sky.keyframe_insert(key, frame=frame)
        background.inputs["Strength"].keyframe_insert("default_value", frame=frame)
    scene.frame_start = round(2 * min(records))
    scene.frame_end = round(2 * max(records))
    scene.frame_set(24)
    scene.view_settings.exposure = 1
    return scene


def survey(scene, parcel, out):
    # Skip the nearby leaves only. Their 45 cm visibility is already stored on
    # the individual vertices. Soil, coping, windows and adjacent houses still
    # intercept and return light in the survey.
    group = bpy.data.collections["Front garden · agaves and deergrass"]
    for ob in group.objects:
        if ob.asset_data:
            ob.hide_render = True
    camera = bpy.data.objects.new(
        "Incident radiance survey", bpy.data.cameras.new("Full sphere")
    )
    scene.collection.objects.link(camera)
    scene.camera = camera
    camera.data.type = "PANO"
    camera.data.panorama_type = "EQUIRECTANGULAR"
    camera.data.clip_start = 0.002
    camera.rotation_euler = (math.pi / 2, 0, 0)
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 64
    scene.cycles.seed = 871
    scene.cycles.use_denoising = scene.cycles.use_adaptive_sampling = False
    scene.render.resolution_x, scene.render.resolution_y = 128, 64
    scene.render.resolution_percentage = 100
    scene.render.use_border = scene.render.film_transparent = False
    scene.render.use_persistent_data = True
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.image_settings.color_depth = "32"
    xy = [(x, z) for z in DEPTH for x in ACROSS]
    points = []
    for x, z in xy:
        source = np.linalg.solve(
            parcel, [x, 0.07 - 0.093 * x + 0.078 * z + HEIGHT, z, 1]
        )
        points.append([source[0], -source[2], source[1]])
    sky = scene.world
    uniform = bpy.data.worlds.new("Unit overcast sky")
    uniform.use_nodes = True
    uniform.node_tree.nodes["Background"].inputs["Color"].default_value = (1, 1, 1, 1)
    uniform.node_tree.nodes["Background"].inputs["Strength"].default_value = 1
    sun = scene.objects[SUN]
    raw = np.empty((64, 128, 4), np.float32)
    panoramas = []
    with TemporaryDirectory() as tmp:
        scene.render.filepath = str(Path(tmp) / "sphere.exr")
        for hour in (*HOURS, None):
            scene.frame_set(round((hour or 12) * 2))
            scene.world = sky if hour is not None else uniform
            # Bounces from the daylight key remain in the panorama. A Blender
            # SUN has no visible disk, so direct sunlight is never counted twice.
            for light in scene.objects:
                if light.type == "LIGHT" and (light != sun or hour is None):
                    light.hide_render = True
            row = []
            for point in points:
                camera.location = point
                bpy.ops.render.render(write_still=True)
                im = bpy.data.images.load(scene.render.filepath, check_existing=False)
                im.pixels.foreach_get(raw.ravel())
                bpy.data.images.remove(im)
                rgb = raw[:, :, :3].copy()
                if not np.isfinite(rgb).all() or rgb.max() <= 0:
                    raise ValueError(f"Invalid light at {point}, hour {hour}")
                row.append(rgb)
            panoramas.append(row)
            print("Measured daylight", hour, flush=True)
    np.savez_compressed(
        out / "front-incident.npz", radiance=np.array(panoramas, dtype=np.float32)
    )
    return points


def run(args):
    out = args.out.resolve()
    if out.is_relative_to(ROOT):
        raise ValueError(
            "Use an external directory for the packed source and panoramas"
        )
    src = Path(bpy.data.filepath)
    meta = json.loads((src.parent / "street-garden.provenance.json").read_text())
    if (
        digest(src) != meta["sha256"]
        or meta["sha256"]
        != json.loads((PHOTO / "street-garden.provenance.json").read_text())[
            "source_scene_sha256"
        ]
    ):
        raise ValueError("Expected the registered street garden")
    records = {}
    for r in csv.DictReader(args.solar.read_text().splitlines()):
        records[float(r["hour"])] = {
            "sun": [float(r[k]) for k in ("east", "up", "south")],
            "key": [float(r[k]) for k in ("key_r", "key_g", "key_b")],
            "sky": float(r["sky_irradiance_up"]),
        }
    if not set(HOURS).issubset(records):
        raise ValueError("Incomplete solar sampling")
    out.mkdir(parents=True, exist_ok=True)
    calibration = json.loads(
        (PHOTO / "fitted-dining/dining-floor-light.provenance.json").read_text()
    )
    source = bpy.data.scenes[SOURCE_SCENE]
    bpy.context.window.scene = source
    source.frame_set(3)
    scene = attach_daylight(source, records, calibration)
    bpy.context.preferences.filepaths.save_version = 0
    filepath = out / "21 Beaver - Site daylight.blend"
    bpy.ops.wm.save_as_mainfile(filepath=str(filepath), compress=True)
    points = survey(scene, np.array(meta["parcel_from_assembled_gltf"]), out)
    raw = out / "front-incident.npz"
    provenance = {
        "scene": filepath.name,
        "scene_sha256": digest(filepath),
        "parent_scene_sha256": meta["sha256"],
        "panoramas": raw.name,
        "panoramas_sha256": digest(raw),
        "solar_table_sha256": digest(args.solar),
        "source_hours": HOURS,
        "uniform_row": len(HOURS),
        "probe_across": ACROSS,
        "probe_depth": DEPTH,
        "height_above_grade_m": HEIGHT,
        "source_points": points,
        "canonical_source_to_game": calibration["fixture_units_per_source_unit"],
        "sky_up_by_hour": [records[h]["sky"] for h in HOURS],
        "build": "scripts/blender-buildings/survey_beaver_front_daylight.py",
    }
    (out / "front-daylight.provenance.json").write_text(
        json.dumps(provenance, indent=2) + "\n"
    )


if __name__ == "__main__":
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("solar", type=Path)
    cli.add_argument("out", type=Path)
    run(cli.parse_args(sys.argv[sys.argv.index("--") + 1 :]))
