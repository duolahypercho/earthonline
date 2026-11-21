"""Open the packed house on the registered street camera with the survey mesh tucked away."""

import argparse
import hashlib
import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def present(out, preview):
    source = Path(bpy.data.filepath)
    record = json.loads(
        (source.parent / "plaster-irradiance.provenance.json").read_text()
    )
    if digest(source) != record["scene_sha256"]:
        raise ValueError("Expected the verified plaster source")
    target = out.resolve() / "21 Beaver Street.blend"
    if target.is_relative_to(Path(__file__).resolve().parents[2]):
        raise ValueError("Save editable Blender files outside the repository")
    target.parent.mkdir(parents=True, exist_ok=True)
    scene = bpy.data.scenes["Front garden · solar day"]
    bpy.context.window.scene = scene
    for coll in scene.collection.children:
        if coll.name == "House cameras":
            raise ValueError("This scene already contains the presentation cameras")
    group = bpy.data.collections.new("House cameras")
    scene.collection.children.link(group)

    def camera(name, eye, target, lens):
        data = bpy.data.cameras.new(name)
        data.lens = lens
        data.clip_start = 0.025
        data.clip_end = 3000
        ob = bpy.data.objects.new(name, data)
        group.objects.link(ob)
        ob.location = eye
        ob.rotation_euler = (
            (Vector(target) - ob.location).to_track_quat("-Z", "Y").to_euler()
        )
        return ob

    scene.camera = camera(
        "Street frontage",
        (-1.99506, 13.34006, 2.7),
        (3.17644, 2.63128, 4.30549),
        21.2275,
    )
    camera("Entrance from the steps", (6.1, 5.55, 1.53), (7.78, 0.24, 1.55), 26)
    camera("Along the side passage", (-1.55, 6.88, 2.9), (0.09, -4, 4.25), 32)
    for ob in list(scene.objects):
        if ob.type == "CAMERA" and ob.name.startswith(
            ("01 ·", "02 ·", "03 ·", "04 ·", "06 ·")
        ):
            group.objects.link(ob)
    receiver = scene.objects["Exterior daylight · neutral wall receivers"]
    receiver.hide_viewport = True
    receiver.hide_render = True
    scene.frame_set(34)
    scene.unit_settings.system = "METRIC"
    scene.render.resolution_x = 1536
    scene.render.resolution_y = 864
    scene.render.resolution_percentage = 100
    scene.view_settings.exposure = 1
    packed_preview = None
    if preview is not None:
        image = bpy.data.images.load(str(preview.resolve()), check_existing=False)
        if tuple(image.size) != (1536, 864):
            raise ValueError("The registered street frame must be 1536 by 864")
        image.name = "Street frontage · Cycles at 17:00"
        image.pack()
        image.use_fake_user = True
        for screen in bpy.data.screens:
            if screen.name.startswith("Rendering"):
                for area in screen.areas:
                    if area.type == "IMAGE_EDITOR":
                        area.spaces.active.image = image
        packed_preview = {"image": image.name, "sha256": digest(preview)}
    scene.cycles.preview_samples = 32
    scene.cycles.use_preview_denoising = True
    bpy.ops.object.select_all(action="DESELECT")
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type == "VIEW_3D":
                space = area.spaces.active
                space.use_local_camera = False
                space.region_3d.view_perspective = "CAMERA"
                if screen == bpy.context.window.screen:
                    space.shading.type = "RENDERED"
                space.overlay.show_overlays = False
                space.shading.color_type = "MATERIAL"
    bpy.context.preferences.filepaths.save_version = 0
    bpy.ops.wm.save_as_mainfile(filepath=str(target), compress=True)
    (out / "house-view.provenance.json").write_text(
        json.dumps(
            {
                "file": target.name,
                "sha256": digest(target),
                "parent_scene_sha256": record["scene_sha256"],
                "opening_camera": scene.camera.name,
                "packed_preview": packed_preview,
                "cameras": sorted(ob.name for ob in group.objects),
                "solar_hour": 17,
                "build": "scripts/blender-buildings/present_beaver_house.py",
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    parser.add_argument(
        "--rendered-preview",
        type=Path,
        help="Pack the registered Cycles frontage render for the Rendering workspace",
    )
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    present(args.out, args.rendered_preview)
