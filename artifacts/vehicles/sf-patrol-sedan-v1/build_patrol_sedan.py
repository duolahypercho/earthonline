#!/usr/bin/env python3
"""Build the hand-authored EarthOnline SF patrol sedan in Blender.

This derives from the repository's editable, procedural project sedan and keeps
its named wheel, door, hood, trunk, glazing, cabin, and lamp parts. It adds an
original black-and-white patrol finish and modelled police equipment. No
text/image-to-3D output is used for the vehicle body.

Run from the repository root:
  blender -b artifacts/vehicles/project-sedan-v1/project-sedan-v1.blend \
    --python artifacts/vehicles/sf-patrol-sedan-v1/build_patrol_sedan.py --
"""

from __future__ import annotations

import hashlib
import json
import math
import os
import sys
from pathlib import Path

import bpy
import bmesh
from mathutils import Matrix, Vector


ASSET_DIR = Path(__file__).resolve().parent
REPO_ROOT = ASSET_DIR.parents[2]
BASE_BLEND = ASSET_DIR.parent / "project-sedan-v1" / "project-sedan-v1.blend"
TARGET_LENGTH_M = 4.95
TARGET_BODY_WIDTH_M = 1.90
TARGET_OVERALL_HEIGHT_M = 1.52
TARGET_WHEELBASE_M = 2.90
TARGET_WHEEL_RADIUS_M = 0.35
TARGET_UNIT_NUMBER = "2417"
FRONT_Y = -1.47
REAR_Y = 1.38
SOURCE_BODY_LENGTH_M = 4.72
SOURCE_OVERALL_LENGTH_M = 4.88
SOURCE_BODY_WIDTH_M = 1.84
SOURCE_WHEEL_RADIUS_M = 0.335
DRIVER_GLB = REPO_ROOT / "public/assets/human-npc-probe-v3/npc-01.glb"
MOTION_GLB = REPO_ROOT / "public/assets/motion/earthonline-humanoid-motion-pack-v1.glb"


def get_args():
    if "--" not in sys.argv:
        return ASSET_DIR / "out"
    tail = sys.argv[sys.argv.index("--") + 1 :]
    return Path(tail[0]).expanduser().resolve() if tail else ASSET_DIR / "out"


def set_socket(node, name, value):
    socket = node.inputs.get(name)
    if socket is not None:
        socket.default_value = value


def make_pbr(name, color, metallic=0.0, roughness=0.4, coat=0.0, coat_roughness=0.2, emission=None, emission_strength=0.0):
    mat = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    mat.diffuse_color = color
    mat.use_nodes = True
    nodes = mat.node_tree.nodes
    shader = next((node for node in nodes if node.type == "BSDF_PRINCIPLED"), None)
    if shader is None:
        shader = nodes.new("ShaderNodeBsdfPrincipled")
    set_socket(shader, "Base Color", color)
    set_socket(shader, "Metallic", metallic)
    set_socket(shader, "Roughness", roughness)
    set_socket(shader, "Coat Weight", coat)
    set_socket(shader, "Coat Roughness", coat_roughness)
    if emission is not None:
        set_socket(shader, "Emission Color", emission)
        set_socket(shader, "Emission", emission)
        set_socket(shader, "Emission Strength", emission_strength)
    mat["vehicle_material"] = True
    return mat


