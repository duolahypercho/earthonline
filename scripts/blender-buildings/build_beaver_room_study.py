"""Save an editable downstairs study from the furnished house and registered cameras."""

import argparse
import gzip
import hashlib
import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from beaver_blender_scene import site_basis
from beaver_street_scene import load_surface

parser = argparse.ArgumentParser()
parser.add_argument("output", type=Path)
parser.add_argument("registrations", nargs="+", type=Path)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
out = args.output.resolve()
if out.is_relative_to(ROOT):
    parser.error("Keep the editable room study outside the checkout.")
out.mkdir(parents=True, exist_ok=True)
scene = bpy.context.scene
scene.frame_set(3)
source = Path(bpy.data.filepath).resolve()
document = out / "21 Beaver - Downstairs.blend"
if source == document:
    parser.error("Build the room study from the original furnished source.")
camera = scene.camera
exposure = scene.view_settings.exposure
with gzip.open(ROOT / "public/data/sf/sf-city.json.gz", "rt") as f:
    house = next(
        b for b in json.load(f)["detailBuildings"] if b["addr"] == "21 Beaver Street"
    )
ring = [Vector(p) for p in zip(house["points"][::2], house["points"][1::2])]
site = site_basis(ring, load_surface(ROOT, house["centroid"])["plan_scale"])[3]
views = {}
registrations = []
sun_direction = None
for number, path in enumerate(args.registrations, 1):
    raw = path.read_bytes()
    report = json.loads(raw)
    direction = report["light"]["solar_direction_in_blender"]
    if sun_direction is not None and direction != sun_direction:
        parser.error("The registered cameras use different solar directions.")
    sun_direction = direction
    for name, pose in report["views"].items():
        if name in views:
            parser.error(f"Duplicate view {name}")
        views[name] = pose
    saved = out / f"camera-registration-{number}.json"
    saved.write_bytes(raw)
    registrations.append(
        {"file": saved.name, "sha256": hashlib.sha256(raw).hexdigest()}
    )

# Freeze the reference dining exposure and sky in this study. The independent
# component drivers (including the door hinge) belong to their objects.
scene.animation_data_clear()
scene.view_settings.exposure = exposure
scene.world.node_tree.animation_data_clear()
sun = next(o for o in scene.objects if o.type == "LIGHT" and o.data.type == "SUN")
sun.animation_data_clear()
sun.rotation_euler = (-Vector(sun_direction)).to_track_quat("-Z", "Y").to_euler()
for marker in list(scene.timeline_markers):
    scene.timeline_markers.remove(marker)
study = bpy.data.collections.new("Downstairs camera positions")
scene.collection.children.link(study)
scene.timeline_markers.new("Dining room", frame=1).camera = camera


def location(point):
    x, y, z = point
    return site @ Vector((x, -z, y))


for frame, (name, pose) in enumerate(views.items(), 2):
    view = camera.copy()
    view.data = camera.data.copy()
    view.name = name.replace("-", " ").capitalize()
    study.objects.link(view)
    view.location = location(pose["plan_eye"])
    view.rotation_euler = (
        (location(pose["plan_target"]) - view.location)
        .to_track_quat("-Z", "Y")
        .to_euler()
    )
    view.data.shift_x = 0
    view.data.shift_y = 0
    view.data.lens = pose["lens"]
    scene.timeline_markers.new(view.name, frame=frame).camera = view
scene.frame_start = 1
scene.frame_end = len(views) + 1
scene.frame_set(1)
scene.camera = camera
for area in bpy.context.screen.areas:
    if area.type == "VIEW_3D":
        area.spaces.active.region_3d.view_perspective = "CAMERA"
        area.spaces.active.overlay.show_overlays = False
bpy.context.preferences.filepaths.save_version = 0
bpy.ops.wm.save_as_mainfile(filepath=str(document), compress=True)
metadata = {
    "source": source.name,
    "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
    "file": document.name,
    "sha256": hashlib.sha256(document.read_bytes()).hexdigest(),
    "attribution": "Earth Online",
    "license": "Original scene and the source materials' existing licences",
    "build": "scripts/blender-buildings/build_beaver_room_study.py",
    "registrations": registrations,
    "views": views,
    "solar_direction_in_blender": sun_direction,
    "exposure": exposure,
}
(out / "downstairs-study.provenance.json").write_text(
    json.dumps(metadata, indent=2) + "\n"
)
