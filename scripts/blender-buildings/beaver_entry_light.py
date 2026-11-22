"""Radiant output of the closed opal shade on the furnished porch."""

import math

import bpy
from mathutils import Vector


def fit_opal_emitter(scene, light):
    """Use the surface of the actual shade as the six-watt emitter."""
    shades = [
        ob
        for ob in scene.objects
        if ob.type == "MESH"
        and any(
            slot.material and slot.material.name.startswith("porch-opal@")
            for slot in ob.material_slots
        )
    ]
    if not shades:
        return None
    if len(shades) != 1:
        raise ValueError("Expected one installed opal porch shade")
    shade = shades[0]
    opal_slots = [
        i
        for i, slot in enumerate(shade.material_slots)
        if slot.material and slot.material.name.startswith("porch-opal@")
    ]
    if len(opal_slots) != 1:
        raise ValueError("Expected one opal material on the fixture")
    slot = opal_slots[0]
    evaluated = shade.evaluated_get(bpy.context.evaluated_depsgraph_get())
    mesh = evaluated.to_mesh()
    try:
        mesh.calc_loop_triangles()
        world = [evaluated.matrix_world @ v.co for v in mesh.vertices]
        moment = Vector((0, 0, 0))
        area = 0.0
        for tri in mesh.loop_triangles:
            if tri.material_index != slot:
                continue
            a, b, c = (world[i] for i in tri.vertices)
            weight = 0.5 * (b - a).cross(c - a).length
            area += weight
            moment += (a + b + c) * (weight / 3)
        if area <= 0 or light.data.energy <= 0:
            raise ValueError("The opal shade needs a surface and a powered lamp")
        centre = moment / area
    finally:
        evaluated.to_mesh_clear()
    radiance = light.data.energy / (math.pi * area)
    mat = shade.material_slots[slot].material
    shader = mat.node_tree.nodes.get("Principled BSDF")
    old_location = list(light.matrix_world.translation)
    # An outward Lambertian emitter integrates to pi times its radiance per
    # square metre. The casing and bracket cast shadows on this actual source.
    shader.inputs["Emission Color"].default_value = (*light.data.color, 1)
    shader.inputs["Emission Strength"].default_value = radiance
    transform = light.matrix_world.copy()
    transform.translation = centre
    light.matrix_world = transform
    light.hide_render = True
    light.hide_set(True)
    shade["radiant_power_w"] = light.data.energy
    shade["emitting_area_m2"] = area
    return {
        "mesh": shade.name,
        "material": mat.name,
        "light": light.name,
        "previous_light_origin": old_location,
        "centre": list(centre),
        "area_m2": area,
        "watts": light.data.energy,
        "radiance": radiance,
        "colour": list(light.data.color),
    }
