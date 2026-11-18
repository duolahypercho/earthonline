"""Hand-authored police helicopter for EarthOnline, built headless in Blender.

    blender -b --factory-startup --python artifacts/vehicles/sf-police-helicopter-v1/build_police_helicopter.py -- [out_dir]

A generic light single-engine utility helicopter in a police air-support
livery. Every surface is authored here from numbers: the fuselage is lofted
through superellipse cross-sections and smoothed by subdivision, the tail,
skids, rotors and searchlight are modelled parts. No text/image-to-3D
generator touched this asset.

Frame (same as the fleet vehicles): 1 Blender unit = 1 m, +Z up, nose toward
-Y. The glTF export turns that into +Y up, nose toward +Z.

Named nodes the runtime drives (src/app/police-helicopter-runtime.js):
  MainRotor        spins about its local up axis (hub at the origin)
  TailRotor        spins about its local X axis
  Searchlight      gimbal; its local -Y (Blender) / +Z (glTF) is the beam
  Light_Beacon_Top, Light_Beacon_Belly, Light_Nav_L, Light_Nav_R, Light_Strobe_Tail
"""

import json
import math
import sys
from pathlib import Path

import bmesh
import bpy
from mathutils import Vector

HERE = Path(__file__).resolve().parent
NAME = "sf-police-helicopter-v1"


def out_dir():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    path = Path(argv[0]) if argv else HERE / "out"
    path.mkdir(parents=True, exist_ok=True)
    return path


# ---------------------------------------------------------------- materials
def material(name, color, rough=0.4, metal=0.0, alpha=1.0, emit=None, emit_strength=0.0, coat=0.0):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nodes = mat.node_tree.nodes
    bsdf = next(n for n in nodes if n.type == "BSDF_PRINCIPLED")
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Roughness"].default_value = rough
    bsdf.inputs["Metallic"].default_value = metal
    if "Coat Weight" in bsdf.inputs:
        bsdf.inputs["Coat Weight"].default_value = coat
    if alpha < 1.0:
        bsdf.inputs["Alpha"].default_value = alpha
        try:
            mat.surface_render_method = "BLENDED"
        except (AttributeError, TypeError):
            pass
    if emit is not None:
        bsdf.inputs["Emission Color"].default_value = (*emit, 1.0)
        bsdf.inputs["Emission Strength"].default_value = emit_strength
    mat.diffuse_color = (*color, alpha)
    return mat


MATS = {}


def build_materials():
    MATS["navy"] = material("Paint_Navy", (0.012, 0.022, 0.055), rough=0.32, metal=0.25, coat=0.6)
    MATS["white"] = material("Paint_White", (0.80, 0.81, 0.83), rough=0.28, metal=0.05, coat=0.6)
    MATS["accent"] = material("Paint_Accent", (0.03, 0.22, 0.78), rough=0.3, metal=0.15, coat=0.4)
    MATS["glass"] = material("Glass_Canopy", (0.015, 0.025, 0.035), rough=0.04, metal=0.0, alpha=0.78)
    MATS["metal"] = material("Metal_Dark", (0.06, 0.065, 0.07), rough=0.42, metal=0.85)
    MATS["steel"] = material("Metal_Bright", (0.55, 0.56, 0.58), rough=0.25, metal=1.0)
    MATS["rotor"] = material("Rotor_Blade", (0.035, 0.037, 0.04), rough=0.55, metal=0.3)
    MATS["tip"] = material("Rotor_Tip", (0.85, 0.62, 0.04), rough=0.45, metal=0.0)
    MATS["rubber"] = material("Rubber_Black", (0.015, 0.015, 0.015), rough=0.85)
    MATS["interior"] = material("Interior_Dark", (0.03, 0.03, 0.032), rough=0.8)
    MATS["screen"] = material("Interior_Screen", (0.02, 0.04, 0.05), rough=0.2, emit=(0.15, 0.55, 0.65), emit_strength=1.5)
    MATS["lens"] = material("Searchlight_Lens", (0.9, 0.92, 0.95), rough=0.05, emit=(1.0, 0.96, 0.86), emit_strength=6.0)
    MATS["red"] = material("Light_Red", (0.6, 0.02, 0.02), rough=0.2, emit=(1.0, 0.05, 0.03), emit_strength=8.0)
    MATS["green"] = material("Light_Green", (0.02, 0.5, 0.1), rough=0.2, emit=(0.05, 1.0, 0.25), emit_strength=8.0)
    MATS["strobe"] = material("Light_White", (0.9, 0.9, 0.9), rough=0.2, emit=(1.0, 1.0, 1.0), emit_strength=8.0)
    MATS["decal"] = material("Livery_Lettering", (0.92, 0.93, 0.95), rough=0.35)


