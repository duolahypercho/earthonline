"""Vehicle v3 part helpers: region cutting, panel separation with rolled edges,
recessed lamp/grille tubs and lathe-built wheels.

All functions work in the Blender frame (X right, -Y nose, Z up).
"""
import math

import bmesh
import bpy
from mathutils import Matrix, Vector


# ---------------------------------------------------------------- materials
def mat(name, color, metallic=0.0, rough=0.5, coat=0.0, emit=None, emit_str=0.0, alpha=1.0, transmission=0.0):
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    m.use_nodes = True
    if name == 'VehicleHeadlamp':
        m['installedLampChannel'] = 'head'
        m.use_backface_culling = True
    elif name == 'VehicleTaillamp':
        m['installedLampChannel'] = 'tail'
        m.use_backface_culling = True
    b = next(n for n in m.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
    b.inputs['Base Color'].default_value = (*color, 1)
    b.inputs['Metallic'].default_value = metallic
    b.inputs['Roughness'].default_value = rough
    for k, v in (('Coat Weight', coat), ('Emission Strength', emit_str), ('Alpha', alpha),
                 ('Transmission Weight', transmission)):
        if k in b.inputs:
            b.inputs[k].default_value = v
    if 'IOR' in b.inputs:
        b.inputs['IOR'].default_value = 1.45 if transmission > 0 else 1.5
    if emit is not None and 'Emission Color' in b.inputs:
        b.inputs['Emission Color'].default_value = (*emit, 1)
    if alpha < 1:
        try:
            m.surface_render_method = 'BLENDED'
        except Exception:
            pass
    return m


def materials(paint=None):
    """The runtime material contract (names are read by vehicle-hero-overlay.js)."""
    M = {}
    M['paint'] = paint or mat('VehiclePaint', (1.0, 1.0, 1.0), metallic=0.2, rough=0.28, coat=1.0)
    M['glass'] = mat('VehicleGlass', (0.11, 0.17, 0.20), rough=0.02, alpha=0.30, transmission=0.78)
    # The service cover is a physical lens, not a transparent overlay. Keeping
    # it opaque makes the open pose read as a real hinged pod instead of a
    # floating sheet while the canonical lamp channel remains on the material.
    M['headcover'] = mat('VehicleHeadlamp', (0.30, 0.35, 0.40), rough=0.06, alpha=1.0)
    M['lens'] = mat('VehicleProjector', (0.80, 0.84, 0.90), rough=0.08, metallic=0.4, emit=(1.0, 0.95, 0.86), emit_str=0.16)
    M['drl'] = mat('VehicleDRL', (0.62, 0.63, 0.64), rough=0.22, emit=(1.0, 0.90, 0.74), emit_str=0.16)
    M['reflector'] = mat('VehicleLampReflector', (0.55, 0.56, 0.58), metallic=1.0, rough=0.18)
    M['tail'] = mat('VehicleTaillamp', (0.42, 0.015, 0.012), rough=0.06, emit=(1.0, 0.05, 0.02), emit_str=0.2)
    M['tailSmoke'] = mat('VehicleTaillampLens', (0.22, 0.012, 0.012), rough=0.10, alpha=1.0)
    M['amber'] = mat('VehicleIndicator', (0.85, 0.42, 0.05), rough=0.12, emit=(1.0, 0.5, 0.05), emit_str=0.2)
    M['reverse'] = mat('VehicleReverse', (0.80, 0.80, 0.78), rough=0.12)
    M['trim'] = mat('VehicleTrimBlack', (0.012, 0.012, 0.013), rough=0.55)
    M['gloss'] = mat('VehicleTrimGloss', (0.010, 0.010, 0.010), rough=0.12, coat=0.6)
    M['chrome'] = mat('VehicleChrome', (0.90, 0.90, 0.90), metallic=1.0, rough=0.10)
    M['rubber'] = mat('VehicleTyre', (0.022, 0.022, 0.022), rough=0.85)
    M['alloy'] = mat('VehicleAlloy', (0.72, 0.73, 0.75), metallic=1.0, rough=0.22)
    M['darkalloy'] = mat('VehicleAlloyDark', (0.06, 0.06, 0.065), metallic=0.6, rough=0.6)
    M['steel'] = mat('VehicleSteel', (0.38, 0.38, 0.40), metallic=1.0, rough=0.5)
    M['caliper'] = mat('VehicleCaliper', (0.10, 0.10, 0.11), metallic=0.3, rough=0.5)
    M['shut'] = mat('VehicleShutLine', (0.03, 0.03, 0.032), rough=0.7)
    M['plate'] = mat('VehiclePlate', (0.92, 0.92, 0.90), rough=0.4)
    M['interior'] = mat('VehicleInterior', (0.12, 0.105, 0.09), rough=0.74)
    M['doorcard'] = mat('VehicleDoorCard', (0.22, 0.19, 0.16), rough=0.76)
    return M


def link(ob, coll=None):
    (coll or bpy.context.scene.collection).objects.link(ob)
    return ob


def obj_from_bm(name, bm, material, smooth=True, coll=None):
    me = bpy.data.meshes.new(name)
    if smooth:
        for f in bm.faces:
            f.smooth = True
    bm.to_mesh(me)
    bm.free()
    ob = bpy.data.objects.new(name, me)
    if material is not None:
        me.materials.append(material)
    return link(ob, coll)


def empty(name, loc, coll=None, size=0.08):
    e = bpy.data.objects.new(name, None)
    e.empty_display_size = size
    e.location = loc
    return link(e, coll)


def parent_keep(ob, parent):
    bpy.context.view_layer.update()
    w = ob.matrix_world.copy()
    ob.parent = parent
    ob.matrix_parent_inverse = Matrix.Identity(4)
    ob.matrix_world = w


def centroid(face):
    return face.calc_center_median()


# ---------------------------------------------------------------- 2D geometry
def point_in_poly(p, poly):
    x, y = p
    inside = False
    n = len(poly)
    for i in range(n):
        x1, y1 = poly[i]
        x2, y2 = poly[(i + 1) % n]
        if (y1 > y) != (y2 > y):
            xin = x1 + (y - y1) * (x2 - x1) / (y2 - y1)
            if xin > x:
                inside = not inside
    return inside


AXES = {'xz': (0, 2, 1), 'yz': (1, 2, 0), 'xy': (0, 1, 2)}


def proj(v, plane):
    a, b, _ = AXES[plane]
    return (v[a], v[b])


def bisect_outline(bm, outline, plane, face_filter, closed=True):
    """Cut exact edges along a 2D outline (in `plane`) on the faces passing face_filter.
    Each outline segment is a plane containing the projection axis."""
    a, b, c = AXES[plane]
    n = len(outline)
    segs = range(n) if closed else range(n - 1)
    for i in segs:
        p1 = outline[i]
        p2 = outline[(i + 1) % n]
        d = Vector((p2[0] - p1[0], p2[1] - p1[1]))
        if d.length < 1e-6:
            continue
        nrm2 = Vector((-d.y, d.x)).normalized()
        co = [0.0, 0.0, 0.0]
        no = [0.0, 0.0, 0.0]
        co[a], co[b] = p1[0], p1[1]
        no[a], no[b] = nrm2.x, nrm2.y
        lo = (min(p1[0], p2[0]) - 0.02, min(p1[1], p2[1]) - 0.02)
        hi = (max(p1[0], p2[0]) + 0.02, max(p1[1], p2[1]) + 0.02)
        faces = []
        for f in bm.faces:
            if not face_filter(f):
                continue
            ps = [proj(v.co, plane) for v in f.verts]
            if max(q[0] for q in ps) < lo[0] or min(q[0] for q in ps) > hi[0]:
                continue
            if max(q[1] for q in ps) < lo[1] or min(q[1] for q in ps) > hi[1]:
                continue
            faces.append(f)
        if not faces:
            continue
        edges = list({e for f in faces for e in f.edges})
        verts = list({v for f in faces for v in f.verts})
        bmesh.ops.bisect_plane(bm, geom=verts + edges + faces, dist=1e-6,
                               plane_co=Vector(co), plane_no=Vector(no))


def faces_in_outline(bm, outline, plane, face_filter):
    out = []
    for f in bm.faces:
        if face_filter(f) and point_in_poly(proj(centroid(f), plane), outline):
            out.append(f)
    return out


# ---------------------------------------------------------------- panels
def rolled_edges(ob, gap=0.0028, flange=0.014, flange_mat_index=None):
    """Shrink each open boundary of a panel along its surface by `gap` and turn a
    flange inward, so a cut reads as a real shut line with a rounded lip."""
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bm.normal_update()
    bverts = [v for v in bm.verts if v.is_boundary]
    moves = {}
    for v in bverts:
        acc = Vector()
        for f in v.link_faces:
            t = centroid(f) - v.co
            t -= v.normal * t.dot(v.normal)
            acc += t
        if acc.length > 1e-9:
            # never move further than 40% of the shortest interior edge: a sliver
            # row next to a cut would otherwise fold inside out
            inner = [e.calc_length() for e in v.link_edges if not e.is_boundary]
            lim = 0.4 * min(inner) if inner else gap
            moves[v] = acc.normalized() * min(gap, lim)
    for v, d in moves.items():
        v.co += d
    bedges = [e for e in bm.edges if e.is_boundary]
    # outward direction of every boundary edge: from its panel face across the edge
    out_dir = {}
    for e in bedges:
        mid = (e.verts[0].co + e.verts[1].co) / 2
        out_dir[e.index] = (mid - centroid(e.link_faces[0])) if e.link_faces else Vector((0, 0, 1))
    src_dir = {}
    for e in bedges:
        for v in e.verts:
            src_dir.setdefault(v, Vector())
            src_dir[v] += out_dir[e.index]
    ret = bmesh.ops.extrude_edge_only(bm, edges=bedges)
    newv = [g for g in ret['geom'] if isinstance(g, bmesh.types.BMVert)]
    for v in newv:
        src = [e.other_vert(v) for e in v.link_edges if e.other_vert(v) not in newv]
        n = src[0].normal if src else Vector((0, 0, 1))
        v.co -= n * flange
    newf = [g for g in ret['geom'] if isinstance(g, bmesh.types.BMFace)]
    for f in newf:
        f.smooth = True
        if flange_mat_index is not None:
            f.material_index = flange_mat_index
        # the flange faces the gap: its normal follows the outward edge direction.
        # Never recalc_face_normals on an open panel - it flips thin strips.
        d = Vector()
        for v in f.verts:
            if v in src_dir:
                d += src_dir[v]
        f.normal_update()
        if d.length > 0 and f.normal.dot(d) < 0:
            f.normal_flip()
    bm.to_mesh(ob.data)
    bm.free()


def offset_along_normals(ob, dist):
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bm.normal_update()
    for v in bm.verts:
        v.co += v.normal * dist
    bm.to_mesh(ob.data)
    bm.free()


def tub(src, name, depth, material, floor_scale=0.92, coll=None):
    """A recess behind a cut region: walls from the region's boundary straight in
    along the average inward normal, and a floor. Returns the housing object."""
    bm = bmesh.new()
    bm.from_mesh(src.data)
    bm.normal_update()
    n = Vector()
    for f in bm.faces:
        n += f.normal * f.calc_area()
    n = n.normalized() if n.length > 0 else Vector((0, -1, 0))
    c = Vector()
    for v in bm.verts:
        c += v.co
    c /= max(1, len(bm.verts))
    bedges = [e for e in bm.edges if e.is_boundary]
    ret = bmesh.ops.extrude_edge_only(bm, edges=bedges)
    newv = [g for g in ret['geom'] if isinstance(g, bmesh.types.BMVert)]
    for v in newv:
        v.co = c + (v.co - c) * floor_scale - n * depth
    # floor: fill the new boundary
    loops = [e for e in bm.edges if e.is_boundary and all(v in newv for v in e.verts)]
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if not any(v in newv for v in f.verts)], context='FACES_ONLY')
    try:
        bmesh.ops.holes_fill(bm, edges=loops, sides=0)
    except Exception:
        pass
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    ob = obj_from_bm(name, bm, material, smooth=False, coll=coll)
    return ob


