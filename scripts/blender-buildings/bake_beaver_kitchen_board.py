"""Bake the fitted chopping board's original oak grain and bump onto its six faces."""

import argparse
import json
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Matrix

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bake_beaver_floor_gloss import ROOT, digest

WIDTHS = (340, 86, 86)
PLANES = ((1, 2), (0, 2), (0, 1))


def board_uv(mesh):
    pts = np.array([list(v.co) for v in mesh.vertices])
    lo, hi = pts.min(0), pts.max(0)
    size = hi - lo
    if size.min() <= 0:
        raise ValueError("Empty board bounds")
    uv = mesh.uv_layers.new(name="Board surface light")
    mesh.uv_layers.active = uv
    for face in mesh.polygons:
        n = np.asarray(face.normal)
        axis = int(np.abs(n).argmax())
        a, b = PLANES[axis]
        left = sum(WIDTHS[:axis])
        y = 256 * int(n[axis] > 0)
        for loop in face.loop_indices:
            p = (pts[mesh.loops[loop].vertex_index] - lo) / size
            uv.data[loop].uv = (
                (left + 2.5 + (WIDTHS[axis] - 5) * p[a]) / 512,
                (y + 2.5 + 251 * p[b]) / 512,
            )
    return uv, lo, size


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("portable", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    if args.output.resolve().is_relative_to(ROOT):
        parser.error("Write the raw board source outside the checkout")
    args.output.mkdir(exist_ok=True, parents=True)
    recipe = json.loads((args.portable / "portable-dining.provenance.json").read_text())
    if digest(Path(bpy.data.filepath)) != recipe["source_scene_sha256"]:
        raise ValueError("The fitted kitchen scene differs from the portable model")
    scene = bpy.data.scenes["Dining · site daylight"]
    bpy.context.window.scene = scene
    scene.frame_set(3)
    board = next(
        obj for obj in scene.objects["chopping-board"].children if obj.type == "MESH"
    )
    for obj in scene.objects:
        if obj.type == "MESH" and obj != board:
            obj.hide_render = True
    bpy.ops.object.select_all(action="DESELECT")
    board.select_set(True)
    bpy.context.view_layer.objects.active = board
    mesh = board.data
    _, lo, size = board_uv(mesh)
    mat = board.active_material
    tree = mat.node_tree
    shader = next(n for n in tree.nodes if n.type == "BSDF_PRINCIPLED")
    output = next(n for n in tree.nodes if n.type == "OUTPUT_MATERIAL")
    pigment = shader.inputs["Base Color"].links[0].from_socket
    if not shader.inputs["Normal"].is_linked:
        raise ValueError("Missing original oak bump")
    emit = tree.nodes.new("ShaderNodeEmission")
    tree.links.new(pigment, emit.inputs["Color"])
    tree.links.new(emit.outputs[0], output.inputs["Surface"])
    texture = tree.nodes.new("ShaderNodeTexImage")
    tree.nodes.active = texture
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 8

    def bake(mode):
        image = bpy.data.images.new(
            "Oak source " + mode, 512, 512, alpha=False, float_buffer=True
        )
        image.colorspace_settings.name = "Non-Color"
        texture.image = image
        bpy.ops.object.bake(
            type=mode,
            target="IMAGE_TEXTURES",
            margin=4,
            margin_type="EXTEND",
            use_clear=True,
            normal_space="TANGENT",
        )
        values = np.empty((512, 512, 4), np.float32)
        image.pixels.foreach_get(values.ravel())
        bpy.data.images.remove(image)
        return values[::-1, :, :3].copy()

    pigment = bake("EMIT")
    tree.links.new(shader.outputs[0], output.inputs["Surface"])
    normals = bake("NORMAL")
    normals[:, :, 1] = 1 - normals[:, :, 1]
    parcel = Matrix(recipe["placement"]["parcel_from_assembled_gltf"])
    up = Matrix(((1, 0, 0, 0), (0, 0, 1, 0), (0, -1, 0, 0), (0, 0, 0, 1)))
    local_from_parcel = (parcel @ up @ board.matrix_world).inverted()
    path = args.output / "kitchen-board-survey.npz"
    np.savez_compressed(path, pigment=pigment, normals=normals)
    notice = {
        "file": path.name,
        "sha256": digest(path),
        "source_scene_sha256": recipe["source_scene_sha256"],
        "source_glb_sha256": recipe["parts"]["fitted-room"]["sha256"],
        "material": mat.name,
        "source_part": board.name,
        "local_from_parcel": [list(r) for r in local_from_parcel][:3],
        "min": lo.tolist(),
        "size": size.tolist(),
        "widths": WIDTHS,
        "roughness": shader.inputs["Roughness"].default_value,
        "camera_glare": 0.0,
        "license": "Project-authored wood material",
        "attribution": "Earth Online",
    }
    if scene.render.use_compositing and scene.compositing_node_group:
        raise ValueError("Inspect the fitted kitchen's changed camera compositor")
    path.with_suffix(".provenance.json").write_text(json.dumps(notice, indent=2) + "\n")


if __name__ == "__main__":
    main()