# ---------------------------------------------------------------- helpers
def new_object(name, mesh, parent=None):
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    if parent is not None:
        obj.parent = parent
    return obj


def empty(name, location, parent=None):
    obj = bpy.data.objects.new(name, None)
    obj.location = location
    bpy.context.scene.collection.objects.link(obj)
    if parent is not None:
        obj.parent = parent
    return obj


def add_mats(obj, keys):
    for key in keys:
        obj.data.materials.append(MATS[key])


def smooth(obj, levels=2, angle=None):
    obj.data.shade_smooth() if hasattr(obj.data, "shade_smooth") else None
    for poly in obj.data.polygons:
        poly.use_smooth = True
    if levels:
        mod = obj.modifiers.new("Subsurf", "SUBSURF")
        mod.levels = levels
        mod.render_levels = levels


def lerp(a, b, t):
    return a + (b - a) * t


def catmull(p0, p1, p2, p3, t):
    t2, t3 = t * t, t * t * t
    return 0.5 * ((2 * p1) + (-p0 + p2) * t + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t2 + (-p0 + 3 * p1 - 3 * p2 + p3) * t3)


def resample(stations, per_span):
    """Catmull-Rom through station tuples (every field interpolated)."""
    out = []
    n = len(stations)
    for i in range(n - 1):
        p0 = stations[max(0, i - 1)]
        p1, p2 = stations[i], stations[i + 1]
        p3 = stations[min(n - 1, i + 2)]
        for k in range(per_span):
            t = k / per_span
            out.append(tuple(catmull(p0[j], p1[j], p2[j], p3[j], t) for j in range(len(p1))))
    out.append(stations[-1])
    return out


def superellipse_ring(a, zb, zt, n_top, n_bot, count, cx=0.0):
    zc = (zt + zb) / 2.0
    b = (zt - zb) / 2.0
    pts = []
    for k in range(count):
        t = 2 * math.pi * k / count
        c, s = math.cos(t), math.sin(t)
        n = n_top if s >= 0 else n_bot
        x = cx + a * math.copysign(abs(c) ** (2.0 / n), c)
        z = zc + b * math.copysign(abs(s) ** (2.0 / n), s)
        pts.append((x, z))
    return pts


