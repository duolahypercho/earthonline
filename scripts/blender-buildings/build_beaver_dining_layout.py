"""Build the fitted dining arrangement in a separate, fully furnished Blender house.

Open the packed ground-floor study, then pass -- DESTINATION. The source Blender
file is identified by its SHA-256. The output directory must be outside Git.
"""

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Matrix, Vector

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from beaver_dining_oak import install_oak
from beaver_dining_parts import ceramic_bowl, enamel_fixture, folded_linen
from beaver_garden_canopy import garden_canopy
from beaver_kitchen_run import extend_kitchen
from beaver_rush_seats import fit_seats
from beaver_stair_finish import stair_details
from beaver_window_furnishings import rear_plaster, window_furnishings


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def group(scene, name, origin):
    collection = bpy.data.collections.new(name)
    scene.collection.children.link(collection)
    root = bpy.data.objects.new(name, None)
    root.empty_display_size = 0.07
    collection.objects.link(root)
    root.location = origin
    collection.asset_mark()
    collection.asset_data.author = "Earth Online"
    return collection, root


def hide_component(name):
    for child in bpy.data.objects[name].children_recursive:
        child.hide_render = True
        child.hide_viewport = True


def set_camera(scene, fit):
    camera = scene.camera.copy()
    camera.data = camera.data.copy()
    camera.animation_data_clear()
    camera.data.animation_data_clear()
    camera.name = "Dining table at 1.62 m"
    scene.collection.objects.link(camera)
    scene.camera = camera
    x, y, z, head, tilt, lens, shift = fit
    camera.location = (x, y, z)
    d = Vector(
        (
            math.cos(head) * math.cos(tilt),
            math.sin(head) * math.cos(tilt),
            math.sin(tilt),
        )
    )
    camera.rotation_euler = d.to_track_quat("-Z", "Y").to_euler()
    camera.data.lens = lens
    camera.data.shift_y = shift
    return camera


