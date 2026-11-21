"""Fit the furnished entrance lamp and two architectural cameras to its Blender study."""

import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parent))
from beaver_entry_light import fit_opal_emitter

# Fitted to the window corners, sill, door head, threshold and porch return.
# The origins, focal lengths and sensor shifts are shared with the game cameras.
VIEWS = (
    (
        "Sidewalk from the design reference",
        (6.4532355, 11.0830209, 2.4128223),
        (6.4499570, 6.0905043, 2.6862584),
        18.0,
        -0.0156892,
    ),
    (
        "Entrance from the design reference",
        (7.3765961, 5.0072994, 1.4336284),
        (7.7523333, 0.0214977, 1.4090858),
        30.4364068,
        -0.0390337,
    ),
)


def build(out):
    src = Path(bpy.data.filepath)
    metadata = json.loads((src.parent / "window-study.provenance.json").read_text())
    if hashlib.sha256(src.read_bytes()).hexdigest() != metadata["sha256"]:
        raise ValueError("Open the registered window study")
    if out.resolve().is_relative_to(Path(__file__).resolve().parents[2]):
        raise ValueError("Save the editable study in the source store")
    out.mkdir(parents=True, exist_ok=True)
    scene = bpy.context.scene
    scene.frame_set(34)
    if scene.objects["front-door"]["Open angle"] != 0:
        raise ValueError("Expected a closed entrance")
    lamp = fit_opal_emitter(scene, scene.objects["Entry sconce"])
    if lamp is None:
        raise ValueError("The installed shade is missing")
    group = bpy.data.collections.new("Reference street and porch cameras")
    scene.collection.children.link(group)
    for name, eye, target, focal, shift in VIEWS:
        camera = bpy.data.cameras.new(name)
        camera.sensor_fit = "HORIZONTAL"
        camera.sensor_width = 36
        camera.lens = focal
        camera.shift_y = shift
        camera.clip_start = 0.035
        camera.clip_end = 3000
        ob = bpy.data.objects.new(name, camera)
        group.objects.link(ob)
        ob.location = eye
        ob.rotation_euler = (
            (Vector(target) - ob.location).to_track_quat("-Z", "Y").to_euler()
        )
    scene.camera = ob
    scene.render.resolution_x = 1672
    scene.render.resolution_y = 941
    scene.render.resolution_percentage = 100
    scene.view_settings.exposure = 1
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 64
    scene.cycles.seed = 38
    scene.render.image_settings.media_type = "IMAGE"
    scene.render.image_settings.file_format = "PNG"
    photo = out / "Porch at five.png"
    scene.render.filepath = str(photo.resolve())
    bpy.ops.render.render(write_still=True)
    preview = bpy.data.images.load(str(photo.resolve()))
    preview.name = "Porch at five · Cycles"
    preview.pack()
    scene.render.filepath = "//" + photo.name
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type == "VIEW_3D":
                area.spaces.active.use_local_camera = False
                area.spaces.active.region_3d.view_perspective = "CAMERA"
            if screen.name.startswith("Rendering") and area.type == "IMAGE_EDITOR":
                area.spaces.active.image = preview
    dst = out / "21 Beaver Street - Opal porch lamp.blend"
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(dst), compress=True)
    record = {
        "file": dst.name,
        "sha256": hashlib.sha256(dst.read_bytes()).hexdigest(),
        "parent_sha256": metadata["sha256"],
        "source_scene_sha256": metadata["source_sha256"],
        "fixture": lamp,
        "preview": photo.name,
        "preview_sha256": hashlib.sha256(photo.read_bytes()).hexdigest(),
        "cameras": VIEWS,
        "build": "scripts/blender-buildings/study_beaver_entry_light.py",
    }
    (out / "porch-light.provenance.json").write_text(
        json.dumps(record, indent=2) + "\n"
    )


if __name__ == "__main__":
    arguments = argparse.ArgumentParser()
    arguments.add_argument("out", type=Path)
    build(arguments.parse_args(sys.argv[sys.argv.index("--") + 1 :]).out)