def tune_existing_material(name, color, metallic, roughness, coat=0.0, coat_roughness=0.2):
    mat = bpy.data.materials.get(name)
    if mat is None:
        return None
    mat.diffuse_color = color
    mat.use_nodes = True
    shader = next((node for node in mat.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
    if shader is not None:
        set_socket(shader, "Base Color", color)
        set_socket(shader, "Metallic", metallic)
        set_socket(shader, "Roughness", roughness)
        set_socket(shader, "Coat Weight", coat)
        set_socket(shader, "Coat Roughness", coat_roughness)
    return mat


def new_collection(name, parent=None):
    collection = bpy.data.collections.new(name)
    (parent or bpy.context.scene.collection).children.link(collection)
    return collection


def move_to_collection(obj, collection):
    for old_collection in list(obj.users_collection):
        old_collection.objects.unlink(obj)
    collection.objects.link(obj)


def preserve_parent(obj, parent):
    # Blender does not always flush a just-assigned location into matrix_world
    # until the view layer updates. Capture the evaluated world pose before
    # parenting so accessories created earlier in the same script keep their
    # intended location when their vehicle root is scaled later.
    bpy.context.view_layer.update()
    world = obj.matrix_world.copy()
    obj.parent = parent
    bpy.context.view_layer.update()
    obj.matrix_parent_inverse = parent.matrix_world.inverted()
    obj.matrix_world = world
    bpy.context.view_layer.update()


def add_box(name, dimensions, location, material, collection, bevel=0.0, parent=None):
    dx, dy, dz = (float(v) for v in dimensions)
    verts = [
        (-dx / 2, -dy / 2, -dz / 2),
        (dx / 2, -dy / 2, -dz / 2),
        (dx / 2, dy / 2, -dz / 2),
        (-dx / 2, dy / 2, -dz / 2),
        (-dx / 2, -dy / 2, dz / 2),
        (dx / 2, -dy / 2, dz / 2),
        (dx / 2, dy / 2, dz / 2),
        (-dx / 2, dy / 2, dz / 2),
    ]
    faces = [
        (0, 3, 2, 1),
        (4, 5, 6, 7),
        (0, 1, 5, 4),
        (1, 2, 6, 5),
        (2, 3, 7, 6),
        (3, 0, 4, 7),
    ]
    mesh = bpy.data.meshes.new(name + "_Mesh")
    mesh.from_pydata(verts, [], faces)
    if material is not None:
        mesh.materials.append(material)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    obj.location = location
    if bevel > 0:
        modifier = obj.modifiers.new("Manufactured edge radii", "BEVEL")
        modifier.width = bevel
        modifier.segments = 3
        modifier.limit_method = "ANGLE"
        modifier.angle_limit = math.radians(24)
        modifier.harden_normals = True
    if parent is not None:
        preserve_parent(obj, parent)
    return obj


def add_cylinder(name, radius, depth, location, material, collection, axis=(0, 0, 1), vertices=24, parent=None):
    axis = Vector(axis).normalized()
    tangent = axis.to_track_quat("Z", "Y")
    verts = []
    for z in (-depth / 2, depth / 2):
        for i in range(vertices):
            angle = 2 * math.pi * i / vertices
            verts.append((radius * math.cos(angle), radius * math.sin(angle), z))
    faces = [tuple(reversed(range(vertices))), tuple(range(vertices, 2 * vertices))]
    for i in range(vertices):
        j = (i + 1) % vertices
        faces.append((i, j, j + vertices, i + vertices))
    mesh = bpy.data.meshes.new(name + "_Mesh")
    mesh.from_pydata(verts, [], faces)
    mesh.materials.append(material)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    obj.location = location
    obj.rotation_mode = "QUATERNION"
    obj.rotation_quaternion = tangent
    for poly in mesh.polygons:
        poly.use_smooth = True
    if parent is not None:
        preserve_parent(obj, parent)
    return obj


def add_poly_tube(name, points, radius, material, collection, parent=None):
    curve = bpy.data.curves.new(name + "_Path", "CURVE")
    curve.dimensions = "3D"
    curve.resolution_u = 1
    curve.bevel_depth = radius
    curve.bevel_resolution = 3
    spline = curve.splines.new("POLY")
    spline.points.add(len(points) - 1)
    for point, value in zip(spline.points, points):
        point.co = (*value, 1.0)
    curve.materials.append(material)
    obj = bpy.data.objects.new(name, curve)
    collection.objects.link(obj)
    if parent is not None:
        preserve_parent(obj, parent)
    return obj


def add_flat_strap(name, points, width, material, collection, parent=None):
    path = [Vector(point) for point in points]
    vertices = []
    for index, point in enumerate(path):
        tangent = (path[min(index + 1, len(path) - 1)] - path[max(index - 1, 0)]).normalized()
        across = Vector((tangent.z, 0, -tangent.x)).normalized() * width * 0.5
        for depth in (-0.002, 0.002):
            for side in (-1, 1):
                vertices.append(tuple(point + across * side + Vector((0, depth, 0))))
    faces = []
    for index in range(len(path) - 1):
        a, b = index * 4, (index + 1) * 4
        faces.extend(((a, a + 1, b + 1, b), (a + 2, b + 2, b + 3, a + 3),
                      (a, b, b + 2, a + 2), (a + 1, a + 3, b + 3, b + 1)))
    faces.extend(((0, 2, 3, 1), (len(vertices) - 4, len(vertices) - 3, len(vertices) - 1, len(vertices) - 2)))
    mesh = bpy.data.meshes.new(name + "_Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.materials.append(material)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    if parent is not None:
        preserve_parent(obj, parent)
    return obj


def convert_curve_to_mesh(obj):
    if obj.type != "CURVE":
        return obj
    collection = obj.users_collection[0]
    old_world = obj.matrix_world.copy()
    old_name = obj.name
    old_materials = list(obj.data.materials)
    depsgraph = bpy.context.evaluated_depsgraph_get()
    mesh = bpy.data.meshes.new_from_object(obj.evaluated_get(depsgraph), depsgraph=depsgraph)
    mesh.name = old_name + "_Mesh"
    for material in old_materials:
        if material and material.name not in [m.name for m in mesh.materials if m]:
            mesh.materials.append(material)
    converted = bpy.data.objects.new(old_name, mesh)
    collection.objects.link(converted)
    converted.matrix_world = old_world
    if obj.parent:
        preserve_parent(converted, obj.parent)
    bpy.data.objects.remove(obj, do_unlink=True)
    return converted


def world_from_side(side, horizontal, vertical, center_y, center_z, x=0.0):
    return Vector((x, center_y + side * horizontal, center_z + vertical))


def world_bound_extent(obj, axis, direction):
    bpy.context.view_layer.update()
    points = world_geometry_points(obj)
    if not points:
        points = [obj.matrix_world @ Vector(corner) for corner in obj.bound_box]
    values = [point[axis] for point in points]
    return max(values) if direction > 0 else min(values)


def world_geometry_points(obj, depsgraph=None):
    """World-space geometry vertices; bound_box may be stale after parenting."""
    if obj.type == "MESH":
        return [obj.matrix_world @ vertex.co for vertex in obj.data.vertices]
    if obj.type in {"CURVE", "FONT"}:
        depsgraph = depsgraph or bpy.context.evaluated_depsgraph_get()
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        try:
            return [evaluated.matrix_world @ vertex.co for vertex in mesh.vertices]
        finally:
            evaluated.to_mesh_clear()
    return []


def side_basis(side):
    normal = Vector((side, 0, 0))
    up = Vector((0, 0, 1))
    horizontal = up.cross(normal)
    return Matrix((horizontal, up, normal)).transposed().to_quaternion()


def project_side_mesh(obj, side, surface, outward_offset=0.003):
    bpy.context.view_layer.update()
    inverse = obj.matrix_world.inverted()
    surface_inverse = surface.matrix_world.inverted()
    direction = (surface_inverse.to_3x3() @ Vector((-side, 0, 0))).normalized()
    def same_side_hit(y, z):
        origin = surface_inverse @ Vector((side * 1.7, y, z))
        hit, point, normal, _face_index = surface.ray_cast(origin, direction, distance=3.4)
        if hit and (surface.matrix_world @ point).x * side > 0.3:
            return point, normal
        return None
    for vertex in obj.data.vertices:
        world = obj.matrix_world @ vertex.co
        result = same_side_hit(world.y, world.z)
        if result is None:
            for delta in (0.025, -0.025, 0.05, -0.05, 0.1, -0.1, 0.15, -0.15, 0.2, -0.2):
                result = same_side_hit(world.y + delta, world.z)
                if result is not None:
                    break
        if result is not None:
            point, normal = result
            world = surface.matrix_world @ point
            world.y = (obj.matrix_world @ vertex.co).y
            world += (surface.matrix_world.to_3x3() @ normal).normalized() * outward_offset
            vertex.co = inverse @ world
    obj.data.update()
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(obj.data)
    bm.free()
    obj.data.update()


def make_rear_text(name, content, center_z, size, material, collection, parent=None):
    font = bpy.data.curves.new(name + "_Font", "FONT")
    font.body = content
    font.size = size
    font.align_x = "CENTER"
    font.align_y = "CENTER"
    font.extrude = 0.0
    font.bevel_depth = 0.0
    font.fill_mode = "BOTH"
    font.materials.append(material)
    obj = bpy.data.objects.new(name, font)
    collection.objects.link(obj)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.ops.object.convert(target="MESH")
    obj = bpy.context.object
    obj.name = name
    body_rear = world_bound_extent(parent or bpy.data.objects["Body"], 1, 1)
    plate = bpy.data.objects.get("PlateRear")
    plate_rear = world_bound_extent(plate, 1, 1) if plate is not None else body_rear
    rear_y = max(body_rear, plate_rear) + 0.004
    for vertex in obj.data.vertices:
        vertex.co = (-vertex.co.x, rear_y, center_z + vertex.co.y)
    obj.data.update()
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(obj.data)
    bm.free()
    if parent is not None:
        preserve_parent(obj, parent)
    return obj


def make_side_text(name, content, side, center_y, center_z, size, material, collection, parent=None, bold=False, plane_x=None, project=True):
    font = bpy.data.curves.new(name + "_Font", "FONT")
    font.body = content
    font.size = size
    if bold:
        bold_path = Path("/System/Library/Fonts/Supplemental/Arial Bold.ttf")
        if bold_path.is_file():
            font.font = bpy.data.fonts.load(str(bold_path))
    font.align_x = "CENTER"
    font.align_y = "CENTER"
    font.extrude = 0.0
    font.bevel_depth = 0.0
    font.fill_mode = "BOTH"
    font.materials.append(material)
    obj = bpy.data.objects.new(name, font)
    collection.objects.link(obj)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.ops.object.convert(target="MESH")
    obj = bpy.context.object
    obj.name = name
    side_x = side * plane_x if plane_x is not None else world_bound_extent(parent or bpy.data.objects["Body"], 0, side) + side * 0.004
    for vertex in obj.data.vertices:
        vertex.co = (side_x, center_y + side * vertex.co.x, center_z + vertex.co.y)
    obj.data.update()
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(obj.data)
    bm.free()
    if parent is not None:
        preserve_parent(obj, parent)
        if project:
            project_side_mesh(obj, side, parent, 0.004)
    return obj


def make_side_polygon(name, points_2d, side, center_y, center_z, material, collection, parent=None, offset=0.004):
    surface = parent or bpy.data.objects["Body"]
    side_x = world_bound_extent(surface, 0, side) + side * offset
    points = [world_from_side(side, x, z, center_y, center_z, side_x) for x, z in points_2d]
    vertices = [tuple(point) for point in points]
    center = world_from_side(side, 0, 0, center_y, center_z, side_x)
    vertices.append(tuple(center))
    center_index = len(vertices) - 1
    faces = []
    for i in range(len(points_2d)):
        faces.append((center_index, i, (i + 1) % len(points_2d)))
    mesh = bpy.data.meshes.new(name + "_Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.materials.append(material)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    if parent is not None:
        preserve_parent(obj, parent)
        project_side_mesh(obj, side, parent, offset)
    return obj


def star_points(outer_radius=0.095, inner_radius=0.054, points=6):
    result = []
    for i in range(points * 2):
        radius = outer_radius if i % 2 == 0 else inner_radius
        angle = math.pi / 2 + i * math.pi / points
        result.append((radius * math.cos(angle), radius * math.sin(angle)))
    return result


def circle_points(radius, segments=32):
    return [
        (radius * math.cos(2 * math.pi * i / segments), radius * math.sin(2 * math.pi * i / segments))
        for i in range(segments)
    ]


def assign_surface_material(obj, material, predicate):
    if obj.type != "MESH":
        return 0
    if material.name not in [m.name for m in obj.data.materials if m]:
        obj.data.materials.append(material)
    mat_index = next(i for i, m in enumerate(obj.data.materials) if m and m.name == material.name)
    changed = 0
    world_matrix = obj.matrix_world
    normal_matrix = world_matrix.to_3x3()
    side = -1 if "_L" in obj.name or obj.name.endswith("_L") else 1
    for poly in obj.data.polygons:
        center = world_matrix @ poly.center
        normal = (normal_matrix @ poly.normal).normalized()
        if predicate(center, normal, side):
            poly.material_index = mat_index
            changed += 1
    return changed


def set_material_on_object(obj, material):
    if obj.type != "MESH":
        return
    obj.data.materials.clear()
    obj.data.materials.append(material)
    for poly in obj.data.polygons:
        poly.material_index = 0


def scale_root_to_contract(root, car_collection):
    body = bpy.data.objects.get("Body")
    if body is None:
        raise RuntimeError("Base asset does not contain Body")
    root.scale = (TARGET_BODY_WIDTH_M / SOURCE_BODY_WIDTH_M, TARGET_LENGTH_M / SOURCE_OVERALL_LENGTH_M, 1.0)
    root.location.y = -(FRONT_Y + REAR_Y) * 0.5 * root.scale.y
    bpy.context.view_layer.update()

    def bounds(objects):
        points = []
        depsgraph = bpy.context.evaluated_depsgraph_get()
        for obj in objects:
            if obj.type not in {"MESH", "CURVE", "FONT"}:
                continue
            points.extend(world_geometry_points(obj, depsgraph))
        if not points:
            raise RuntimeError("No vehicle geometry found for measurement")
        mins = [min(p[i] for p in points) for i in range(3)]
        maxs = [max(p[i] for p in points) for i in range(3)]
        return [maxs[i] - mins[i] for i in range(3)], mins, maxs

    body_dims, _body_min, _body_max = bounds([body])
    root.scale.x *= TARGET_BODY_WIDTH_M / body_dims[0]
    current_dims, _minimum, _maximum = bounds(car_collection.all_objects)
    root.scale.y *= TARGET_LENGTH_M / current_dims[1]
    current_dims, _minimum, _maximum = bounds(car_collection.all_objects)
    root.scale.z *= TARGET_OVERALL_HEIGHT_M / current_dims[2]
    bpy.context.view_layer.update()

    for side, tag in ((-1, "L"), (1, "R")):
        for axle, y_world in (("F", -TARGET_WHEELBASE_M / 2), ("R", TARGET_WHEELBASE_M / 2)):
            name = f"Wheel_{axle}{tag}"
            wheel = bpy.data.objects.get(name)
            if wheel is None:
                continue
            wheel.location.y = (y_world - root.location.y) / root.scale.y
            wheel.location.z = (TARGET_WHEEL_RADIUS_M - root.location.z) / root.scale.z
            radial_y = (TARGET_WHEEL_RADIUS_M * 2) / (0.67 * root.scale.y)
            radial_z = (TARGET_WHEEL_RADIUS_M * 2) / (0.67 * root.scale.z)
            wheel.scale = (1.0, radial_y, radial_z)
    bpy.context.view_layer.update()
    return bounds


def install_metric_travel_compensators(car_collection, root):
    """Keep authored window and seat travel in metres under the scaled root."""
    def wrap(child_name, parent, blender_axis, factor):
        wrapper = bpy.data.objects.new(f"{child_name}_TravelCompensator", None)
        car_collection.objects.link(wrapper)
        wrapper.empty_display_type = "PLAIN_AXES"
        wrapper.empty_display_size = 0.04
        wrapper.parent = parent
        wrapper.matrix_parent_inverse = Matrix.Identity(4)
        scale = [1.0, 1.0, 1.0]
        scale[blender_axis] = factor
        wrapper.scale = scale
        wrapper["purpose"] = "counter vehicle-root axis scale so local travel remains metric"
        bpy.context.view_layer.update()
        preserve_parent(bpy.data.objects[child_name], wrapper)

    # The GLB axes map Blender Z to up (window travel) and Blender Y to forward
    # (seat travel). Parent wrappers cancel only those root-axis scales; the
    # child world matrices are preserved, so the resting geometry is unchanged.
    window_factor = 1.0 / root.scale.z
    for name in ("Window_FL", "Window_FR", "Window_RL", "Window_RR"):
        child = bpy.data.objects.get(name)
        if child is not None:
            wrap(name, child.parent, 2, window_factor)
    seat_factor = 1.0 / root.scale.y
    for name in ("Seat_FL", "Seat_FR"):
        child = bpy.data.objects.get(name)
        if child is not None:
            wrap(name, root, 1, seat_factor)


def create_police_finish(car_collection, root):
    # The source sedan's sculpted black character-line strips cross the white
    # patrol doors as a heavy stripe absent from the selected design.
    for obj in list(car_collection.all_objects):
        if obj.name.startswith("CharLine"):
            bpy.data.objects.remove(obj, do_unlink=True)
    black = tune_existing_material("VehiclePaint", (0.002, 0.003, 0.005, 1.0), 0.0, 0.30, 0.42, 0.16)
    if black is not None:
        black_shader = next((node for node in black.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
        if black_shader is not None:
            set_socket(black_shader, "IOR Level", 0.22)
    tune_existing_material("VehicleTrimBlack", (0.006, 0.008, 0.010, 1.0), 0.025, 0.43, 0.10, 0.28)
    tune_existing_material("VehicleAlloyDark", (0.024, 0.028, 0.034, 1.0), 0.62, 0.40, 0.08, 0.3)
    tune_existing_material("VehicleSeat", (0.025, 0.029, 0.034, 1.0), 0.02, 0.68, 0.0, 0.4)
    tune_existing_material("VehicleSeatInsert", (0.035, 0.040, 0.045, 1.0), 0.0, 0.72, 0.0, 0.4)
    white = make_pbr("PatrolPaint_White", (0.70, 0.73, 0.76, 1.0), 0.02, 0.42, 0.12, 0.30)
    gold = make_pbr("PatrolBadge_Gold", (0.50, 0.27, 0.065, 1.0), 0.64, 0.34, 0.18, 0.24)
    badge_ink = make_pbr("PatrolBadge_Enamel", (0.012, 0.023, 0.040, 1.0), 0.10, 0.38, 0.22, 0.24)
    wheel_steel = make_pbr("PatrolWheel_Steel", (0.065, 0.077, 0.090, 1.0), 0.82, 0.43, 0.08, 0.26)
    lens_red = make_pbr("PatrolEmergency_RedLens", (0.34, 0.014, 0.020, 1.0), 0.0, 0.08, 0.45, 0.08)
    lens_blue = make_pbr("PatrolEmergency_BlueLens", (0.010, 0.048, 0.28, 1.0), 0.0, 0.08, 0.45, 0.08)
    lens_clear = make_pbr("PatrolEmergency_ClearLens", (0.54, 0.61, 0.67, 1.0), 0.0, 0.08, 0.45, 0.08)
    for lens, alpha in ((lens_red, 0.47), (lens_blue, 0.47), (lens_clear, 0.29)):
        lens.surface_render_method = "BLENDED"
        shader = next(node for node in lens.node_tree.nodes if node.type == "BSDF_PRINCIPLED")
        set_socket(shader, "Alpha", alpha)
        set_socket(shader, "IOR", 1.49)
    led_red = make_pbr("PatrolEmergency_RedLED", (0.40, 0.008, 0.004, 1.0), 0.0, 0.13, 0.0, 0.2, (1.0, 0.018, 0.006, 1.0), 0.32)
    led_blue = make_pbr("PatrolEmergency_BlueLED", (0.005, 0.04, 0.36, 1.0), 0.0, 0.13, 0.0, 0.2, (0.010, 0.11, 1.0, 1.0), 0.32)
    led_white = make_pbr("PatrolEmergency_WhiteLED", (0.48, 0.57, 0.64, 1.0), 0.0, 0.13, 0.0, 0.2, (0.72, 0.84, 1.0, 1.0), 0.12)
    screen = make_pbr("PatrolEquipment_Screen", (0.005, 0.018, 0.028, 1.0), 0.18, 0.20, 0.35, 0.1, (0.018, 0.12, 0.19, 1.0), 0.3)
    polycarbonate = make_pbr("PatrolEquipment_ClearShield", (0.10, 0.14, 0.17, 0.35), 0.0, 0.16, 0.95, 0.05)
    metal = bpy.data.materials.get("VehicleSteel") or make_pbr("VehicleSteel", (0.20, 0.22, 0.25, 1.0), 0.80, 0.36)
    ink = make_pbr("PatrolDecal_Black", (0.009, 0.014, 0.020, 1.0), 0.0, 0.45, 0.0, 0.3)
    white_text = make_pbr("PatrolDecal_White", (0.88, 0.90, 0.93, 1.0), 0.0, 0.38, 0.0, 0.3)

    glass = bpy.data.materials.get("VehicleGlass")
    if glass is not None:
        glass.use_nodes = True
        glass.surface_render_method = "BLENDED"
        glass_shader = next((node for node in glass.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
        if glass_shader is not None:
            set_socket(glass_shader, "Base Color", (0.080, 0.115, 0.145, 1.0))
            set_socket(glass_shader, "Alpha", 0.38)
            set_socket(glass_shader, "Metallic", 0.0)
            set_socket(glass_shader, "Roughness", 0.075)
            set_socket(glass_shader, "Transmission Weight", 0.0)
            set_socket(glass_shader, "IOR", 1.49)
            set_socket(glass_shader, "Coat Weight", 0.36)
            set_socket(glass_shader, "Coat Roughness", 0.09)
    windshield_material = make_pbr("PatrolGlass_Windshield", (0.11, 0.145, 0.17, 1.0), 0.0, 0.075, 0.35, 0.09)
    windshield_material.surface_render_method = "BLENDED"
    windshield_shader = next(node for node in windshield_material.node_tree.nodes if node.type == "BSDF_PRINCIPLED")
    set_socket(windshield_shader, "Alpha", 0.20)
    set_socket(windshield_shader, "IOR", 1.49)
    set_material_on_object(bpy.data.objects["Windscreen"], windshield_material)

    door_paint_count = 0
    for name in ("Door_FL", "Door_FR", "Door_RL", "Door_RR"):
        obj = bpy.data.objects.get(name)
        if obj is None:
            raise RuntimeError(f"Missing articulated door {name}")
        door_paint_count += assign_surface_material(
            obj,
            white,
            lambda p, n, _side: (abs(n.x) > 0.76 and p.z > 0.28 and p.z < 0.89),
        )

    body = bpy.data.objects["Body"]
    fender_count = assign_surface_material(
        body,
        white,
        lambda p, n, _side: (abs(n.x) > 0.42 and -2.20 < p.y < -1.16 and 0.33 < p.z < 0.96),
    )

    for obj in list(car_collection.all_objects):
        if obj.type != "MESH":
            continue
        if obj.name.startswith("Wheel_") and any(token in obj.name for token in ("_Spoke", "_Lip", "_Hub", "_Cap")):
            set_material_on_object(obj, wheel_steel)
        elif obj.name.startswith("Wheel_") and "_Barrel" in obj.name:
            set_material_on_object(obj, bpy.data.materials["VehicleAlloyDark"])
        elif obj.name.startswith("Wheel_") and "_Lug" in obj.name:
            set_material_on_object(obj, metal)
        elif obj.name.startswith("Wheel_") and "_Caliper" in obj.name:
            set_material_on_object(obj, bpy.data.materials["VehicleAlloyDark"])

    root["asset_id"] = "sf-patrol-sedan-v1"
    root["asset_type"] = "hero_vehicle_source"
    root["scale_m_per_blender_unit"] = 1.0
    root["source_axis_forward"] = "-Y"
    root["source_axis_up"] = "+Z"
    root["gltf_axis_forward"] = "+Z"
    root["gltf_axis_up"] = "+Y"
    root["target_length_m"] = TARGET_LENGTH_M
    root["target_body_width_m"] = TARGET_BODY_WIDTH_M
    root["target_overall_height_m"] = TARGET_OVERALL_HEIGHT_M
    root["target_wheelbase_m"] = TARGET_WHEELBASE_M
    root["target_wheel_radius_m"] = TARGET_WHEEL_RADIUS_M
    root["unit_number"] = TARGET_UNIT_NUMBER
    root["livery_note"] = "Original black-and-white San Francisco patrol concept; generic six-point door crest."
    root["visual_reference_role"] = "Design intent only; not a geometry source."
    return {
        "black": black,
        "white": white,
        "gold": gold,
        "badge_ink": badge_ink,
        "wheel_steel": wheel_steel,
        "lens_red": lens_red,
        "lens_blue": lens_blue,
        "lens_clear": lens_clear,
        "led_red": led_red,
        "led_blue": led_blue,
        "led_white": led_white,
        "screen": screen,
        "polycarbonate": polycarbonate,
        "metal": metal,
        "ink": ink,
        "white_text": white_text,
        "door_paint_faces": door_paint_count,
        "front_fender_paint_faces": fender_count,
    }


def add_lightbar(materials, collection, root):
    black = bpy.data.materials["VehicleTrimBlack"]
    reflector = make_pbr("PatrolEmergency_Reflector", (0.54, 0.59, 0.64, 1.0), 0.78, 0.17, 0.35, 0.10)
    bar = bpy.data.objects.new("Lightbar_Root", None)
    collection.objects.link(bar)
    bar.empty_display_type = "CUBE"
    bar.empty_display_size = 0.05
    bar["part_role"] = "emergency_lightbar"
    preserve_parent(bar, root)

    # One sealed, low-profile blue/clear/red bar as seen in the design sheet.
    add_box("Lightbar_Base", (1.16, 0.255, 0.018), (0, 0.10, 1.472), black, collection, bevel=0.009, parent=bar)
    add_box("Lightbar_RearSeal", (1.14, 0.012, 0.048), (0, 0.228, 1.503), black, collection, bevel=0.004, parent=bar)
    add_box("Lightbar_FrontSeal", (1.14, 0.012, 0.048), (0, -0.028, 1.503), black, collection, bevel=0.004, parent=bar)
    for x in (-0.39, 0.39):
        add_box("Lightbar_Mount", (0.055, 0.09, 0.022), (x, 0.10, 1.454), materials["metal"], collection, bevel=0.007, parent=bar)
    for side, tag in ((-1, "Blue"), (1, "Red")):
        lens = materials["lens_blue"] if side < 0 else materials["lens_red"]
        diode = materials["led_blue"] if side < 0 else materials["led_red"]
        for index, distance in enumerate((0.13, 0.255, 0.38, 0.505)):
            x = side * distance
            add_box(f"Lightbar_{tag}_{index}_Reflector", (0.115, 0.195, 0.034), (x, 0.10, 1.501), reflector, collection, bevel=0.009, parent=bar)
            cover = add_box(f"Lightbar_{tag}_{index}_Lens", (0.12, 0.226, 0.046), (x, 0.10, 1.506), lens, collection, bevel=0.008, parent=bar)
            cover["switchable_lamp_material"] = diode.name
            for row, y in enumerate((0.024, 0.176)):
                for chip, dx in enumerate((-0.031, 0.0, 0.031)):
                    add_box(f"Lightbar_{tag}_{index}_LED_{row}_{chip}", (0.014, 0.009, 0.017), (x + dx, y, 1.506), diode, collection, bevel=0.002, parent=bar)
    add_box("Lightbar_Takedown_Reflector", (0.104, 0.19, 0.034), (0, 0.10, 1.501), reflector, collection, bevel=0.008, parent=bar)
    add_box("Lightbar_Takedown_Lens", (0.108, 0.226, 0.046), (0, 0.10, 1.506), materials["lens_clear"], collection, bevel=0.008, parent=bar)
    for y in (0.025, 0.175):
        for x in (-0.025, 0.025):
            add_box("Lightbar_Takedown_LED", (0.014, 0.009, 0.017), (x, y, 1.506), materials["led_white"], collection, bevel=0.002, parent=bar)


def add_pushbar(materials, collection, root):
    black = bpy.data.materials["VehicleTrimBlack"]
    y = -2.405
    # Continuous rounded steel loops preserve the bumper silhouette and mount at two frame points.
    add_poly_tube(
        "Pushbar_OuterLoop",
        [(-0.59, y + 0.02, 0.29), (-0.59, y, 0.68), (-0.52, y, 0.78), (0.52, y, 0.78), (0.59, y, 0.68), (0.59, y + 0.02, 0.29)],
        0.024,
        black,
        collection,
        root,
    )
    add_poly_tube("Pushbar_LowerRail", [(-0.55, y - 0.008, 0.34), (0.55, y - 0.008, 0.34)], 0.022, black, collection, root)
    add_poly_tube("Pushbar_UpperRail", [(-0.51, y, 0.66), (0.51, y, 0.66)], 0.020, black, collection, root)
    for x in (-0.25, 0.25):
        add_poly_tube(f"Pushbar_CenterBrace_{'L' if x < 0 else 'R'}", [(x, y, 0.34), (x, y - 0.004, 0.66)], 0.016, black, collection, root)
    for x in (-0.41, 0.41):
        add_box("Pushbar_FrameMount", (0.095, 0.19, 0.095), (x, -2.30, 0.42), materials["metal"], collection, bevel=0.012, parent=root)
        for z in (0.39, 0.45):
            add_cylinder("Pushbar_MountBolt", 0.009, 0.008, (x, -2.398, z), materials["metal"], collection, axis=(0, 1, 0), parent=root)


def add_spotlight(materials, collection, root):
    black = bpy.data.materials["VehicleTrimBlack"]
    chrome = bpy.data.materials.get("VehicleChrome") or materials["metal"]
    lens = make_pbr("PatrolSpotlight_Lens", (0.78, 0.79, 0.74, 1.0), 0.08, 0.11, 0.9, 0.06, (0.72, 0.75, 0.68, 1.0), 0.15)
    base = add_box("Spotlight_ApillarBase", (0.035, 0.09, 0.12), (-0.98, -0.73, 1.12), black, collection, bevel=0.014, parent=root)
    arm = add_poly_tube("Spotlight_SupportArm", [(-0.98, -0.73, 1.11), (-1.02, -0.62, 1.16), (-1.04, -0.51, 1.17)], 0.009, chrome, collection, root)
    head = add_cylinder("Spotlight_Housing", 0.065, 0.055, (-1.04, -0.51, 1.17), black, collection, axis=(0, -1, 0.12), vertices=32, parent=root)
    bezel = add_cylinder("Spotlight_Bezel", 0.050, 0.008, (-1.04, -0.543, 1.174), chrome, collection, axis=(0, -1, 0.12), vertices=32, parent=root)
    face = add_cylinder("Spotlight_Lens", 0.043, 0.004, (-1.04, -0.55, 1.175), lens, collection, axis=(0, -1, 0.12), vertices=32, parent=root)
    for obj in (base, arm, head, bezel, face):
        obj["part_role"] = "driver_side_spotlight"


def add_auxiliary_lamps(materials, collection, root):
    for side, tag in ((-1, "L"), (1, "R")):
        color = materials["led_blue"] if side < 0 else materials["led_red"]
        add_box(f"Grille_InterceptorLamp_{tag}", (0.12, 0.025, 0.055), (side * 0.37, -2.376, 0.51), color, collection, bevel=0.012, parent=root)
        add_box(f"RearDeck_InterceptorLamp_{tag}", (0.11, 0.035, 0.045), (side * 0.37, 1.93, 0.98), color, collection, bevel=0.01, parent=root)


def add_glazing_details(collection, root):
    """Model the bonded pane details that survive an ordinary game camera."""
    conductor = make_pbr("PatrolGlass_DefrosterConductor", (0.18, 0.10, 0.065, 1.0), 0.22, 0.46)
    rear = bpy.data.objects["RearWindow"]
    points = world_geometry_points(rear)
    zmin = min(p.z for p in points)
    zmax = max(p.z for p in points)
    ymin = min(p.y for p in points)
    ymax = max(p.y for p in points)
    xmax = max(abs(p.x) for p in points)
    for index in range(7):
        fraction = 0.19 + index * 0.105
        z = zmin + fraction * (zmax - zmin)
        y = ymax - fraction * (ymax - ymin) + 0.003
        half_width = xmax * (0.88 - 0.12 * fraction)
        line = add_poly_tube(
            f"RearGlass_Defroster_{index:02d}",
            [(-half_width, y, z), (half_width, y, z)],
            0.0014, conductor, collection, root,
        )
        line["part_role"] = "rear_glass_defroster"


def add_patrol_wheels(collection):
    """Replace the civilian five-spoke faces with stamped patrol steel discs."""
    for obj in list(collection.all_objects):
        if obj.name.startswith("Wheel_") and any(part in obj.name for part in ("_Spoke", "_Lip", "_Hub", "_Cap", "_Lug")):
            bpy.data.objects.remove(obj, do_unlink=True)
    steel = make_pbr("PatrolWheel_BlackStampedSteel", (0.012, 0.015, 0.019, 1.0), 0.60, 0.42, 0.08, 0.26)
    recess = make_pbr("PatrolWheel_VentRecess", (0.003, 0.004, 0.005, 1.0), 0.05, 0.9)
    cap_metal = make_pbr("PatrolWheel_HubcapSteel", (0.25, 0.27, 0.28, 1.0), 0.82, 0.31, 0.12, 0.2)
    for axle in ("F", "R"):
        for side, tag in ((-1, "L"), (1, "R")):
            wheel = bpy.data.objects[f"Wheel_{axle}{tag}"]
            hub = wheel.matrix_world.translation.copy()
            face_x = hub.x + side * 0.101
            name = f"Wheel_{axle}{tag}"
            backing = add_cylinder(f"{name}_SteelVentBacking", 0.235, 0.004,
                                   (face_x - side * 0.016, hub.y, hub.z), recess,
                                   collection, axis=(1, 0, 0), vertices=48, parent=wheel)
            disc = add_cylinder(f"{name}_StampedSteelDisc", 0.248, 0.022,
                                (face_x, hub.y, hub.z), steel, collection,
                                axis=(1, 0, 0), vertices=96)
            cutters = []
            for index in range(10):
                angle = 2 * math.pi * index / 10
                cutter = add_cylinder(f"{name}_VentCutter_{index}", 0.027, 0.060,
                                      (face_x, hub.y + math.cos(angle) * 0.169,
                                       hub.z + math.sin(angle) * 0.169),
                                      None, collection, axis=(1, 0, 0), vertices=16)
                cutters.append(cutter)
            bpy.ops.object.select_all(action="DESELECT")
            for cutter in cutters:
                cutter.select_set(True)
            bpy.context.view_layer.objects.active = cutters[0]
            bpy.ops.object.join()
            cutter_group = bpy.context.view_layer.objects.active
            cut = disc.modifiers.new("Stamped ventilation apertures", "BOOLEAN")
            cut.operation = "DIFFERENCE"
            cut.object = cutter_group
            bpy.ops.object.select_all(action="DESELECT")
            disc.select_set(True)
            bpy.context.view_layer.objects.active = disc
            bpy.ops.object.modifier_apply(modifier=cut.name)
            bpy.data.objects.remove(cutter_group, do_unlink=True)
            bevel = disc.modifiers.new("Pressed steel lip", "BEVEL")
            bevel.width = 0.004
            bevel.segments = 2
            bevel.limit_method = "ANGLE"
            bevel.angle_limit = math.radians(32)
            preserve_parent(disc, wheel)
            add_cylinder(f"{name}_HubcapRing", 0.080, 0.006,
                         (face_x + side * 0.014, hub.y, hub.z), cap_metal,
                         collection, axis=(1, 0, 0), vertices=48, parent=wheel)
            add_cylinder(f"{name}_Hubcap", 0.069, 0.019,
                         (face_x + side * 0.025, hub.y, hub.z), cap_metal,
                         collection, axis=(1, 0, 0), vertices=48, parent=wheel)
            for index in range(5):
                angle = 2 * math.pi * index / 5
                add_cylinder(f"{name}_LugNut_{index}", 0.009, 0.008,
                             (face_x + side * 0.015,
                              hub.y + math.cos(angle) * 0.090,
                              hub.z + math.sin(angle) * 0.090),
                             cap_metal, collection, axis=(1, 0, 0), vertices=12, parent=wheel)
            for part in (backing, disc):
                part["part_role"] = "patrol_steel_wheel_face"


def add_driver(materials, collection, root):
    """Fit a CC0-derived preview human to the driver's seat using the shared drive clip."""
    driver_path = Path(os.environ.get("SF_PATROL_DRIVER_GLB", DRIVER_GLB))
    if not driver_path.is_file() or not MOTION_GLB.is_file():
        raise FileNotFoundError(f"Driver preview or motion pack missing: {driver_path}, {MOTION_GLB}")

    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(driver_path))
    imported = [obj for obj in bpy.data.objects if obj not in before]
    rig = next((obj for obj in imported if obj.type == "ARMATURE"), None)
    body = next((obj for obj in imported if obj.type == "MESH" and obj.parent == rig), None)
    if rig is None or body is None:
        raise RuntimeError("The officer source requires one skinned human mesh and an armature")
    for obj in imported:
        if obj != rig and obj != body:
            bpy.data.objects.remove(obj, do_unlink=True)

    before_motion = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(MOTION_GLB))
    motion_objects = {obj.name: obj for obj in bpy.data.objects if obj not in before_motion}
    for obj in motion_objects.values():
        obj.animation_data_clear()
    bpy.context.scene.frame_set(0)
    bpy.context.view_layer.update()
    rest = {name: obj.matrix_world.copy() for name, obj in motion_objects.items()}
    action = bpy.data.actions.get("drive")
    if action is None:
        raise RuntimeError("Shared motion pack is missing the drive clip")
    for name, obj in motion_objects.items():
        slot = next((item for item in action.slots if item.identifier == "OB" + name), None)
        if slot is not None:
            obj.animation_data_create()
            obj.animation_data.action = action
            obj.animation_data.action_slot = slot
    bpy.context.scene.frame_set(12)
    for bone in rig.data.bones:
        joint = motion_objects.get(bone.name)
        if joint is None:
            continue
        rig.pose.bones[bone.name].matrix = joint.matrix_world @ rest[bone.name].inverted() @ bone.matrix_local
        bpy.context.view_layer.update()
    # The shared generic drive clip puts both feet under this sedan's floor.
    # Bend the shins into the actual footwell while preserving their full bone
    # lengths, pelvis contact, and the source body's 1.75 m physical height.
    for name, angle in (("LeftLeg", -90), ("RightLeg", -50)):
        shin = rig.pose.bones[name]
        knee = shin.head.copy()
        shin.matrix = Matrix.Translation(knee) @ Matrix.Rotation(math.radians(angle), 4, "X") @ Matrix.Translation(-knee) @ shin.matrix
        bpy.context.view_layer.update()
    spine = rig.pose.bones["Spine1"]
    thorax = spine.head.copy()
    spine.matrix = Matrix.Translation(thorax) @ Matrix.Rotation(math.radians(-20), 4, "X") @ Matrix.Translation(-thorax) @ spine.matrix
    bpy.context.view_layer.update()
    head = rig.pose.bones["Head"]
    neck = head.head.copy()
    head.matrix = Matrix.Translation(neck) @ Matrix.Rotation(math.radians(12), 4, "X") @ Matrix.Translation(-neck) @ head.matrix
    bpy.context.view_layer.update()
    upper_back = rig.pose.bones["Spine2"]
    settled = upper_back.matrix.copy()
    settled.translation.z -= 0.065
    upper_back.matrix = settled
    bpy.context.view_layer.update()
    for obj in motion_objects.values():
        bpy.data.objects.remove(obj, do_unlink=True)

    # The preview atlas remains on face and hands. Its garment alpha has cutout
    # holes, so the fitted uniform uses opaque cloth material instead.
    base_mat = body.data.materials[0]
    def uniform_variant(name, color, roughness):
        mat = base_mat.copy()
        mat.name = name
        shader = next((node for node in mat.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
        if shader:
            for link in list(mat.node_tree.links):
                if link.to_socket in (shader.inputs["Base Color"], shader.inputs["Alpha"]):
                    mat.node_tree.links.remove(link)
            set_socket(shader, "Base Color", color)
            set_socket(shader, "Alpha", 1.0)
            set_socket(shader, "Roughness", roughness)
            set_socket(shader, "Metallic", 0.0)
        return mat
    shirt = uniform_variant("Officer_NavyUniformShirt", (0.020, 0.046, 0.091, 1.0), 0.87)
    trousers = uniform_variant("Officer_NavyUniformTrousers", (0.018, 0.033, 0.058, 1.0), 0.92)
    boots = uniform_variant("Officer_BlackBoots", (0.010, 0.013, 0.018, 1.0), 0.58)
    for mat in (shirt, trousers, boots):
        body.data.materials.append(mat)
    # Split the skin shell at the uniform neckline before assigning materials.
    # A face-center cutoff on unsplit triangles makes a jagged cloth/skin edge.
    neck_z = 1.475
    bm = bmesh.new()
    bm.from_mesh(body.data)
    bmesh.ops.bisect_plane(
        bm,
        geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
        plane_co=Vector((0, 0, neck_z)),
        plane_no=Vector((0, 0, 1)),
        dist=0.00001,
    )
    bm.to_mesh(body.data)
    bm.free()
    body.data.update()
    for face in body.data.polygons:
        center = sum((body.data.vertices[i].co for i in face.vertices), Vector()) / len(face.vertices)
        influences = set()
        for vertex_index in face.vertices:
            for group in body.data.vertices[vertex_index].groups:
                if group.weight > 0.24:
                    influences.add(body.vertex_groups[group.group].name)
        hand = any("Hand" in name or "Thumb" in name for name in influences)
        face_or_hair = any(name in {"Head", "Jaw", "LeftEye", "RightEye"} for name in influences)
        if face_or_hair or (hand and center.z < 0.98):
            face.material_index = 0
        elif center.z < 0.17 or any("Foot" in name or "Toe" in name for name in influences):
            face.material_index = 3
        elif center.z < 0.94:
            face.material_index = 2
        else:
            face.material_index = 1
    # The underlying skin torso and shirt shells are separate islands. Make
    # the fitted jacket opaque below the bisected neckline, with skin above it.
    parents = list(range(len(body.data.vertices)))
    def component(index):
        while parents[index] != index:
            parents[index] = parents[parents[index]]
            index = parents[index]
        return index
    for edge in body.data.edges:
        a, b = edge.vertices
        parents[component(a)] = component(b)
    islands = {}
    for face in body.data.polygons:
        islands.setdefault(component(face.vertices[0]), []).append(face)
    for faces in islands.values():
        shirt_faces = sum(face.material_index == 1 for face in faces)
        if len(faces) > 1000 and shirt_faces > 400:
            for face in faces:
                center_z = sum(body.data.vertices[index].co.z for index in face.vertices) / len(face.vertices)
                face.material_index = 0 if center_z > neck_z else 1
    smoothing = body.modifiers.new("OfficerPreviewSubdivision", "SUBSURF")
    smoothing.levels = 1
    smoothing.render_levels = 1

    # The front seat bases are lowered by 0.14 m within their existing rail
    # envelope. This retains the source human's metric scale and roof clearance.
    rig.location = (-0.36, 0.12, -0.04)
    bpy.context.view_layer.update()
    occupant = bpy.data.objects.new("Occupant_FL", None)
    collection.objects.link(occupant)
    occupant.location = (-0.36, 0.12, -0.04)
    occupant["part_role"] = "removable_driver_preview"
    occupant["seat_id"] = "Seat_FL"
    occupant["source_glb"] = str(driver_path.relative_to(REPO_ROOT))
    occupant["source_status"] = "preview_only_not_production_authorized"
    preserve_parent(occupant, root)

    depsgraph = bpy.context.evaluated_depsgraph_get()
    baked_mesh = bpy.data.meshes.new_from_object(body.evaluated_get(depsgraph), preserve_all_data_layers=True, depsgraph=depsgraph)
    baked_mesh.name = "Officer_Driver_Seated_Mesh"
    if not baked_mesh.materials:
        for material in body.data.materials:
            baked_mesh.materials.append(material)
    posed = bpy.data.objects.new("Officer_Driver_Seated", baked_mesh)
    collection.objects.link(posed)
    posed.matrix_world = body.matrix_world.copy()
    posed["part_role"] = "seated_driver_body"
    preserve_parent(posed, occupant)

    source_collection = new_collection("Officer_SourceRig_PreviewOnly")
    source_collection.hide_render = True
    source_collection.hide_viewport = True
    for obj in (rig, body):
        move_to_collection(obj, source_collection)
        obj.hide_render = True
    rig.name = "Officer_SourceRig"
    body.name = "Officer_SourceSkinnedBody"

    # Small independent uniform details at source metre scale, attached to
    # the removable occupant rather than painted onto the vehicle body.
    navy = make_pbr("Officer_Equipment_Black", (0.009, 0.014, 0.022, 1.0), 0.0, 0.65)
    add_box("Officer_Nameplate", (0.078, 0.006, 0.020), (-0.27, 0.178, 0.96), materials["metal"], collection, bevel=0.002, parent=occupant)
    add_cylinder("Officer_Badge", 0.028, 0.004, (-0.44, 0.178, 0.96), materials["gold"], collection, axis=(0, -1, 0), parent=occupant)
    add_flat_strap("Officer_Seatbelt", [(-0.53, 0.215, 1.03), (-0.44, 0.16, 0.89), (-0.25, 0.10, 0.60)], 0.038, navy, collection, occupant)

    for image in bpy.data.images:
        if image.source == "FILE" and image.filepath:
            try:
                image.pack()
            except RuntimeError:
                pass
    return {
        "source": str(driver_path.relative_to(REPO_ROOT)),
        "sourceSha256": hashlib.sha256(driver_path.read_bytes()).hexdigest(),
        "motionSha256": hashlib.sha256(MOTION_GLB.read_bytes()).hexdigest(),
        "sourceHeightM": 1.75,
        "poseClip": "drive",
        "poseFrame": 12,
        "productionAuthorized": False,
    }


def add_prisoner_partition(materials, collection, root):
    black = bpy.data.materials["VehicleTrimBlack"]
    frame = materials["metal"]
    y = 0.62
    add_poly_tube("Partition_Frame_Top", [(-0.52, y, 1.22), (0.52, y, 1.22)], 0.014, frame, collection, root)
    add_poly_tube("Partition_Frame_Bottom", [(-0.52, y, 0.46), (0.52, y, 0.46)], 0.015, frame, collection, root)
    for side, tag in ((-1, "L"), (1, "R")):
        add_poly_tube(f"Partition_Frame_{tag}", [(side * 0.52, y, 0.46), (side * 0.52, y, 1.22)], 0.014, frame, collection, root)
    add_box("Partition_LowerShield", (0.93, 0.025, 0.22), (0, y, 0.59), black, collection, bevel=0.008, parent=root)
    add_box("Partition_ClearPanel", (0.89, 0.012, 0.27), (0, y - 0.012, 0.84), materials["polycarbonate"], collection, bevel=0.006, parent=root)
    # Fine grille is separate, named geometry so it can be simplified in lower LODs.
    for i in range(11):
        x = -0.46 + i * 0.092
        add_poly_tube(f"Partition_Grille_V_{i:02d}", [(x, y - 0.027, 0.47), (x, y - 0.027, 1.20)], 0.0027, frame, collection, root)
    for i in range(7):
        z = 0.49 + i * 0.115
        add_poly_tube(f"Partition_Grille_H_{i:02d}", [(-0.49, y - 0.028, z), (0.49, y - 0.028, z)], 0.0027, frame, collection, root)
    for side in (-1, 1):
        add_cylinder("Partition_MountBolt", 0.009, 0.012, (side * 0.47, y - 0.04, 0.53), materials["metal"], collection, axis=(0, 1, 0), parent=root)


def add_mobile_data_terminal(materials, collection, root):
    black = bpy.data.materials["VehicleTrimBlack"]
    screen = materials["screen"]
    add_poly_tube("MDT_MountArm", [(-0.08, -0.08, 0.83), (-0.10, -0.20, 0.99), (-0.10, -0.28, 1.02)], 0.012, materials["metal"], collection, root)
    frame = add_box("MDT_Frame", (0.33, 0.055, 0.22), (-0.10, -0.29, 1.02), black, collection, bevel=0.016, parent=root)
    display = add_box("MDT_Display", (0.294, 0.012, 0.182), (-0.10, -0.325, 1.025), screen, collection, bevel=0.009, parent=root)
    display.rotation_euler.x = math.radians(-15)
    frame.rotation_euler.x = math.radians(-15)
    frame["part_role"] = "mobile_data_terminal"
    display["part_role"] = "mobile_data_terminal_display"
    for i in range(4):
        add_box(f"MDT_StatusLight_{i}", (0.014, 0.004, 0.014), (-0.213 + i * 0.03, -0.333, 0.946), materials["lens_blue"], collection, bevel=0.003, parent=root)


def add_livery(materials, collection, root):
    body = bpy.data.objects["Body"]
    for side, tag in ((-1, "L"), (1, "R")):
        front_door = bpy.data.objects[f"Door_F{'L' if side < 0 else 'R'}"]
        make_side_text(f"DoorLettering_{tag}_SAN_FRANCISCO", "SAN FRANCISCO", side, 0.10, 0.745, 0.054, materials["ink"], collection, front_door, bold=True)
        make_side_text(f"DoorLettering_{tag}_POLICE", "POLICE", side, 0.10, 0.620, 0.137, materials["ink"], collection, front_door, bold=True)
        make_side_polygon(f"DoorCrest_{tag}_StarOutline", star_points(0.109, 0.061), side, -0.32, 0.675, materials["badge_ink"], collection, front_door, 0.004)
        make_side_polygon(f"DoorCrest_{tag}_Star", star_points(0.100, 0.056), side, -0.32, 0.675, materials["gold"], collection, front_door, 0.008)
        make_side_text(f"EmergencyMarking_{tag}_Label", "EMERGENCY", side, 1.80, 0.820, 0.039, materials["white_text"], collection, body, bold=True, plane_x=0.940, project=False)
        make_side_text(f"EmergencyMarking_{tag}_Number", "911", side, 1.80, 0.745, 0.079, materials["white_text"], collection, body, bold=True, plane_x=0.940, project=False)
    make_rear_text("RearLettering_SF_POLICE", "SAN FRANCISCO POLICE", 0.73, 0.052, materials["white_text"], collection, bpy.data.objects["Body"])
    make_rear_text("RearLettering_Unit", TARGET_UNIT_NUMBER, 0.554, 0.060, materials["ink"], collection, bpy.data.objects["Body"])


def make_collision_collection(root):
    coll = new_collection("Collision_Proxies")
    coll.hide_render = True
    coll.hide_viewport = True
    proxy = add_box("Collider_MainCabin", (1.64, 3.25, 0.74), (0, 0.05, 0.49), None, coll)
    proxy.display_type = "WIRE"
    proxy.hide_render = True
    proxy["collision_role"] = "approximate_chassis_hull"
    proxy["physics_owner"] = "runtime asset adapter; verify before shipping"
    for y, name in ((-2.25, "Collider_FrontBumper"), (2.25, "Collider_RearBumper")):
        ob = add_box(name, (1.58, 0.20, 0.28), (0, y, 0.40), None, coll)
        ob.display_type = "WIRE"
        ob.hide_render = True
        ob["collision_role"] = "bumper_contact"
    root["collision_proxy_note"] = "Art-source proxies only; runtime collision behavior is not wired by this Blender stage."


def add_surface_reference(image_path):
    if not image_path or not image_path.is_file():
        return None
    collection = new_collection("Reference_Design_Intent")
    collection.hide_render = True
    image = bpy.data.images.load(str(image_path), check_existing=True)
    image.pack()
    empty = bpy.data.objects.new("Reference_SF_Patrol_6View", None)
    collection.objects.link(empty)
    empty.empty_display_type = "IMAGE"
    empty.data = image
    empty.empty_display_size = 4.0
    empty.location = (7.0, 0.0, 1.0)
    empty.hide_render = True
    empty["reference_role"] = "design intent only; not geometry input"
    return image


def parent_top_level_vehicle_parts(car_collection, root):
    top_level = [obj for obj in list(car_collection.all_objects) if obj.parent is None]
    for obj in top_level:
        if obj == root:
            continue
        preserve_parent(obj, root)


def create_root(car_collection):
    root = bpy.data.objects.new("VehicleRoot", None)
    car_collection.objects.link(root)
    root.empty_display_type = "PLAIN_AXES"
    root.empty_display_size = 0.16
    parent_top_level_vehicle_parts(car_collection, root)
    return root


def create_review_rig(scene, review_collection):
    floor_mat = make_pbr("Review_Concrete", (0.090, 0.095, 0.102, 1.0), 0.0, 0.82, 0.0, 0.5)
    floor = add_box("ReviewGround", (28.0, 28.0, 0.06), (0, 0, -0.045), floor_mat, review_collection)
    floor["review_only"] = True

    world = bpy.data.worlds.new("Patrol Studio World") if scene.world is None else scene.world
    scene.world = world
    world.use_nodes = True
    bg = next((node for node in world.node_tree.nodes if node.type == "BACKGROUND"), None)
    if bg is not None:
        bg.inputs["Color"].default_value = (0.27, 0.29, 0.32, 1.0)
        bg.inputs["Strength"].default_value = 0.16

    camera_data = bpy.data.cameras.new("Patrol_Review_Camera")
    camera = bpy.data.objects.new("Patrol_Review_Camera", camera_data)
    review_collection.objects.link(camera)
    camera.data.lens = 52
    camera.data.clip_start = 0.05
    camera.data.clip_end = 150
    scene.camera = camera

    def area(name, location, energy, color, size, target):
        data = bpy.data.lights.new(name, "AREA")
        data.energy = energy
        data.color = color
        data.shape = "DISK"
        data.size = size
        obj = bpy.data.objects.new(name, data)
        review_collection.objects.link(obj)
        obj.location = location
        obj.rotation_euler = (Vector(target) - obj.location).to_track_quat("-Z", "Y").to_euler()
        return obj

    area("Studio_Key", (-4.5, -4.5, 6.3), 680, (0.91, 0.94, 1.0), 3.2, (0, 0, 0.8))
    area("Studio_Fill", (4.0, -0.5, 3.6), 240, (0.78, 0.84, 1.0), 4.0, (0, 0, 0.85))
    area("Studio_Rim", (1.3, 4.8, 4.8), 500, (1.0, 0.80, 0.66), 2.8, (0, 0, 0.9))
    area("Studio_Top", (-0.5, 0.5, 7.0), 180, (1.0, 1.0, 1.0), 3.4, (0, 0, 0.4))
    try:
        scene.render.engine = "BLENDER_EEVEE"
    except TypeError:
        scene.render.engine = "CYCLES"
    scene.render.resolution_x = 1600
    scene.render.resolution_y = 900
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.render.film_transparent = False
    if hasattr(scene, "eevee") and hasattr(scene.eevee, "taa_render_samples"):
        scene.eevee.taa_render_samples = 72
    scene.view_settings.view_transform = "AgX"
    looks = [item.identifier for item in scene.view_settings.bl_rna.properties["look"].enum_items]
    if "AgX - Medium High Contrast" in looks:
        scene.view_settings.look = "AgX - Medium High Contrast"
    return camera, floor


def aim_camera(camera, location, target, orthographic=False, ortho_scale=6.8, lens=52):
    camera.location = location
    camera.rotation_euler = (Vector(target) - camera.location).to_track_quat("-Z", "Y").to_euler()
    camera.data.type = "ORTHO" if orthographic else "PERSP"
    if orthographic:
        camera.data.ortho_scale = ortho_scale
    else:
        camera.data.lens = lens


def render_views(scene, camera, out_dir, material_map):
    out_dir.mkdir(parents=True, exist_ok=True)
    views = [
        ("sf-patrol-sedan-v1-front34.png", (-5.7, -7.1, 3.5), (0, 0, 0.79), False, 6.8),
        ("sf-patrol-sedan-v1-side.png", (-7.4, 0.0, 1.45), (0, 0, 0.78), True, 6.4),
        ("sf-patrol-sedan-v1-rear34.png", (5.5, 7.1, 3.4), (0, 0, 0.82), False, 6.8),
        ("sf-patrol-sedan-v1-top.png", (0.0, -0.1, 8.0), (0, 0, 0.35), True, 6.0),
    ]
    for filename, location, target, ortho, scale in views:
        aim_camera(camera, location, target, ortho, scale)
        scene.render.filepath = str(out_dir / filename)
        bpy.ops.render.render(write_still=True)

    floor = bpy.data.objects.get("ReviewGround")
    if floor is not None:
        floor.hide_render = True
    aim_camera(camera, (0.0, 0.0, -7.2), (0.0, 0.0, 0.65), True, 6.2)
    scene.render.filepath = str(out_dir / "sf-patrol-sedan-v1-underbody.png")
    bpy.ops.render.render(write_still=True)
    if floor is not None:
        floor.hide_render = False

    door_names = ("Door_FL", "Door_RL", "Door_FR", "Door_RR")
    door_rots = {name: bpy.data.objects[name].rotation_euler.copy() for name in door_names}
    bpy.data.objects["Door_FL"].rotation_euler.z = math.radians(61)
    bpy.data.objects["Door_RL"].rotation_euler.z = math.radians(51)
    bpy.data.objects["Door_FR"].rotation_euler.z = math.radians(-61)
    bpy.data.objects["Door_RR"].rotation_euler.z = math.radians(-51)
    bpy.context.view_layer.update()
    aim_camera(camera, (-5.3, -5.8, 2.5), (0, 0.03, 0.8), False, 6.8, 55)
    scene.render.filepath = str(out_dir / "sf-patrol-sedan-v1-doors-open.png")
    bpy.ops.render.render(write_still=True)
    aim_camera(camera, (-1.80, 0.12, 0.95), (-0.38, 0.17, 1.02), False, 6.8, 52)
    scene.render.filepath = str(out_dir / "sf-patrol-sedan-v1-driver-cabin.png")
    bpy.ops.render.render(write_still=True)
    for name, rotation in door_rots.items():
        bpy.data.objects[name].rotation_euler = rotation
    bpy.context.view_layer.update()

    light_strengths = {}
    for key in ("led_red", "led_blue", "led_white"):
        mat = material_map[key]
        shader = next(node for node in mat.node_tree.nodes if node.type == "BSDF_PRINCIPLED")
        socket = shader.inputs.get("Emission Strength")
        if socket is not None:
            light_strengths[key] = socket.default_value
            socket.default_value = 7.5 if key != "led_white" else 1.6
    studio_energies = {obj.name: obj.data.energy for obj in scene.objects if obj.type == "LIGHT" and obj.name.startswith("Studio_")}
    for name, energy in studio_energies.items():
        bpy.data.objects[name].data.energy = energy * 0.09
    background = next((node for node in scene.world.node_tree.nodes if node.type == "BACKGROUND"), None)
    old_background_strength = background.inputs["Strength"].default_value if background else None
    if background:
        background.inputs["Strength"].default_value = 0.035
    flash_lights = []
    for name, x, color in (("Review_BlueFlash", -0.46, (0.03, 0.13, 1.0)), ("Review_RedFlash", 0.46, (1.0, 0.018, 0.012))):
        data = bpy.data.lights.new(name, "POINT")
        data.energy = 105
        data.color = color
        data.shadow_soft_size = 0.22
        lamp = bpy.data.objects.new(name, data)
        scene.collection.objects.link(lamp)
        lamp.location = (x, 0.02, 1.70)
        flash_lights.append(lamp)
    aim_camera(camera, (-5.7, -7.1, 3.2), (0, 0, 0.82), False, 6.8, 52)
    scene.render.filepath = str(out_dir / "sf-patrol-sedan-v1-emergency-lights.png")
    bpy.ops.render.render(write_still=True)
    for lamp in flash_lights:
        bpy.data.objects.remove(lamp, do_unlink=True)
    for name, energy in studio_energies.items():
        bpy.data.objects[name].data.energy = energy
    if background and old_background_strength is not None:
        background.inputs["Strength"].default_value = old_background_strength
    for key, strength in light_strengths.items():
        shader = next(node for node in material_map[key].node_tree.nodes if node.type == "BSDF_PRINCIPLED")
        shader.inputs["Emission Strength"].default_value = strength


def measure(objects):
    bpy.context.view_layer.update()
    points = []
    depsgraph = bpy.context.evaluated_depsgraph_get()
    for obj in objects:
        if obj.type not in {"MESH", "CURVE", "FONT"} or obj.name.startswith("Collider_"):
            continue
        points.extend(world_geometry_points(obj, depsgraph))
    if not points:
        return [0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]
    mins = [min(p[i] for p in points) for i in range(3)]
    maxs = [max(p[i] for p in points) for i in range(3)]
    return [maxs[i] - mins[i] for i in range(3)], mins, maxs


def count_geometry(objects):
    meshes = [obj for obj in objects if obj.type == "MESH"]
    triangles = sum(max(0, len(poly.vertices) - 2) for obj in meshes for poly in obj.data.polygons)
    uv_objects = sum(bool(obj.data.uv_layers) for obj in meshes)
    return {"mesh_object_count": len(meshes), "polygon_triangle_estimate": triangles, "uv_mapped_mesh_count": uv_objects}


def save_contract(out_dir, report):
    contract = {
        "assetId": "sf-patrol-sedan-v1",
        "assetStage": "Blender source and unintegrated game export candidate",
        "scale": "1 Blender unit = 1 metre",
        "coordinateConvention": {"sourceBlender": {"up": "+Z", "forward": "-Y"}, "gltf": {"up": "+Y", "forward": "+Z"}},
        "dimensionsM": {
            "overallLength": report["bounds_m"]["length"],
            "overallWidthIncludingAccessories": report["bounds_m"]["width_including_mirrors_and_spotlight"],
            "bodyWidthExcludingMirrors": report["bounds_m"]["body_width_excluding_mirrors"],
            "overallHeight": report["bounds_m"]["height"],
            "targetLength": TARGET_LENGTH_M,
            "targetBodyWidth": TARGET_BODY_WIDTH_M,
            "targetOverallHeight": TARGET_OVERALL_HEIGHT_M,
            "wheelbase": TARGET_WHEELBASE_M,
            "wheelRadius": TARGET_WHEEL_RADIUS_M,
        },
        "runtimePartNodes": ["VehicleRoot", "Body", "Door_FL", "Door_FR", "Door_RL", "Door_RR", "Hood", "Trunk", "Wheel_FL", "Wheel_FR", "Wheel_RL", "Wheel_RR", "Window_FL", "Window_FR", "Window_RL", "Window_RR", "Occupant_FL"],
        "policeEquipment": ["Lightbar_Root", "Pushbar_OuterLoop", "Spotlight_Housing", "PrisonerPartition", "MDT_Frame"],
        "occupantPreview": report["driverPreview"],
        "driverFitM": report["driverFitM"],
        "knownScopeLimits": [
            "This is a Blender asset stage; doors, seats, removable occupant state, collision response, damage, LOD switching, and runtime lighting are not wired into gameplay here.",
            "The seated driver's CC0-derived source is a development preview, not a production-authorized character; the GLB contains a static posed derivative and the Blender scene retains the source rig.",
            "The source sedan is a hand-built generic design; this asset is a patrol concept, not a certified replica of a specific manufacturer or current department fleet vehicle.",
            "The repository's formal AAA visual gate currently records REJECT; no blind review round was performed for this asset.",
        ],
    }
    (out_dir / "asset-contract.json").write_text(json.dumps(contract, indent=2) + "\n")
    return contract


def export_glb(car_collection, root, filepath):
    bpy.ops.object.select_all(action="DESELECT")
    selected = list(car_collection.all_objects)
    for obj in selected:
        if obj.type in {"MESH", "EMPTY", "CURVE", "FONT"}:
            obj.select_set(True)
    bpy.context.view_layer.objects.active = root
    bpy.ops.export_scene.gltf(
        filepath=str(filepath),
        export_format="GLB",
        use_selection=True,
        export_apply=True,
        export_extras=True,
        export_cameras=False,
        export_lights=False,
        export_materials="EXPORT",
        export_yup=True,
        export_animations=True,
    )
    bpy.ops.object.select_all(action="DESELECT")


def main():
    if not BASE_BLEND.is_file():
        raise FileNotFoundError(f"Base Blender source missing: {BASE_BLEND}")
    source_path = Path(bpy.data.filepath).resolve() if bpy.data.filepath else None
    if source_path and source_path != BASE_BLEND.resolve():
        raise RuntimeError(f"Expected project-sedan source {BASE_BLEND}, got {source_path}")

    out_dir = get_args()
    renders_dir = out_dir / "renders"
    out_dir.mkdir(parents=True, exist_ok=True)
    car_collection = bpy.data.collections.get("Sedan")
    if car_collection is None:
        raise RuntimeError("Base file must contain the Sedan collection")
    car_collection.name = "SF_Patrol_Sedan_Assembly"

    # Drop only the base file's old preview camera/light objects. The source blend
    # is opened in a dedicated background process; no user's active scene is edited.
    car_object_names = {obj.name for obj in car_collection.all_objects}
    for obj in list(bpy.data.objects):
        if obj.name not in car_object_names:
            bpy.data.objects.remove(obj, do_unlink=True)
    for collection in list(bpy.data.collections):
        if collection != car_collection and collection.name != "SF_Patrol_Sedan_Assembly":
            if not collection.objects and not collection.children:
                bpy.data.collections.remove(collection)

    # White paint is assigned only to the exterior door skins and front-quarter
    # faces. The original doors remain separate and keep their hinge pivots.
    root = create_root(car_collection)
    materials = create_police_finish(car_collection, root)
    review_collection = new_collection("Review_Rig")
    add_lightbar(materials, car_collection, root)
    add_pushbar(materials, car_collection, root)
    add_spotlight(materials, car_collection, root)
    add_auxiliary_lamps(materials, car_collection, root)
    add_prisoner_partition(materials, car_collection, root)
    add_mobile_data_terminal(materials, car_collection, root)

    bounds_builder = scale_root_to_contract(root, car_collection)
    install_metric_travel_compensators(car_collection, root)
    for name in ("Seat_FL", "Seat_FR"):
        seat = bpy.data.objects[name]
        world = seat.matrix_world.copy()
        world.translation.z -= 0.14
        seat.matrix_world = world
    for name in ("Seat_FL_Rail", "Seat_FR_Rail"):
        rail = bpy.data.objects[name]
        world = rail.matrix_world.copy()
        world.translation.z += 0.06
        rail.matrix_world = world
    floor_pan = bpy.data.objects["FloorPan"]
    floor_world = floor_pan.matrix_world.copy()
    floor_world.translation.z -= 0.08
    floor_pan.matrix_world = floor_world
    bpy.context.view_layer.update()
    add_glazing_details(car_collection, root)
    driver_source = add_driver(materials, car_collection, root)
    # Place readable text and badges after final scaling so every decal projects
    # onto the actual exterior surface instead of floating over the door curvature.
    add_livery(materials, car_collection, root)
    add_surface_reference(Path(os.environ.get("SF_PATROL_REFERENCE_IMAGE", "")))
    make_collision_collection(root)

    camera, _floor = create_review_rig(bpy.context.scene, review_collection)
    bpy.context.scene.render.resolution_x = 1600
    bpy.context.scene.render.resolution_y = 900

    # Stabilize the tyre contact plane after all transforms and preserve the
    # published 2.90 m axle spacing in the source axes.
    for side, tag in ((-1, "L"), (1, "R")):
        for axle, y_world in (("F", -TARGET_WHEELBASE_M / 2), ("R", TARGET_WHEELBASE_M / 2)):
            wheel = bpy.data.objects[f"Wheel_{axle}{tag}"]
            wheel.location.y = (y_world - root.location.y) / root.scale.y
            wheel.location.z = (TARGET_WHEEL_RADIUS_M - root.location.z) / root.scale.z
            wheel.scale.y = (TARGET_WHEEL_RADIUS_M * 2) / (0.67 * root.scale.y)
            wheel.scale.z = (TARGET_WHEEL_RADIUS_M * 2) / (0.67 * root.scale.z)
    bpy.context.view_layer.update()
    add_patrol_wheels(car_collection)

    dims, minimum, maximum = measure(car_collection.all_objects)
    body_dims, _body_min, _body_max = bounds_builder([bpy.data.objects["Body"]])
    wheel_centers = {
        name: [round(v, 6) for v in bpy.data.objects[name].matrix_world.translation]
        for name in ("Wheel_FL", "Wheel_FR", "Wheel_RL", "Wheel_RR")
    }
    wheelbase_measured = abs(wheel_centers["Wheel_RL"][1] - wheel_centers["Wheel_FL"][1])
    dimensions_ok = (
        abs(dims[1] - TARGET_LENGTH_M) <= 0.05
        and abs(body_dims[0] - TARGET_BODY_WIDTH_M) <= 0.015
        and TARGET_BODY_WIDTH_M <= dims[0] <= 2.28
        and abs(dims[2] - TARGET_OVERALL_HEIGHT_M) <= 0.025
        and abs(wheelbase_measured - TARGET_WHEELBASE_M) <= 0.015
    )
    if not dimensions_ok:
        outliers = []
        for obj in car_collection.all_objects:
            if obj.type not in {"MESH", "CURVE", "FONT"}:
                continue
            corners = world_geometry_points(obj)
            if not corners:
                continue
            spans = [max(point[i] for point in corners) - min(point[i] for point in corners) for i in range(3)]
            outliers.append((max(spans), obj.name, [round(v, 4) for v in spans], [round(v, 4) for v in obj.matrix_world.translation]))
        outliers.sort(reverse=True)
        raise RuntimeError(
            "Dimensional contract failed: "
            f"bounds={dims}, wheelbase={wheelbase_measured:.4f} m, largest_objects={outliers[:12]}"
        )

    driver_points = world_geometry_points(bpy.data.objects["Officer_Driver_Seated"])
    head_top = max(driver_points, key=lambda point: point.z)
    floor_top = max(point.z for point in world_geometry_points(bpy.data.objects["FloorPan"]))
    seat_bottom = min(point.z for point in world_geometry_points(bpy.data.objects["Seat_FL_Base"]))
    shoes_bottom = min(point.z for point in driver_points)
    liner = bpy.data.objects["Headliner"]
    liner_inverse = liner.matrix_world.inverted()
    roof_hit, roof_point, _normal, _face = liner.ray_cast(
        liner_inverse @ Vector((head_top.x, head_top.y, 2.0)),
        (liner_inverse.to_3x3() @ Vector((0, 0, -1))).normalized(),
        distance=3.0,
    )
    if not roof_hit:
        raise RuntimeError("Driver headroom probe missed the roof liner")
    headroom = (liner.matrix_world @ roof_point).z - head_top.z
    driver_fit = {
        "headTopZ": round(head_top.z, 4),
        "roofLinerZAtHead": round((liner.matrix_world @ roof_point).z, 4),
        "headroom": round(headroom, 4),
        "seatBaseBottomZ": round(seat_bottom, 4),
        "cabinFloorTopZ": round(floor_top, 4),
        "seatFloorGap": round(seat_bottom - floor_top, 4),
        "shoeBottomZ": round(shoes_bottom, 4),
        "shoeFloorGap": round(shoes_bottom - floor_top, 4),
    }
    driver_fits = headroom >= 0.05 and seat_bottom >= floor_top - 0.005 and 0 <= shoes_bottom - floor_top <= 0.12
    if not driver_fits:
        raise RuntimeError(f"Driver cabin fit failed: {driver_fit}")

    glb_path = out_dir / "sf-patrol-sedan-v1.glb"
    export_glb(car_collection, root, glb_path)
    render_views(bpy.context.scene, camera, renders_dir, materials)
    # Save after rendering so the source opens in its neutral door pose and no
    # Blender .blend1 backup is emitted from an intermediate save.
    bpy.ops.wm.save_as_mainfile(filepath=str(out_dir / "sf-patrol-sedan-v1.blend"))

    geo = count_geometry(car_collection.all_objects)
    report = {
        "assetId": "sf-patrol-sedan-v1",
        "sourceBlend": str(BASE_BLEND.relative_to(REPO_ROOT)),
        "sourceBlendSha256": hashlib.sha256(BASE_BLEND.read_bytes()).hexdigest(),
        "blenderVersion": bpy.app.version_string,
        "scaleMPerBlenderUnit": 1.0,
        "bounds_m": {"length": round(dims[1], 6), "width_including_mirrors_and_spotlight": round(dims[0], 6), "body_width_excluding_mirrors": round(body_dims[0], 6), "height": round(dims[2], 6)},
        "boundsMin_m": [round(v, 6) for v in minimum],
        "boundsMax_m": [round(v, 6) for v in maximum],
        "wheelbase_m": round(wheelbase_measured, 6),
        "wheelRadius_m": TARGET_WHEEL_RADIUS_M,
        "wheelCenters_m": wheel_centers,
        "liveryPaintFaceCounts": {"whiteDoorFaces": materials["door_paint_faces"], "whiteFrontQuarterFaces": materials["front_fender_paint_faces"]},
        "geometry": geo,
        "driverPreview": driver_source,
        "driverFitM": driver_fit,
        "export": {"path": str(glb_path.relative_to(REPO_ROOT)), "sha256": hashlib.sha256(glb_path.read_bytes()).hexdigest(), "bytes": glb_path.stat().st_size},
        "sourceBlendOutput": str((out_dir / "sf-patrol-sedan-v1.blend").relative_to(REPO_ROOT)),
        "renderDirectory": str(renders_dir.relative_to(REPO_ROOT)),
        "gameStage": "Blender source + GLB candidate; runtime integration not performed",
        "checks": {"dimensions": "PASS", "driverCabinFit": "PASS", "assetBodyHandBuilt": True, "wheelNodesPreserved": True, "doorNodesPreserved": True},
    }
    (out_dir / "build-report.json").write_text(json.dumps(report, indent=2) + "\n")
    save_contract(out_dir, report)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
