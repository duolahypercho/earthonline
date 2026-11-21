"""Measure the street and courtyard surfaces seen in the house glass, in Cycles."""

import argparse
import hashlib
import json
import math
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

import bpy
import numpy as np

HOURS = (8.5, 12.0, 17.0, 22.0)
WIDTH, HEIGHT = 160, 80
ROOT = Path(__file__).resolve().parents[2]
PHOTO = ROOT / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def geometry(scene, rooms):
    selected = []
    positions = []
    groups = []
    glass = {}
    # Survey the two sides separately so each bake can be restored and checked.
    graph = bpy.context.evaluated_depsgraph_get()
    for ob in sorted(scene.objects, key=lambda o: o.name):
        if ob.type != "MESH" or ob.hide_render or not ob.visible_glossy:
            continue
        obj = ob.evaluated_get(graph)
        mesh = obj.to_mesh()
        if not mesh or not len(mesh.polygons):
            obj.to_mesh_clear()
            continue
        xyz = np.empty((len(mesh.vertices), 3), np.float32)
        mesh.vertices.foreach_get("co", xyz.ravel())
        matrix = np.asarray(obj.matrix_world)
        xyz = (xyz @ matrix[:3, :3].T + matrix[:3, 3]).astype(np.float32)
        mesh.calc_loop_triangles()
        indices = np.empty((len(mesh.loop_triangles), 3), np.int32)
        mesh.loop_triangles.foreach_get("vertices", indices.ravel())
        corners = np.empty_like(indices)
        mesh.loop_triangles.foreach_get("loops", corners.ravel())
        tris = xyz[indices]
        if ob.name.split(" - ")[-1].startswith("glass-door@"):
            glass[ob.name] = [float(tris[:, :, 1].min()), float(tris[:, :, 1].max())]
            obj.to_mesh_clear()
            continue
        lo, hi = tris.min(1), tris.max(1)
        use = (
            ((hi[:, 1] >= -0.605) | (lo[:, 1] <= -16.717))
            & (hi[:, 0] > -65)
            & (lo[:, 0] < 72)
            & (lo[:, 1] < 82)
            & (hi[:, 1] > -82)
            & (hi[:, 2] > -25)
        )
        if rooms:
            # Between the two elevations, keep the entire furnished house
            # and the immediate adjoining walls. The bands are disjoint.
            use = (
                ~use
                & (hi[:, 0] > -1.0)
                & (lo[:, 0] < 10.5)
                & (lo[:, 1] > -16.717)
                & (hi[:, 1] < -0.605)
                & (hi[:, 2] > -2.0)
                & (lo[:, 2] < 12.0)
            )
        if use.any():
            # Material slots on the original objects retain texture coordinates,
            # normal maps, shader drivers and the other objects' cast shadows.
            real = bpy.data.meshes.new_from_object(
                obj, preserve_all_data_layers=True, depsgraph=graph
            )
            if len(real.loops) != len(mesh.loops):
                raise ValueError(
                    "Evaluated mesh topology changed while collecting " + ob.name
                )
            selected.append((ob, real, corners[use]))
            positions.append(tris[use])
            groups.append([ob.name, int(use.sum())])
        obj.to_mesh_clear()
    for ob, mesh, _ in selected:
        ob.modifiers.clear()
        ob.data = mesh
        color = mesh.color_attributes.new(
            name="Window optics · outgoing light", type="FLOAT_COLOR", domain="CORNER"
        )
        mesh.color_attributes.active_color = color
    if (
        len(glass) != 4
        or not rooms
        and not any(name.startswith("frontage-broadleaf") for name, n in groups)
    ):
        raise ValueError("Expected the furnished house and surveyed broadleaf")
    return selected, np.concatenate(positions), groups, glass


def sky_camera(scene):
    cam = bpy.data.cameras.new("Incident sky")
    obj = bpy.data.objects.new("Sky sampled at the house", cam)
    scene.collection.objects.link(obj)
    cam.type = "PANO"
    cam.panorama_type = "EQUIRECTANGULAR"
    cam.clip_start = 0.001
    cam.clip_end = 0.01
    obj.location = (0, 0, 60)
    obj.rotation_euler = (math.pi / 2, 0, 0)
    scene.camera = obj
    scene.render.resolution_x = WIDTH
    scene.render.resolution_y = HEIGHT
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = False
    scene.render.image_settings.file_format = "OPEN_EXR"
    scene.render.image_settings.color_mode = "RGB"
    scene.render.image_settings.color_depth = "32"


