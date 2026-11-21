"""Save an editable view of the furnished house with matched window inspection cameras."""

import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector

VIEWS = (
    (
        "left-window",
        "Upper sash from the west",
        (-2.8, 9.4, 1.73),
        (3.6, -0.08, 5.12),
        39,
    ),
    (
        "right-window",
        "Upper sash from the east",
        (7.25, 7.2, 1.73),
        (3.6, -0.08, 5.12),
        34,
    ),
    (
        "lower-glass",
        "Ground floor glass at eye height",
        (0.5, 6.5, 1.73),
        (3.15, -0.08, 1.64),
        31,
    ),
    ("arcade", "Three arched casements", (1.9, 9, 5.65), (3.05, -0.59, 8.25), 34),
    ("rear-from-yard", "Courtyard windows", (1.5, -22, 2.2), (3.75, -16.73, 4.6), 25),
    ("inside-out", "From the bedroom window", (3.85, -4, 4.8), (3.45, 0, 5), 25),
)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(out):
    src = Path(bpy.data.filepath)
    parent = json.loads((src.parent / "house-view.provenance.json").read_text())
    if digest(src) != parent["sha256"] or out.resolve().is_relative_to(
        Path(__file__).resolve().parents[2]
    ):
        raise ValueError(
            "Open the registered furnished house and save the study outside the checkout"
        )
    s = bpy.context.scene
    if s.objects["front-door"]["Open angle"] != 0:
        raise ValueError("Expected the closed-door study")
    group = bpy.data.collections.new("Window inspection cameras")
    s.collection.children.link(group)
    for tag, name, eye, target, lens in VIEWS:
        data = bpy.data.cameras.new(name)
        data.lens = lens
        data.clip_start = 0.025
        data.clip_end = 3000
        ob = bpy.data.objects.new(name, data)
        group.objects.link(ob)
        ob["inspection_view"] = tag
        ob.location = eye
        ob.rotation_euler = (
            (Vector(target) - ob.location).to_track_quat("-Z", "Y").to_euler()
        )
        if tag == "left-window":
            s.camera = ob
    s.frame_set(34)
    s.render.engine = "CYCLES"
    prefs = bpy.context.preferences.addons["cycles"].preferences
    try:
        prefs.compute_device_type = "METAL"
        devices = prefs.get_devices_for_type("METAL")
        for dev in devices:
            dev.use = dev.type == "METAL"
        s.cycles.device = "GPU" if any(d.use for d in devices) else "CPU"
    except TypeError:
        s.cycles.device = "CPU"
    s.cycles.seed = 38
    s.cycles.samples = 64
    s.view_settings.exposure = 1
    s.render.resolution_x = 1536
    s.render.resolution_y = 864
    s.render.resolution_percentage = 100
    s.render.image_settings.file_format = "PNG"
    for layer in s.view_layers:
        layer.use_pass_glossy_direct = True
        layer.use_pass_glossy_indirect = True
        layer.use_pass_glossy_color = True
        layer.use_pass_transmission_direct = True
        layer.use_pass_transmission_indirect = True
    out.mkdir(parents=True, exist_ok=True)
    png = out / "Upper sash at 17.00.png"
    s.render.filepath = str(png)
    bpy.ops.render.render(write_still=True, scene=s.name)
    im = bpy.data.images.load(str(png), check_existing=False)
    im.name = "Street tree in the upper sash · Cycles at 17:00"
    im.pack()
    im.use_fake_user = True
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type == "VIEW_3D":
                area.spaces.active.use_local_camera = False
                area.spaces.active.region_3d.view_perspective = "CAMERA"
            if screen.name.startswith("Rendering") and area.type == "IMAGE_EDITOR":
                area.spaces.active.image = im
    dst = out / "21 Beaver Street - Window optics.blend"
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(dst), compress=True)
    record = {
        "file": dst.name,
        "sha256": digest(dst),
        "parent_sha256": parent["sha256"],
        "source_sha256": parent["parent_scene_sha256"],
        "preview": png.name,
        "preview_sha256": digest(png),
        "cameras": [list(r) for r in VIEWS],
        "build": "scripts/blender-buildings/study_beaver_windows.py",
    }
    (out / "window-study.provenance.json").write_text(
        json.dumps(record, indent=2) + "\n"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=Path)
    save(parser.parse_args(sys.argv[sys.argv.index("--") + 1 :]).out)
