"""Bake the tabletop's object-space ring shader for portable, UV-based materials."""

import bpy
from mathutils import Matrix


def bake_end_grain(scene, mesh):
    end_slots = [
        i
        for i, mat in enumerate(mesh.materials)
        if mat and mat.name.startswith("Dining tabletop · end grain")
    ]
    if not end_slots:
        return
    (slot,) = end_slots
    source = mesh.materials[slot]
    lo = [min(v.co[k] for v in mesh.vertices) for k in range(3)]
    hi = [max(v.co[k] for v in mesh.vertices) for k in range(3)]
    # Only the cut faces use these coordinates. Their opposite ends occupy
    # separate halves because the distortion noise also samples object X.
    layer = mesh.uv_layers[0]
    for face in mesh.polygons:
        if face.material_index != slot:
            continue
        for loop in face.loop_indices:
            p = mesh.vertices[mesh.loops[loop].vertex_index].co
            y = (p.y - lo[1]) / (hi[1] - lo[1])
            z = (p.z - lo[2]) / (hi[2] - lo[2])
            layer.data[loop].uv = (
                0.012 + 0.476 * y + 0.5 * (face.normal.x > 0),
                0.06 + 0.88 * z,
            )
    # Bake the very same shader on a temporary copy at its local origin.
    # Feeding its base colour into emission isolates wood from room lighting.
    work = mesh.copy()
    obj = bpy.data.objects.new("End grain bake", work)
    scene.collection.objects.link(obj)
    obj.matrix_world = Matrix.Identity(4)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    work.uv_layers.active_index = 0
    work.uv_layers[0].active_render = True
    image = bpy.data.images.new(
        source.name + " - cut faces", width=256, height=64, alpha=False
    )
    image.colorspace_settings.name = "sRGB"
    dummy = bpy.data.images.new("Discarded long grain", width=8, height=8)
    bake_mats = []
    for i, mat in enumerate(list(work.materials)):
        temp = mat.copy()
        bake_mats.append(temp)
        work.materials[i] = temp
        tree = temp.node_tree
        target = tree.nodes.new("ShaderNodeTexImage")
        target.image = image if i == slot else dummy
        tree.nodes.active = target
        if i == slot:
            color = (
                tree.nodes["Principled BSDF"].inputs["Base Color"].links[0].from_socket
            )
            emission = tree.nodes.new("ShaderNodeEmission")
            tree.links.new(color, emission.inputs["Color"])
            tree.links.new(
                emission.outputs[0], tree.nodes["Material Output"].inputs["Surface"]
            )
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 1
    bpy.ops.object.bake(type="EMIT", margin=2, use_clear=True)
    image.pack()
    finish = source.copy()
    tree = finish.node_tree
    coords = tree.nodes.new("ShaderNodeUVMap")
    coords.uv_map = layer.name
    pigment = tree.nodes.new("ShaderNodeTexImage")
    pigment.image = image
    tree.links.new(coords.outputs["UV"], pigment.inputs["Vector"])
    tree.links.new(
        pigment.outputs["Color"], tree.nodes["Principled BSDF"].inputs["Base Color"]
    )
    mesh.materials[slot] = finish
    bpy.data.objects.remove(obj, do_unlink=True)
    bpy.data.meshes.remove(work)
    bpy.data.images.remove(dummy)
    for mat in bake_mats:
        bpy.data.materials.remove(mat)
