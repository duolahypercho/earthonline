"""Export the sewn blind and measure the light leaving both sides of its folds."""

import argparse
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from bake_beaver_floor_gloss import survey
from beaver_floor_gloss import SOURCES
from build_beaver_window_linen import CLOTH, GROUP, PHOTO, ROOT, digest


def export(scene, group, path):
    # Keep the source's two-sided cloth in the packed .blend. Generic glTF
    # viewers get the photographed PBR face and both sets of UV coordinates.
    flax = scene.objects[CLOTH].active_material
    nodes = flax.node_tree.nodes
    flax.node_tree.links.new(
        nodes["Principled BSDF"].outputs[0], nodes["Material Output"].inputs[0]
    )
    for o in scene.objects:
        o.select_set(False)
    for o in group.objects:
        o.select_set(True)
        o["source_part"] = o.name
    bpy.ops.export_scene.gltf(
        filepath=str(path),
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_texcoords=True,
        export_normals=True,
        export_extras=True,
    )


def run(destination):
    src = Path(bpy.data.filepath)
    meta = json.loads((src.parent / "window-linen.provenance.json").read_text())
    calibration = json.loads(
        (
            PHOTO / "furniture/photo/fitted-dining/dining-floor-light.provenance.json"
        ).read_text()
    )
    if (
        digest(src) != meta["sha256"]
        or meta["source_sha256"] != calibration["source_scene"]["sha256"]
        or destination.is_relative_to(ROOT)
    ):
        raise ValueError(
            "Expected the matching packed window variant and an external destination"
        )
    destination.mkdir(exist_ok=True, parents=True)
    scene = bpy.data.scenes["Dining · site daylight"]
    bpy.context.window.scene = scene
    grp = bpy.data.collections[GROUP]
    glb = destination / "sewn-roman-blind.glb"
    export(scene, grp, glb)
    bpy.ops.wm.open_mainfile(filepath=str(src))
    scene = bpy.data.scenes["Dining · site daylight"]
    bpy.context.window.scene = scene
    scene.frame_set(3)
    sun = next(
        o
        for o in scene.objects
        if o.type == "LIGHT" and o.data.type == "SUN" and not o.hide_render
    )
    site_sun = sun.rotation_euler.to_quaternion()
    next(n for n in scene.world.node_tree.nodes if n.type == "TEX_SKY").sun_disc = False
    sheets = [scene.objects[n] for n in (CLOTH, "Linen-covered headrail")]
    for o in scene.objects:
        o.select_set(False)
    for o in sheets:
        o.data.uv_layers.active = o.data.uv_layers["Light across folds"]
        o.select_set(True)
    bpy.context.view_layer.objects.active = sheets[0]
    mat = sheets[0].active_material
    # The transport chart covers centimetres per texel. A geometric normal
    # integrates light across a yarn bundle; the runtime supplies the fine
    # photographed weave separately. Sub-texel normal detail aliases in a bake.
    for node in mat.node_tree.nodes:
        if node.type in ("BSDF_PRINCIPLED", "BSDF_TRANSLUCENT"):
            for link in list(node.inputs["Normal"].links):
                mat.node_tree.links.remove(link)
    texture = mat.node_tree.nodes.new("ShaderNodeTexImage")
    mat.node_tree.nodes.active = texture
    w, h = 512, 112
    data = np.empty((8, 2, h, w, 3), np.float32)

    def capture(row):
        for side in range(2):
            for o in sheets:
                o.select_set(True)
            bpy.context.view_layer.objects.active = sheets[0]
            bpy.context.view_layer.update()
            image = bpy.data.images.new(
                "Light reflected and transmitted by the weave",
                w,
                h,
                alpha=False,
                float_buffer=True,
            )
            image.colorspace_settings.name = "Non-Color"
            texture.image = image
            scene.cycles.samples = 1152
            bpy.ops.object.bake(
                type="COMBINED",
                pass_filter={
                    "COLOR",
                    "DIRECT",
                    "INDIRECT",
                    "DIFFUSE",
                    "GLOSSY",
                    "TRANSMISSION",
                    "EMIT",
                },
                target="IMAGE_TEXTURES",
                margin=2,
                use_clear=True,
                uv_layer="Light across folds",
            )
            values = np.empty((h, w, 4), np.float32)
            image.pixels.foreach_get(values.ravel())
            bpy.data.images.remove(image)
            field = values[::-1, :, :3].copy()
            if row == 0:
                field *= calibration["fixture_units_per_source_unit"]
            if row == 2:
                field /= np.asarray(calibration["source_key_rgb"])
            if not np.isfinite(field).all() or field.max() <= 0:
                raise ValueError(f"Invalid cloth survey {row}/{side}")
            data[row, side] = field
            print(
                "Window transport",
                row,
                side,
                np.percentile(field, [50, 95, 100]),
                flush=True,
            )
            for o in sheets:
                o.data.flip_normals()
        if row == 6:
            scene.objects["Sunlight transfer"].rotation_euler = site_sun.to_euler()
            capture(7)

    survey(scene, None, calibration, None, points=[], source_ready=capture)
    raw = destination / "window-light.npz"
    np.savez_compressed(raw, light=data)
    info = {
        "source_scene": src.name,
        "source_scene_sha256": meta["sha256"],
        "parent_scene_sha256": meta["source_sha256"],
        "design_sha256": meta["design_image_sha256"],
        "sha256": digest(raw),
        "geometry": glb.name,
        "geometry_sha256": digest(glb),
        "sources": [*SOURCES, "site-noon-sun"],
        "site_sun_blender_xyz": list(site_sun @ Vector((0, 0, 1))),
        "house_sha256": calibration["house_sha256"],
        "credits": meta["materials"],
        "attribution": meta["attribution"],
        "license": meta["license"],
        "source_url": meta["source_url"],
    }
    raw.with_suffix(".provenance.json").write_text(json.dumps(info, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    run(args.output.resolve())
