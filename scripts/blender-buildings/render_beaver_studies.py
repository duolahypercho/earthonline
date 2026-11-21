"""Reopen the source, exercise the hinged entry, and render its camera bookmarks."""

import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy

parser = argparse.ArgumentParser()
parser.add_argument("output", type=Path)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
output = args.output.resolve()
repo = Path(__file__).resolve().parents[2]
if output.is_relative_to(repo):
    parser.error("Keep review renders outside the checkout.")
output.mkdir(parents=True, exist_ok=True)
scene = bpy.context.scene
bookmarks = sorted(scene.timeline_markers, key=lambda marker: marker.frame)
names = ("upper", "rear", "dining", "bedroom", "entry", "kitchen-detail")
assert len(bookmarks) == len(names) and all(marker.camera for marker in bookmarks)


def bounds(objects):
    coords = [
        obj.matrix_world @ vertex.co for obj in objects for vertex in obj.data.vertices
    ]
    return [
        round(max(p[i] for p in coords) - min(p[i] for p in coords), 4)
        for i in range(3)
    ]


if bpy.context.scene.unit_settings.scale_length != 1.0:
    raise ValueError("Expected metric geometry")
door = bpy.data.objects["front-door"]
assert door["Open angle"] == 0
paint = next(o for o in door.children if "door-paint" in o.name)
latch = min(paint.data.vertices, key=lambda v: v.co.x).co.copy()
hinge = door.matrix_world.translation.copy()
closed = paint.matrix_world @ latch
assert 0.95 < (closed - hinge).xy.length < 0.98


def swing(degrees):
    door["Open angle"] = degrees
    door.update_tag()
    bpy.context.view_layer.update()
    return (
        paint.evaluated_get(bpy.context.evaluated_depsgraph_get()).matrix_world @ latch
    )


opened = swing(90.0)
assert abs((opened - hinge).xy.length - (closed - hinge).xy.length) < 0.001
assert (opened - closed).length > 1.32 and opened.y < closed.y - 0.94
assert (swing(0.0) - closed).length < 0.001
sizes = {
    name: bounds(bpy.data.objects[name].children)
    for name in ("oak-dining-table", "enamel-refrigerator", "window-seat", "double-bed")
}
sizes["painted-door-leaf"] = bounds([paint])
assert abs(sizes["oak-dining-table"][2] - 0.75) < 0.001
assert abs(sizes["painted-door-leaf"][2] - 2.05) < 0.001
assert abs(sizes["painted-door-leaf"][0] - 0.96) < 0.004
unpacked = [
    im.name for im in bpy.data.images if im.source == "FILE" and not im.packed_file
]
if unpacked:
    raise ValueError(f"Missing packed sources: {unpacked}")
devices = bpy.context.preferences.addons["cycles"].preferences
if sys.platform == "darwin":
    try:
        devices.compute_device_type = "METAL"
        hardware = devices.get_devices_for_type("METAL")
        if any(d.type == "METAL" for d in hardware):
            for dev in hardware:
                dev.use = dev.type == "METAL"
            scene.cycles.device = "GPU"
    except RuntimeError:
        scene.cycles.device = "CPU"
files = []
for bookmark, name in zip(bookmarks, names):
    scene.frame_set(bookmark.frame)
    assert scene.camera == bookmark.camera
    path = output / f"{name}.png"
    scene.render.filepath = str(path)
    bpy.ops.render.render(write_still=True)
    files.append(
        {
            "file": path.name,
            "camera": scene.camera.name,
            "lens_mm": scene.camera.data.lens,
            "exposure": scene.view_settings.exposure,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        }
    )
    print("Rendered", path, flush=True)
source = Path(bpy.data.filepath)
report = {
    "blend": str(source),
    "blend_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
    "blender": bpy.app.version_string,
    "engine": scene.render.engine,
    "size": [scene.render.resolution_x, scene.render.resolution_y],
    "samples": scene.cycles.samples,
    "components": sum("component" in o for o in scene.objects),
    "packed_images": sum(bool(im.packed_file) for im in bpy.data.images),
    "measured_xyz_m": sizes,
    "latch_travel_90_deg_m": round((opened - closed).length, 4),
    "door_rest_angle_deg": door["Open angle"],
    "photos": files,
}
(output / "blender-verification.json").write_text(json.dumps(report, indent=2) + "\n")