def build(out, oak_source):
    layout = json.loads((HERE / "beaver_dining_layout.json").read_text())
    source = Path(bpy.data.filepath)
    if digest(source) != layout["source_scene_sha256"]:
        raise ValueError("Open the packed ground-floor source described by the layout")
    scene = bpy.data.scenes["Dining · reference sunlight"]
    bpy.context.window.scene = scene
    site = bpy.data.scenes["Dining room · design camera"]
    whole = bpy.data.scenes["Scene"]
    camera = set_camera(scene, layout["fit_camera_xyz_heading_tilt_lens_shift"])
    prior = Vector((*layout["previous_table_center"], 0))
    target = Vector((*layout["table_center"], 0))
    move = (
        Matrix.Translation(target)
        @ Matrix.Rotation(layout["table_rotation_rad"], 4, "Z")
        @ Matrix.Translation(-prior)
    )
    placed = []
    for obj in scene.objects:
        if obj.name == "oak-dining-table" or (
            obj.type == "EMPTY" and obj.name.startswith("dining-chair")
        ):
            obj.matrix_world = move @ obj.matrix_world
            placed.append(obj.name)
    if len(placed) != 7:
        raise ValueError("Expected the original table and its six complete chairs")
    for name in [
        "painted-fruit-bowl",
        "red-apple",
        "red-apple-small",
        "orchard-pear",
        "stoneware-mug",
        "stoneware-mug-rear",
        "open-recipe-book",
        "striped-table-runner",
        "dining-pendant",
    ]:
        hide_component(name)
    # All four scenes share the same installed components and point light.
    # Put new parts in a shared collection, avoiding duplicate chairs or lights.
    install_oak(scene, oak_source, out / "oak", layout["wood_scan_inputs"])
    rear_plaster(scene)
    groups = []
    for name, key, maker in [
        ("Ash-glazed mixing bowl", "bowl_center", ceramic_bowl),
        ("Folded kitchen linen", "linen_center", folded_linen),
    ]:
        coll, root = group(scene, name, layout[key])
        obj = maker(coll)
        obj.parent = root
        if key == "linen_center":
            root.rotation_euler.z = math.radians(14)
        groups.append(coll)
    coll, root = group(scene, "Black enamel dining pendant", layout["pendant_bottom"])
    for obj in enamel_fixture(coll, root.location.z, 3.18):
        obj.parent = root
    groups.append(coll)
    light = next(
        o
        for o in scene.objects
        if o.type == "LIGHT" and o.name.startswith("Dining pendant")
    )
    light.location = root.location + Vector((0, 0, 0.045))
    coll, root = group(scene, "Ground-floor stair joinery", (0, 0, 0))
    for obj in stair_details(scene, coll):
        obj.parent = root
    groups.append(coll)
    bpy.context.view_layer.update()
    seats, _ = group(scene, "Dining rush seats", (0, 0, 0))
    fit_seats(scene, seats)
    groups.append(seats)
    for name, maker in [
        ("Kitchen preparation run", extend_kitchen),
        ("Rear window textiles", window_furnishings),
        ("Canopy in the rear garden", garden_canopy),
    ]:
        coll, _ = group(scene, name, (0, 0, 0))
        maker(scene, coll)
        groups.append(coll)
    for s in bpy.data.scenes:
        if s == scene:
            continue
        for coll in groups:
            s.collection.children.link(coll)
    site.collection.objects.link(camera)
    site.camera = camera
    scene.name = "Dining · design daylight"
    site.name = "Dining · site daylight"
    # Direction inferred from the lines cast by two sash bars in the supplied
    # design. The separate site scene retains the simulated solar position.
    daylight = layout["design_daylight"]
    az = math.radians(daylight["azimuth_from_rear_degrees"])
    el = math.radians(daylight["elevation_degrees"])
    ray = Vector(
        (math.sin(az) * math.cos(el), -math.cos(az) * math.cos(el), math.sin(el))
    )
    sun = next(
        o
        for o in scene.objects
        if o.type == "LIGHT" and o.data.type == "SUN" and not o.hide_render
    )
    sun.rotation_euler = (-ray).to_track_quat("-Z", "Y").to_euler()
    sun.data.angle = math.radians(daylight["solar_diameter_degrees"])
    sun.data.color = (1, 0.77, 0.52)
    # Compensate the simplified opposite street frontage at the physical
    # front window. This is a diffuse return for the photographic light study.
    glass = scene.objects["ground-storey - glass-door@ffffff"]
    pane = [
        glass.matrix_world @ v.co
        for v in glass.data.vertices
        if (glass.matrix_world @ v.co).y > -0.50
    ]
    lo = Vector(tuple(min(v[i] for v in pane) for i in range(3)))
    hi = Vector(tuple(max(v[i] for v in pane) for i in range(3)))
    return_light = bpy.data.lights.new(
        "Diffuse street return through the front window", "AREA"
    )
    return_light.shape = "RECTANGLE"
    return_light.size = hi.x - lo.x
    return_light.size_y = hi.z - lo.z
    return_light.color = (1, 0.91, 0.78)
    return_light.energy = daylight["front_diffuse_watts"]
    return_light.specular_factor = 0
    return_ob = bpy.data.objects.new(return_light.name, return_light)
    scene.collection.objects.link(return_ob)
    return_ob.location = (lo + hi) * 0.5 + Vector((0, 0.42, 0))
    return_ob.rotation_euler = Vector((0, -1, 0)).to_track_quat("-Z", "Y").to_euler()
    return_ob.visible_glossy = False
    return_ob.visible_transmission = False
    scene.view_settings.exposure = daylight["camera_exposure_ev"]
    scene.render.filepath = "//Dining - fitted room.png"
    site.render.filepath = "//Dining - site daylight.png"
    scene.unit_settings.system = "METRIC"
    whole.objects["front-door"]["Open angle"] = 0.0
    bpy.context.view_layer.update()
    for area in bpy.context.screen.areas:
        if area.type == "VIEW_3D":
            area.spaces.active.region_3d.view_perspective = "CAMERA"
            area.spaces.active.overlay.show_overlays = False
    bpy.context.preferences.filepaths.save_version = 0
    path = out / "21 Beaver - Fitted dining room.blend"
    bpy.ops.wm.save_as_mainfile(filepath=str(path), compress=True)
    record = {
        "source": source.name,
        "source_sha256": layout["source_scene_sha256"],
        "design_image_sha256": layout["design_image_sha256"],
        "file": path.name,
        "sha256": digest(path),
        "attribution": "Earth Online",
        "license": "Project-authored assembly and new materials. Existing photographs retain their source licences.",
        "build": "scripts/blender-buildings/build_beaver_dining_layout.py",
        "layout_sha256": digest(HERE / "beaver_dining_layout.json"),
        "rigid_components": sorted(placed),
        "part_collections": [c.name for c in groups],
        "camera_m": camera.location.z,
        "sunlight_scene": scene.name,
        "site_scene": site.name,
    }
    (out / "fitted-dining.provenance.json").write_text(
        json.dumps(record, indent=2) + "\n"
    )


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("destination", type=Path)
    p.add_argument(
        "--oak-source",
        type=Path,
        required=True,
        help="Folder containing the three pinned photographic inputs",
    )
    args = p.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    out = args.destination.resolve()
    if out.is_relative_to(HERE.parents[1]):
        p.error("Choose an external asset folder")
    out.mkdir(parents=True, exist_ok=True)
    build(out, args.oak_source.resolve())
