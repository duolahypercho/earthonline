"""Bake the original end-grain material in its authored object coordinates."""

import bpy
import numpy as np


def capture_end_grain(boards, parcel_inverse, z_min, z_max, bottom, top):
    """Project the two physical saw cuts independently into padded 512 × 48 strips."""
    bpy.ops.object.select_all(action="DESELECT")
    target = bpy.data.images.new(
        "Eight end-grain faces", 512, 96, alpha=True, float_buffer=True
    )
    target.colorspace_settings.name = "Non-Color"
    for board in boards:
        source = board.data
        cut = source.materials[1].copy()
        graph = cut.node_tree
        base = graph.nodes["Principled BSDF"].inputs["Base Color"].links[0].from_socket
        emit = graph.nodes.new("ShaderNodeEmission")
        graph.links.new(base, emit.inputs[0])
        graph.links.new(
            emit.outputs[0], graph.nodes["Material Output"].inputs["Surface"]
        )
        bake = graph.nodes.new("ShaderNodeTexImage")
        bake.image = target
        graph.nodes.active = bake
        polygons = [p for p in source.polygons if p.material_index == 1]
        if not polygons or any(abs(p.normal.x) <= 0.86 for p in polygons):
            raise ValueError(
                "Dining cut-face material no longer follows the original saw cuts"
            )
        mesh = bpy.data.meshes.new("Saw cuts")
        mesh.from_pydata(
            [v.co[:] for v in source.vertices], [], [p.vertices[:] for p in polygons]
        )
        mesh.update()
        mesh.materials.append(cut)
        obj = bpy.data.objects.new(board.name + " cut-face receiver", mesh)
        bpy.context.scene.collection.objects.link(obj)
        # Object coordinates in the Wave texture must stay in the board's local axes.
        obj.matrix_world = board.matrix_world.copy()
        to_parcel = parcel_inverse @ board.matrix_world
        points = [to_parcel @ v.co for v in source.vertices]
        mid_x = (min(p.x for p in points) + max(p.x for p in points)) * 0.5
        uv = mesh.uv_layers.new(name="End face in parcel metres")
        for poly in mesh.polygons:
            x_mean = sum(points[i].x for i in poly.vertices) / len(poly.vertices)
            row = 48 if x_mean > mid_x else 0
            for loop in poly.loop_indices:
                v = points[mesh.loops[loop].vertex_index]
                u = (-v.y - z_min) / (z_max - z_min)
                h = (v.z - bottom) / (top - bottom)
                uv.data[loop].uv = ((4 + 504 * u) / 512, (row + 4 + 40 * h) / 96)
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
    bpy.context.scene.cycles.samples = 4
    bpy.ops.object.bake(
        type="EMIT",
        target="IMAGE_TEXTURES",
        margin_type="EXTEND",
        margin=4,
        use_clear=True,
    )
    raw = np.empty(512 * 96 * 4, np.float32)
    target.pixels.foreach_get(raw)
    color = raw.reshape(96, 512, 4)[:, :, :3]
    for page in (color[4:44, 4:-4], color[52:92, 4:-4]):
        if (
            not np.isfinite(page).all()
            or np.mean(page[:, :, 0] > 0) < 0.98
            or np.ptp(page) < 0.02
        ):
            raise ValueError("Incomplete Blender end-grain material bake")
    return np.rint(255 * np.sqrt(np.clip(color, 0, 1))).astype(np.uint8)