# ---------------------------------------------------------------- lathe / wheels
def lathe(name, profile, segs, material, axis='X', coll=None, smooth=True, closed=False):
    """Spin a (radius, axial) profile about the X axis."""
    bm = bmesh.new()
    rings = []
    for i in range(segs):
        a = 2 * math.pi * i / segs
        ca, sa = math.cos(a), math.sin(a)
        rings.append([bm.verts.new((ax, r * ca, r * sa)) for r, ax in profile])
    for i in range(segs):
        r0, r1 = rings[i], rings[(i + 1) % segs]
        for j in range(len(profile) - 1):
            try:
                bm.faces.new((r0[j], r0[j + 1], r1[j + 1], r1[j]))
            except ValueError:
                pass
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-6)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return obj_from_bm(name, bm, material, smooth=smooth, coll=coll)


def box(name, size, loc, material, rot=(0, 0, 0), bevel=0.0, segs=2, coll=None):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=size, verts=bm.verts)
    if bevel > 0:
        bmesh.ops.bevel(bm, geom=list(bm.edges), offset=bevel, segments=segs, affect='EDGES', profile=0.5)
    ob = obj_from_bm(name, bm, material, smooth=bevel > 0, coll=coll)
    ob.location = loc
    ob.rotation_euler = rot
    return ob