def loft(name, stations, ring=32, per_span=3, mat_keys=("navy",), mat_for=None, cap=True, parent=None, sub=2, cuts=()):
    """stations: (y, half_width, z_bottom, z_top, n_top, n_bottom). `cuts` are (point, normal)
    planes bisected through the shell first, so livery and glazing edges run straight."""
    rows = resample(stations, per_span)
    bm = bmesh.new()
    rings = []
    for (y, a, zb, zt, nt, nb) in rows:
        ring_pts = superellipse_ring(max(a, 0.004), zb, max(zt, zb + 0.008), nt, nb, ring)
        rings.append([bm.verts.new((x, y, z)) for (x, z) in ring_pts])
    for i in range(len(rings) - 1):
        r0, r1 = rings[i], rings[i + 1]
        for k in range(ring):
            k1 = (k + 1) % ring
            bm.faces.new((r0[k], r0[k1], r1[k1], r1[k]))
    if cap:
        for r, flip in ((rings[0], True), (rings[-1], False)):
            centre = sum((v.co for v in r), Vector()) / len(r)
            c = bm.verts.new(centre)
            for k in range(ring):
                k1 = (k + 1) % ring
                verts = (r[k1], r[k], c) if flip else (r[k], r[k1], c)
                bm.faces.new(verts)
    bm.normal_update()
    for (co, no) in cuts:
        geom = bm.verts[:] + bm.edges[:] + bm.faces[:]
        bmesh.ops.bisect_plane(bm, geom=geom, plane_co=co, plane_no=no)
    bm.normal_update()
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = new_object(name, mesh, parent)
    add_mats(obj, mat_keys)
    if mat_for is not None:
        for poly in obj.data.polygons:
            poly.material_index = mat_for(Vector(poly.center), Vector(poly.normal))
    # Outward normals regardless of ring winding.
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(obj.data)
    bm.free()
    smooth(obj, sub)
    return obj


def box(name, size, location, mat_key, parent=None, bevel=0.0, rotation=(0, 0, 0)):
    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    for v in bm.verts:
        v.co = Vector((v.co.x * size[0], v.co.y * size[1], v.co.z * size[2]))
    bm.to_mesh(mesh)
    bm.free()
    obj = new_object(name, mesh, parent)
    obj.location = location
    obj.rotation_euler = rotation
    add_mats(obj, [mat_key])
    if bevel > 0:
        mod = obj.modifiers.new("Bevel", "BEVEL")
        mod.width = bevel
        mod.segments = 2
        for poly in obj.data.polygons:
            poly.use_smooth = True
    return obj


def cylinder(name, radius, depth, location, mat_key, parent=None, rotation=(0, 0, 0), verts=24, radius2=None):
    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=verts,
                          radius1=radius, radius2=radius if radius2 is None else radius2, depth=depth)
    bm.to_mesh(mesh)
    bm.free()
    obj = new_object(name, mesh, parent)
    obj.location = location
    obj.rotation_euler = rotation
    add_mats(obj, [mat_key])
    for poly in obj.data.polygons:
        poly.use_smooth = abs(poly.normal.z) < 0.9
    return obj


def sphere(name, radius, location, mat_key, parent=None, scale=(1, 1, 1)):
    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bmesh.ops.create_uvsphere(bm, u_segments=20, v_segments=12, radius=radius)
    bm.to_mesh(mesh)
    bm.free()
    obj = new_object(name, mesh, parent)
    obj.location = location
    obj.scale = scale
    add_mats(obj, [mat_key])
    for poly in obj.data.polygons:
        poly.use_smooth = True
    return obj


def tube_path(name, points, radius, mat_key, parent=None, resolution=6):
    curve = bpy.data.curves.new(name, "CURVE")
    curve.dimensions = "3D"
    curve.bevel_depth = radius
    curve.bevel_resolution = 3
    curve.resolution_u = resolution
    curve.use_fill_caps = True
    spline = curve.splines.new("BEZIER")
    spline.bezier_points.add(len(points) - 1)
    for bp, p in zip(spline.bezier_points, points):
        bp.co = p
        bp.handle_left_type = "AUTO"
        bp.handle_right_type = "AUTO"
    obj = bpy.data.objects.new(name, curve)
    bpy.context.scene.collection.objects.link(obj)
    if parent is not None:
        obj.parent = parent
    curve.materials.append(MATS[mat_key])
    return obj