def sky_image(scene, directory):
    scene.render.filepath = str(directory / "sky.exr")
    bpy.ops.render.render(write_still=True, scene=scene.name)
    image = bpy.data.images.load(scene.render.filepath, check_existing=False)
    pixels = np.empty((HEIGHT, WIDTH, 4), np.float32)
    image.pixels.foreach_get(pixels.ravel())
    bpy.data.images.remove(image)
    # Blender image memory starts at the bottom, i.e. negative elevation.
    return pixels[:, :, :3].copy()


def bake(out, rooms=False):
    src = Path(bpy.data.filepath)
    record = json.loads((src.parent / "plaster-irradiance.provenance.json").read_text())
    manifest = json.loads((PHOTO / "wall-daylight.provenance.json").read_text())
    if (
        digest(src) != record["scene_sha256"]
        or record["scene_sha256"] != manifest["source_scene_sha256"]
    ):
        raise ValueError("Open the registered plaster and daylight source")
    if out.resolve().is_relative_to(ROOT):
        raise ValueError("Keep optical measurements outside the repository")
    out.mkdir(exist_ok=True, parents=True)
    sc = bpy.context.scene
    sc.render.engine = "CYCLES"
    prefs = bpy.context.preferences.addons["cycles"].preferences
    try:
        prefs.compute_device_type = "METAL"
        devices = prefs.get_devices_for_type("METAL")
        for device in devices:
            device.use = device.type == "METAL"
        sc.cycles.device = "GPU" if any(d.use for d in devices) else "CPU"
    except TypeError:
        sc.cycles.device = "CPU"
    sc.cycles.samples = 32
    sc.cycles.seed = 97
    sc.cycles.use_denoising = False
    sc.cycles.use_adaptive_sampling = False
    selected, xyz, groups, panes = geometry(sc, rooms)
    np.save(out / "triangles.npy", xyz)
    del xyz
    print("Optical surfaces", len(selected), sum(n for _, n in groups), flush=True)
    sky_camera(sc)
    bpy.ops.object.select_all(action="DESELECT")
    for ob, _, _ in selected:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = selected[0][0]
    sc.render.bake.target = "VERTEX_COLORS"
    worlds = []
    unit = bpy.data.worlds.new("Unit overcast field")
    unit.use_nodes = True
    unit.node_tree.nodes["Background"].inputs["Color"].default_value = (1, 1, 1, 1)
    unit.node_tree.nodes["Background"].inputs["Strength"].default_value = 1
    with TemporaryDirectory() as tmp:
        for i, hour in enumerate((*HOURS, None)):
            sc.frame_set(round(2 * (hour if hour is not None else 12)))
            if hour is None:
                sc.world = unit
                for light in sc.objects:
                    if light.type == "LIGHT":
                        light.hide_render = True
            if abs(sc.objects["front-door"]["Open angle"]) > 0.0001:
                raise ValueError("The registered door must be closed")
            bpy.ops.object.bake(
                type="COMBINED",
                pass_filter={"DIFFUSE", "DIRECT", "INDIRECT", "EMIT"},
                target="VERTEX_COLORS",
                use_clear=True,
            )
            fields = []
            for ob, mesh, corners in selected:
                data = mesh.color_attributes.active_color.data
                rgba = np.empty((len(data), 4), np.float32)
                data.foreach_get("color", rgba.ravel())
                fields.append(rgba[corners, :3].astype(np.float16))
            colors = np.concatenate(fields)
            if not np.isfinite(colors).all() or colors.max() <= 0:
                raise ValueError("Empty optical light bake")
            np.save(out / f"light-{i}.npy", colors)
            worlds.append(sky_image(sc, Path(tmp)))
            print(
                "Finished optical light", hour, "peak", float(colors.max()), flush=True
            )
    np.save(out / "sky.npy", np.asarray(worlds))
    parcel = json.loads(
        (src.parent.parent / "street-garden/street-garden.provenance.json").read_text()
    )["parcel_from_assembled_gltf"]
    meta = {
        "region": "rooms" if rooms else "street and courtyard",
        "source": src.name,
        "source_sha256": digest(src),
        "house_sha256": manifest["house_sha256"],
        "hours": list(HOURS),
        "samples": sc.cycles.samples,
        "groups": groups,
        "panes": panes,
        "canonical_source_to_game": record["canonical_source_to_game"],
        "parcel_from_assembled_gltf": parcel,
        "files": {f.name: digest(f) for f in sorted(out.glob("*.npy"))},
        "build": "scripts/blender-buildings/bake_beaver_window_optics.py",
    }
    (out / "optics-source.provenance.json").write_text(
        json.dumps(meta, indent=2) + "\n"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=Path)
    parser.add_argument(
        "--rooms",
        action="store_true",
        help="Measure reflections seen from inside the house",
    )
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    bake(args.out, args.rooms)