def cyl(name, r, depth, loc, material, axis='X', segs=24, r2=None, coll=None, smooth=True):
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=segs, radius1=r,
                          radius2=r if r2 is None else r2, depth=depth)
    if axis == 'X':
        bmesh.ops.rotate(bm, verts=bm.verts, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, 'Y'))
    elif axis == 'Y':
        bmesh.ops.rotate(bm, verts=bm.verts, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, 'X'))
    ob = obj_from_bm(name, bm, material, smooth=smooth, coll=coll)
    ob.location = loc
    return ob


def join(objs, name):
    objs = [o for o in objs if o is not None]
    if not objs:
        return None
    bpy.ops.object.select_all(action='DESELECT')
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.join()
    ob = bpy.context.active_object
    ob.name = name
    return ob


def apply_mods(ob):
    bpy.ops.object.select_all(action='DESELECT')
    bpy.context.view_layer.objects.active = ob
    ob.select_set(True)
    for m in list(ob.modifiers):
        try:
            bpy.ops.object.modifier_apply(modifier=m.name)
        except RuntimeError:
            ob.modifiers.remove(m)


# ---------------------------------------------------------------- projected patches
def clip_poly(poly, axis, lo, hi):
    """Sutherland-Hodgman clip of a 2D polygon to lo <= p[axis] <= hi."""
    def clip(pts, keep, inter):
        out = []
        for i in range(len(pts)):
            a, b = pts[i - 1], pts[i]
            ka, kb = keep(a), keep(b)
            if kb:
                if not ka:
                    out.append(inter(a, b))
                out.append(b)
            elif ka:
                out.append(inter(a, b))
        return out

    def at(a, b, v):
        t = (v - a[axis]) / ((b[axis] - a[axis]) or 1e-12)
        return (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t)
    pts = clip(list(poly), lambda p: p[axis] >= lo, lambda a, b: at(a, b, lo))
    if pts:
        pts = clip(pts, lambda p: p[axis] <= hi, lambda a, b: at(a, b, hi))
    return pts