def slab(name, outline, thickness, mat_key, parent=None, axis="x", offset=0.0, bevel=0.012):
    """Extrude a 2D outline [(u, v)] into a thin plate. axis='x': plate in the YZ plane."""
    bm = bmesh.new()
    half = thickness / 2.0
    front, back = [], []
    for (u, v) in outline:
        if axis == "x":
            front.append(bm.verts.new((offset + half, u, v)))
            back.append(bm.verts.new((offset - half, u, v)))
        else:  # plate in the XY plane (horizontal)
            front.append(bm.verts.new((u, v, offset + half)))
            back.append(bm.verts.new((u, v, offset - half)))
    bm.faces.new(front)
    bm.faces.new(list(reversed(back)))
    n = len(outline)
    for k in range(n):
        k1 = (k + 1) % n
        bm.faces.new((front[k], back[k], back[k1], front[k1]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = new_object(name, mesh, parent)
    add_mats(obj, [mat_key])
    if bevel > 0:
        mod = obj.modifiers.new("Bevel", "BEVEL")
        mod.width = bevel
        mod.segments = 2
    return obj


def blade(name, length, chord, thickness, root, parent, mat_key="rotor", tip_key="tip", axis="x", tip_len=0.32):
    """A rotor blade along +axis from `root`, aerofoil-ish lozenge section."""
    group = []
    span_main = length - tip_len
    for part, (start, span, key) in enumerate(((0.0, span_main, mat_key), (span_main, tip_len, tip_key))):
        bm = bmesh.new()
        sec = [(-chord * 0.5, 0.0), (-chord * 0.2, thickness * 0.5), (chord * 0.5, 0.0), (-chord * 0.2, -thickness * 0.5)]
        rings = []
        for s in (start, start + span):
            ring = []
            for (c, t) in sec:
                if axis == "x":
                    ring.append(bm.verts.new((root[0] + s, root[1] + c, root[2] + t)))
                else:
                    ring.append(bm.verts.new((root[0] + c, root[1], root[2] + s)))
            rings.append(ring)
        bm.faces.new(rings[0][::-1])
        bm.faces.new(rings[1])
        for k in range(4):
            k1 = (k + 1) % 4
            bm.faces.new((rings[0][k], rings[0][k1], rings[1][k1], rings[1][k]))
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        mesh = bpy.data.meshes.new(f"{name}_{part}")
        bm.to_mesh(mesh)
        bm.free()
        obj = new_object(f"{name}_{part}", mesh, parent)
        add_mats(obj, [key])
        group.append(obj)
    return group


def text_mesh(name, body, size, location, rotation, mat_key, parent=None, extrude=0.003):
    curve = bpy.data.curves.new(name, "FONT")
    curve.body = body
    curve.size = size
    curve.extrude = extrude
    curve.align_x = "CENTER"
    curve.align_y = "CENTER"
    obj = bpy.data.objects.new(name, curve)
    bpy.context.scene.collection.objects.link(obj)
    obj.location = location
    obj.rotation_euler = rotation
    if parent is not None:
        obj.parent = parent
    curve.materials.append(MATS[mat_key])
    return obj


# ---------------------------------------------------------------- model
# Fuselage stations (y, half width, z bottom, z top, n_top, n_bottom).
FUSELAGE = [
    (-2.70, 0.03, 1.02, 1.08, 2.0, 2.0),
    (-2.58, 0.30, 0.74, 1.45, 2.2, 2.2),
    (-2.30, 0.56, 0.50, 1.86, 2.4, 2.5),
    (-1.85, 0.72, 0.38, 2.12, 2.6, 2.9),
    (-1.20, 0.80, 0.33, 2.27, 2.8, 3.3),
    (-0.40, 0.82, 0.32, 2.33, 3.0, 3.6),
    (0.50, 0.82, 0.33, 2.34, 3.0, 3.6),
    (1.25, 0.77, 0.40, 2.30, 2.9, 3.2),
    (1.85, 0.60, 0.62, 2.18, 2.6, 2.6),
    (2.45, 0.38, 0.98, 1.98, 2.2, 2.2),
    (3.10, 0.235, 1.25, 1.80, 2.0, 2.0),
    (5.00, 0.17, 1.36, 1.72, 2.0, 2.0),
    (7.05, 0.12, 1.43, 1.68, 2.0, 2.0),
    (7.45, 0.07, 1.47, 1.64, 2.0, 2.0),
]

COWLING = [
    (-0.75, 0.03, 2.22, 2.28, 2.0, 2.0),
    (-0.55, 0.40, 2.18, 2.52, 2.4, 2.0),
    (0.10, 0.52, 2.18, 2.74, 2.8, 2.0),
    (1.10, 0.50, 2.16, 2.72, 2.8, 2.0),
    (1.95, 0.36, 2.02, 2.55, 2.4, 2.0),
    (2.55, 0.16, 1.85, 2.18, 2.0, 2.0),
    (2.75, 0.04, 1.86, 1.94, 2.0, 2.0),
]


def fuselage_material(center, normal):
    # 0 navy, 1 white, 2 accent, 3 glass
    x, y, z = center.x, center.y, center.z
    if y < 3.0:
        # Wrap-around canopy: windscreen, roof eyebrow, chin bubbles.
        if y < -0.55 and z > 1.18:
            return 3
        # Side door windows (front and rear cabin doors), a pillar between.
        if abs(x) > 0.45 and 1.30 < z < 2.12 and (-0.45 < y < 0.30 or 0.42 < y < 1.15):
            return 3
        if z < 0.98:
            return 0
        if z < 1.12:
            return 2
        return 1
    return 0


def build():
    root = empty("PoliceHelicopter", (0, 0, 0))
    cuts = [((0, 0, z), (0, 0, 1)) for z in (0.98, 1.12, 1.18, 1.30, 2.12)]
    cuts += [((0, y, 0), (0, 1, 0)) for y in (-0.55, -0.45, 0.30, 0.42, 1.15, 3.0)]
    cuts += [((x, 0, 0), (1, 0, 0)) for x in (-0.45, 0.45)]
    body = loft("Fuselage", FUSELAGE, ring=48, per_span=4, mat_keys=("navy", "white", "accent", "glass"),
                mat_for=fuselage_material, parent=root, cuts=cuts)
    loft("Cowling", COWLING, ring=28, per_span=3, mat_keys=("white",), parent=root)
    # Engine intakes and exhaust.
    for side in (-1, 1):
        box(f"Intake_{'L' if side < 0 else 'R'}", (0.03, 0.34, 0.16), (side * 0.50, 0.55, 2.47), "metal", root, bevel=0.01)
    cylinder("Exhaust", 0.11, 0.42, (0.0, 2.35, 2.25), "metal", root, rotation=(math.radians(-72), 0, 0), verts=20, radius2=0.13)

    # Cabin interior: the dark tub seen through the glazing.
    box("Interior_Floor", (1.44, 2.8, 0.06), (0, -0.6, 0.48), "interior", root)
    for side in (-1, 1):
        box(f"Seat_Front_{side}", (0.46, 0.46, 0.12), (side * 0.38, -0.95, 0.80), "interior", root, bevel=0.03)
        box(f"Seat_Back_{side}", (0.46, 0.10, 0.70), (side * 0.38, -0.70, 1.15), "interior", root, bevel=0.03)
        box(f"Seat_Rear_{side}", (0.46, 0.46, 0.12), (side * 0.38, 0.55, 0.80), "interior", root, bevel=0.03)
    box("Instrument_Panel", (1.20, 0.30, 0.32), (0, -1.75, 1.22), "interior", root, bevel=0.04)
    for side in (-1, 1):
        box(f"Instrument_Screen_{side}", (0.26, 0.02, 0.16), (side * 0.30, -1.91, 1.30), "screen", root)
    box("Cabin_Bulkhead", (1.5, 0.05, 1.6), (0, 1.25, 1.3), "interior", root)

    # Door seams and handles.
    for side in (-1, 1):
        x = side * 0.815
        for y in (-0.52, 0.36, 1.22):
            box(f"Door_Seam_{side}_{y}", (0.006, 0.012, 1.25), (x, y, 1.38), "metal", root)
        box(f"Door_Handle_{side}", (0.02, 0.14, 0.03), (side * 0.83, 0.05, 1.22), "steel", root, bevel=0.006)
        box(f"Step_{side}", (0.14, 0.42, 0.03), (side * 0.93, -0.30, 0.42), "metal", root, bevel=0.008)

    # Tail: vertical fin (upper + ventral), stabiliser with end plates.
    slab("Tail_Fin", [(6.75, 1.62), (7.55, 1.60), (7.75, 2.62), (7.42, 2.66)], 0.07, "navy", root)
    slab("Tail_Ventral", [(6.95, 1.48), (7.45, 1.48), (7.55, 1.12), (7.35, 1.10)], 0.06, "navy", root)
    slab("Stabiliser", [(-1.05, 5.45), (1.05, 5.45), (1.05, 5.82), (-1.05, 5.88)], 0.055, "navy", root, axis="z", offset=1.52)
    for side in (-1, 1):
        slab(f"Endplate_{side}", [(5.42, 1.30), (5.86, 1.30), (5.92, 1.86), (5.56, 1.86)], 0.04, "accent", root, offset=side * 1.06)

    # Skids: two runners swept up at the toe, two arched cross tubes.
    for side in (-1, 1):
        x = side * 1.02
        tube_path(f"Skid_{side}", [(x, -2.05, 0.22), (x, -1.78, 0.05), (x, -0.6, 0.04), (x, 1.85, 0.04), (x, 2.05, 0.08)], 0.042, "metal", root)
    for y in (-0.95, 1.0):
        tube_path(f"Crosstube_{y}", [(-1.02, y, 0.05), (-0.86, y, 0.42), (-0.4, y, 0.55), (0.4, y, 0.55), (0.86, y, 0.42), (1.02, y, 0.05)], 0.045, "metal", root)

    # Rotor mast and hub.
    cylinder("Mast", 0.075, 0.42, (0, 0.20, 2.94), "steel", root, verts=20)
    rotor = empty("MainRotor", (0, 0.20, 3.18), root)
    cylinder("Rotor_Hub", 0.22, 0.12, (0, 0, 0), "metal", rotor, verts=24)
    sphere("Rotor_Cap", 0.12, (0, 0, 0.07), "metal", rotor, scale=(1, 1, 0.6))
    for k in range(4):
        arm = empty(f"Blade_Arm_{k}", (0, 0, 0), rotor)
        arm.rotation_euler = (0, 0, k * math.pi / 2)
        blade(f"Blade_{k}", 5.0, 0.30, 0.045, (0.18, 0.0, 0.0), arm)
        box(f"Blade_Grip_{k}", (0.32, 0.12, 0.08), (0.30, 0, 0), "metal", arm, bevel=0.01)

    tail_rotor = empty("TailRotor", (0.19, 7.30, 2.08), root)
    cylinder("TailRotor_Hub", 0.07, 0.10, (0, 0, 0), "metal", tail_rotor, rotation=(0, math.pi / 2, 0), verts=16)
    for k in range(2):
        arm = empty(f"TailBlade_Arm_{k}", (0, 0, 0), tail_rotor)
        arm.rotation_euler = (k * math.pi, 0, 0)
        blade(f"TailBlade_{k}", 0.82, 0.14, 0.025, (0.0, 0.0, 0.05), arm, axis="z", tip_len=0.12)
    cylinder("TailRotor_Gearbox", 0.09, 0.22, (0.08, 7.30, 2.08), "navy", root, rotation=(0, math.pi / 2, 0), verts=16)

    # Searchlight on a gimbal under the right of the nose; FLIR ball on the left.
    cylinder("Searchlight_Mount", 0.04, 0.22, (0.52, -1.95, 0.30), "metal", root, verts=12)
    light = empty("Searchlight", (0.52, -1.95, 0.16), root)
    cylinder("Searchlight_Body", 0.15, 0.36, (0, 0, 0), "metal", light, rotation=(math.pi / 2, 0, 0), verts=24, radius2=0.17)
    cylinder("Searchlight_Lens", 0.145, 0.02, (0, -0.19, 0), "lens", light, rotation=(math.pi / 2, 0, 0), verts=24)
    cylinder("Searchlight_Yoke", 0.03, 0.40, (0, 0, 0), "metal", light, rotation=(0, math.pi / 2, 0), verts=10)
    sphere("Flir_Turret", 0.17, (-0.50, -2.00, 0.18), "metal", root)
    sphere("Flir_Window", 0.08, (-0.50, -2.13, 0.16), "glass", root, scale=(1, 0.4, 1))

    # Lights.
    sphere("Light_Beacon_Top", 0.06, (0, 2.20, 2.40), "red", root, scale=(1, 1, 0.7))
    sphere("Light_Beacon_Belly", 0.06, (0, 0.80, 0.30), "red", root, scale=(1, 1, 0.7))
    sphere("Light_Nav_L", 0.04, (-1.08, 5.66, 1.52), "red", root)
    sphere("Light_Nav_R", 0.04, (1.08, 5.66, 1.52), "green", root)
    sphere("Light_Strobe_Tail", 0.04, (0, 7.78, 2.62), "strobe", root)

    # Antennae.
    box("Antenna_Belly", (0.02, 0.20, 0.22), (0, 0.2, 0.26), "metal", root)
    box("Antenna_Boom", (0.02, 0.16, 0.24), (0, 4.4, 1.86), "metal", root)

    # Livery lettering: white on the navy lower cabin, both sides, and a tail number.
    for side in (-1, 1):
        rot = (math.pi / 2, 0, math.pi / 2 if side > 0 else -math.pi / 2)
        text_mesh(f"Lettering_POLICE_{side}", "POLICE", 0.34, (side * 0.835, 0.30, 0.74), rot, "decal", root)
        text_mesh(f"Lettering_Tail_{side}", "12", 0.24, (side * 0.045, 7.42, 2.05), rot, "decal", root)
    return root, body


# ---------------------------------------------------------------- export + review
def finalize(root):
    bpy.context.view_layer.update()
    # Convert curves/text to meshes so the GLB carries them.
    for obj in list(bpy.context.scene.objects):
        if obj.type in {"CURVE", "FONT"}:
            bpy.ops.object.select_all(action="DESELECT")
            obj.select_set(True)
            bpy.context.view_layer.objects.active = obj
            bpy.ops.object.convert(target="MESH")
    stats = {"objects": 0, "triangles": 0}
    depsgraph = bpy.context.evaluated_depsgraph_get()
    for obj in bpy.context.scene.objects:
        if obj.type != "MESH":
            continue
        stats["objects"] += 1
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        mesh.calc_loop_triangles()
        stats["triangles"] += len(mesh.loop_triangles)
        evaluated.to_mesh_clear()
    return stats


def measure():
    lo = Vector((1e9, 1e9, 1e9))
    hi = Vector((-1e9, -1e9, -1e9))
    depsgraph = bpy.context.evaluated_depsgraph_get()
    for obj in bpy.context.scene.objects:
        if obj.type != "MESH" or obj.name.startswith("Blade_") and False:
            continue
        evaluated = obj.evaluated_get(depsgraph)
        for corner in evaluated.bound_box:
            p = obj.matrix_world @ Vector(corner)
            lo = Vector((min(lo.x, p.x), min(lo.y, p.y), min(lo.z, p.z)))
            hi = Vector((max(hi.x, p.x), max(hi.y, p.y), max(hi.z, p.z)))
    return {"min": list(lo), "max": list(hi), "size": list(hi - lo)}


def render_reviews(directory):
    scene = bpy.context.scene
    try:
        scene.render.engine = "BLENDER_EEVEE"
    except TypeError:
        scene.render.engine = "CYCLES"
    scene.render.resolution_x = 1280
    scene.render.resolution_y = 800
    scene.render.film_transparent = False
    world = bpy.data.worlds.new("Review") if not scene.world else scene.world
    scene.world = world
    world.use_nodes = True
    bg = next(n for n in world.node_tree.nodes if n.type == "BACKGROUND")
    bg.inputs["Color"].default_value = (0.42, 0.48, 0.56, 1.0)
    bg.inputs["Strength"].default_value = 0.9
    sun_data = bpy.data.lights.new("ReviewSun", "SUN")
    sun_data.energy = 3.5
    sun = bpy.data.objects.new("ReviewSun", sun_data)
    sun.rotation_euler = (math.radians(50), math.radians(10), math.radians(35))
    scene.collection.objects.link(sun)
    cam_data = bpy.data.cameras.new("ReviewCam")
    cam_data.lens = 50
    cam = bpy.data.objects.new("ReviewCam", cam_data)
    scene.collection.objects.link(cam)
    scene.camera = cam
    target = Vector((0, 2.0, 1.4))
    shots = {
        "front34": Vector((-9.0, -11.0, 4.0)),
        "side": Vector((-17.0, 2.0, 2.0)),
        "rear34": Vector((9.0, 15.0, 5.0)),
        "below": Vector((-6.0, -9.0, -6.0)),
        "top": Vector((0.01, 2.0, 22.0)),
    }
    written = []
    for label, eye in shots.items():
        cam.location = eye
        cam.rotation_euler = (target - eye).to_track_quat("-Z", "Y").to_euler()
        path = directory / "renders" / f"{NAME}-{label}.png"
        path.parent.mkdir(parents=True, exist_ok=True)
        scene.render.filepath = str(path)
        bpy.ops.render.render(write_still=True)
        written.append(str(path.name))
    for obj in (sun, cam):
        bpy.data.objects.remove(obj, do_unlink=True)
    return written


def export_glb(root, path):
    bpy.ops.object.select_all(action="DESELECT")
    for obj in bpy.context.scene.objects:
        if obj.name.startswith("Review"):
            continue
        obj.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=str(path),
        export_format="GLB",
        use_selection=True,
        export_apply=True,
        export_extras=True,
        export_cameras=False,
        export_lights=False,
        export_materials="EXPORT",
        export_yup=True,
        export_animations=False,
    )


def main():
    directory = out_dir()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    build_materials()
    root, _ = build()
    stats = finalize(root)
    dims = measure()
    bpy.ops.wm.save_as_mainfile(filepath=str(directory / f"{NAME}.blend"))
    glb = directory / f"{NAME}.glb"
    export_glb(root, glb)
    renders = []
    if "--no-renders" not in sys.argv:
        renders = render_reviews(directory)
    report = {
        "asset": NAME,
        "method": "hand-authored Blender script (lofted superellipse fuselage + modelled parts); no AI 3D generation",
        "blender": bpy.app.version_string,
        "frame": "+Z up, nose -Y (GLB: +Y up, nose +Z)",
        "dimensions_m": dims,
        "main_rotor_diameter_m": 2 * (0.18 + 5.0),
        "mesh_objects": stats["objects"],
        "triangles": stats["triangles"],
        "glb": glb.name,
        "glb_bytes": glb.stat().st_size,
        "renders": renders,
        "named_nodes": ["MainRotor", "TailRotor", "Searchlight", "Light_Beacon_Top", "Light_Beacon_Belly",
                        "Light_Nav_L", "Light_Nav_R", "Light_Strobe_Tail"],
    }
    (directory / "build-report.json").write_text(json.dumps(report, indent=2) + "\n")
    print("BUILD-REPORT", json.dumps(report))


main()