def projected_patch(name, bvh, poly, material, offset, direction, step=0.01, coll=None):
    """A clean grid patch following the body surface inside a front/rear-view (x, z)
    outline. Rays travel along `direction` (+1: toward +Y, i.e. onto the nose from
    the front). The patch sits `offset` metres along the surface normal."""
    if len(poly) < 3:
        return None
    xs = [p[0] for p in poly]
    zs = [p[1] for p in poly]
    x0, x1, z0, z1 = min(xs), max(xs), min(zs), max(zs)
    nx = max(2, int((x1 - x0) / step) + 2)
    nz = max(2, int((z1 - z0) / step) + 2)
    bm = bmesh.new()
    grid = {}
    start = -20.0 * direction
    for i in range(nx):
        x = x0 + (x1 - x0) * i / (nx - 1)
        for j in range(nz):
            z = z0 + (z1 - z0) * j / (nz - 1)
            hit, nrm, _, _ = bvh.ray_cast(Vector((x, start, z)), Vector((0, direction, 0)))
            if hit is None:
                continue
            if nrm.y * direction > 0:
                nrm = -nrm
            grid[(i, j)] = bm.verts.new(hit + nrm * offset)
    for i in range(nx - 1):
        for j in range(nz - 1):
            q = [grid.get(k) for k in ((i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1))]
            if all(q):
                cx = sum(v.co.x for v in q) / 4
                cz = sum(v.co.z for v in q) / 4
                if point_in_poly((cx, cz), poly):
                    try:
                        bm.faces.new(q)
                    except ValueError:
                        pass
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
    if not bm.faces:
        bm.free()
        return None
    # exact outline: cut the patch (only the patch) along each outline edge and drop outside faces
    for k in range(len(poly)):
        a, b = poly[k - 1], poly[k]
        d = Vector((b[0] - a[0], b[1] - a[1]))
        if d.length < 1e-6:
            continue
        n2 = Vector((-d.y, d.x)).normalized()
        bmesh.ops.bisect_plane(bm, geom=list(bm.verts) + list(bm.edges) + list(bm.faces), dist=1e-6,
                               plane_co=Vector((a[0], 0, a[1])), plane_no=Vector((n2.x, 0, n2.y)))
    kill = [f for f in bm.faces if not point_in_poly((f.calc_center_median().x, f.calc_center_median().z), poly)]
    bmesh.ops.delete(bm, geom=kill, context='FACES')
    for f in bm.faces:
        f.normal_update()
        if f.normal.y * direction > 0:
            f.normal_flip()
    return obj_from_bm(name, bm, material, smooth=True, coll=coll)


def side_patch(name, bvh, poly, material, offset, side, step=0.02, coll=None):
    """Like projected_patch, but on a flank: `poly` is a side-view (y, z) outline and
    rays travel along -side*X onto the body."""
    if len(poly) < 3:
        return None
    ys = [p[0] for p in poly]
    zs = [p[1] for p in poly]
    y0, y1, z0, z1 = min(ys), max(ys), min(zs), max(zs)
    ny = max(2, int((y1 - y0) / step) + 2)
    nz = max(2, int((z1 - z0) / step) + 2)
    bm = bmesh.new()
    grid = {}
    for i in range(ny):
        y = y0 + (y1 - y0) * i / (ny - 1)
        for j in range(nz):
            z = z0 + (z1 - z0) * j / (nz - 1)
            hit, nrm, _, _ = bvh.ray_cast(Vector((side * 5.0, y, z)), Vector((-side, 0, 0)))
            if hit is None:
                continue
            if nrm.x * side < 0:
                nrm = -nrm
            grid[(i, j)] = bm.verts.new(hit + nrm * offset)
    for i in range(ny - 1):
        for j in range(nz - 1):
            q = [grid.get(k) for k in ((i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1))]
            if all(q):
                cy = sum(v.co.y for v in q) / 4
                cz = sum(v.co.z for v in q) / 4
                if point_in_poly((cy, cz), poly):
                    try:
                        f = bm.faces.new(q)
                    except ValueError:
                        continue
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
    if not bm.faces:
        bm.free()
        return None
    for f in bm.faces:
        f.normal_update()
        if f.normal.x * side < 0:
            f.normal_flip()
    return obj_from_bm(name, bm, material, smooth=True, coll=coll)
