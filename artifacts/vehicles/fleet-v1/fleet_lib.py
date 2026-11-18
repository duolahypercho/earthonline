# fleet_lib.py - loft machinery and shared materials for the runtime vehicle
# fleet (artifacts/vehicles/fleet-v1). Original procedural shapes; no third
# party vehicle is referenced. Blender 5.x, headless.
#
# Frame (Blender): X right, -Y nose, Z up, ground at z = 0. Exported Y-up so
# the glTF nose is +Z, which is the frame src/vehicles/vehicle-geometry.js uses.
import bpy, bmesh, math, os, sys
from mathutils import Vector, Matrix
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', '..', 'scripts', 'assets'))

ROOT_NAME = 'Vehicle'

# ---------------- materials (names are the runtime contract) ----------------
def mat(name, color, metallic=0.0, rough=0.5, coat=0.0, emit=None, emit_str=0.0, alpha=1.0):
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes['Principled BSDF']
    b.inputs['Base Color'].default_value = (*color, 1)
    b.inputs['Metallic'].default_value = metallic
    b.inputs['Roughness'].default_value = rough
    for k, v in (('Coat Weight', coat), ('Clearcoat', coat), ('Emission Strength', emit_str), ('Alpha', alpha)):
        if k in b.inputs: b.inputs[k].default_value = v
    if emit is not None:
        for k in ('Emission Color', 'Emission'):
            if k in b.inputs: b.inputs[k].default_value = (*emit, 1)
    if alpha < 1:
        try: m.blend_method = 'BLEND'
        except Exception: pass
        try: m.surface_render_method = 'BLENDED'
        except Exception: pass
    return m

def materials():
    M = {}
    M['paint'] = mat('VehiclePaint', (0.55, 0.56, 0.58), metallic=0.85, rough=0.30, coat=1.0)
    M['glass'] = mat('VehicleGlass', (0.10, 0.12, 0.14), rough=0.02, alpha=0.30)
    M['headcover'] = mat('VehicleHeadlamp', (0.05, 0.06, 0.08), rough=0.02, alpha=0.30)
    M['lens'] = mat('VehicleProjector', (0.80, 0.84, 0.90), rough=0.10, metallic=0.4, emit=(1.0, 0.97, 0.90), emit_str=1.0)
    M['drl'] = mat('VehicleDRL', (0.85, 0.88, 0.92), rough=0.10, emit=(1.0, 1.0, 1.0), emit_str=1.0)
    M['tail'] = mat('VehicleTaillamp', (0.40, 0.012, 0.010), rough=0.08, emit=(1.0, 0.05, 0.02), emit_str=0.2)
    M['amber'] = mat('VehicleIndicator', (0.85, 0.42, 0.05), rough=0.12, emit=(1.0, 0.5, 0.05), emit_str=0.2)
    M['reverse'] = mat('VehicleReverse', (0.80, 0.80, 0.78), rough=0.12)
    M['trim'] = mat('VehicleTrimBlack', (0.012, 0.012, 0.013), rough=0.55)
    M['gloss'] = mat('VehicleTrimGloss', (0.010, 0.010, 0.010), rough=0.15, coat=0.6)
    M['chrome'] = mat('VehicleChrome', (0.90, 0.90, 0.90), metallic=1.0, rough=0.12)
    M['rubber'] = mat('VehicleTyre', (0.020, 0.020, 0.020), rough=0.82)
    M['alloy'] = mat('VehicleAlloy', (0.75, 0.76, 0.78), metallic=1.0, rough=0.20)
    M['darkalloy'] = mat('VehicleAlloyDark', (0.06, 0.06, 0.065), metallic=0.6, rough=0.7)
    M['steel'] = mat('VehicleSteel', (0.40, 0.40, 0.42), metallic=1.0, rough=0.55)
    M['caliper'] = mat('VehicleCaliper', (0.55, 0.05, 0.04), rough=0.5)
    M['interior'] = mat('VehicleInterior', (0.050, 0.045, 0.045), rough=0.9)
    M['seat'] = mat('VehicleSeat', (0.045, 0.040, 0.038), rough=0.85)
    M['dash'] = mat('VehicleDash', (0.035, 0.033, 0.032), rough=0.75)
    M['headliner'] = mat('VehicleHeadliner', (0.14, 0.135, 0.125), rough=0.95)
    M['carpet'] = mat('VehicleCarpet', (0.03, 0.028, 0.026), rough=1.0)
    M['shut'] = mat('VehicleShutLine', (0.045, 0.045, 0.048), rough=0.65)
    M['plate'] = mat('VehiclePlate', (0.92, 0.92, 0.90), rough=0.4)
    M['livery'] = mat('VehicleLivery', (0.94, 0.72, 0.12), rough=0.45)
    M['liveryDark'] = mat('VehicleLiveryDark', (0.02, 0.02, 0.022), rough=0.5)
    M['sign'] = mat('VehicleSign', (0.95, 0.93, 0.80), rough=0.3, emit=(1.0, 0.95, 0.75), emit_str=0.3)
    return M

# ---------------- scene helpers ----------------
def root():
    c = bpy.data.collections.get(ROOT_NAME)
    if c is None:
        c = bpy.data.collections.new(ROOT_NAME); bpy.context.scene.collection.children.link(c)
    return c

KEEP = {'Cam', 'Sun', 'Fill', 'Ground'}
def clear():
    for ob in list(bpy.data.objects):
        if ob.name not in KEEP: bpy.data.objects.remove(ob, do_unlink=True)
    for me in list(bpy.data.meshes):
        if me.users == 0: bpy.data.meshes.remove(me)

def link(ob):
    root().objects.link(ob); return ob

def smooth(ob, angle=35):
    for p in ob.data.polygons: p.use_smooth = True
    bpy.context.view_layer.objects.active = ob
    ob.select_set(True)
    try: bpy.ops.object.shade_auto_smooth(angle=math.radians(angle))
    except Exception:
        try: bpy.ops.object.shade_smooth_by_angle(angle=math.radians(angle))
        except Exception: pass
    ob.select_set(False)

def new_obj(name, bm, material, do_smooth=True, angle=35):
    me = bpy.data.meshes.new(name); bm.to_mesh(me); bm.free()
    ob = bpy.data.objects.new(name, me); me.materials.append(material); link(ob)
    if do_smooth: smooth(ob, angle)
    return ob

def prim_cyl(name, radius, depth, loc, axis='X', segs=24, mat=None, do_smooth=True, r2=None):
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=segs, radius1=radius, radius2=radius if r2 is None else r2, depth=depth)
    if axis == 'X': bmesh.ops.rotate(bm, verts=bm.verts, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, 'Y'))
    elif axis == 'Y': bmesh.ops.rotate(bm, verts=bm.verts, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, 'X'))
    ob = new_obj(name, bm, mat or bpy.data.materials['VehicleTrimBlack'], do_smooth)
    ob.location = loc
    return ob

def prim_box(name, size, loc, mat=None, bevel=0.0, bevel_segs=2, rot=(0, 0, 0)):
    bm = bmesh.new(); bmesh.ops.create_cube(bm, size=1.0); bmesh.ops.scale(bm, vec=size, verts=bm.verts)
    ob = new_obj(name, bm, mat or bpy.data.materials['VehicleTrimBlack'], do_smooth=bevel > 0)
    ob.location = loc; ob.rotation_euler = rot
    if bevel > 0:
        b = ob.modifiers.new('Bevel', 'BEVEL'); b.width = bevel; b.segments = bevel_segs; b.limit_method = 'NONE'
    return ob

def boolean_apply(target, cutter, op='DIFFERENCE', keep=False, solvers=('EXACT', 'EXACT_SELF')):
    """EXACT solver (keeps quads). Its inside/outside classification can flip with vertex order on these
    lofts and return an empty mesh; on an empty result from a non-empty target the target is restored and
    the op retried with self-intersection handling. MANIFOLD is avoided (triangulates into shading fans)."""
    backup = target.data.copy()
    for sv in solvers:
        mod = target.modifiers.new('Bool', 'BOOLEAN'); mod.operation = op; mod.object = cutter
        try:
            mod.solver = 'EXACT'
            mod.use_self = sv == 'EXACT_SELF'
        except TypeError:
            target.modifiers.remove(mod); continue
        bpy.context.view_layer.objects.active = target
        try: bpy.ops.object.modifier_apply(modifier=mod.name)
        except RuntimeError:
            if mod.name in target.modifiers: target.modifiers.remove(mod)
            continue
        if len(target.data.polygons) > 0 or len(backup.polygons) == 0: break
        old = target.data; target.data = backup.copy(); bpy.data.meshes.remove(old)
    bpy.data.meshes.remove(backup)
    if not keep: bpy.data.objects.remove(cutter, do_unlink=True)

# ---------------- loft machinery ----------------
def gauss_smooth(vals, sigma_samples):
    if sigma_samples <= 0: return list(vals)
    r = int(3 * sigma_samples)
    k = [math.exp(-0.5 * (i / sigma_samples) ** 2) for i in range(-r, r + 1)]; ks = sum(k)
    n = len(vals); res = []
    for i in range(n):
        acc = 0.0
        for j, w in zip(range(-r, r + 1), k):
            acc += vals[min(n - 1, max(0, i + j))] * w
        res.append(acc / ks)
    return res

def lerp_stations(st, key, y):
    if y <= st[0]['y']: return st[0][key]
    for a, b in zip(st, st[1:]):
        if y <= b['y']:
            t = (y - a['y']) / (b['y'] - a['y']); return a[key] + (b[key] - a[key]) * t
    return st[-1][key]

class Profile:
    """Smoothed longitudinal profile: half / bottom / top as functions of y, swept by a superellipse."""
    def __init__(self, stations, n_top, n_bot, tumble=0.0, step=0.02, sigma=0.05, n_top_fn=None):
        self.n_top, self.n_bot, self.tumble = n_top, n_bot, tumble
        self.n_top_fn = n_top_fn
        self.y0, self.y1 = stations[0]['y'], stations[-1]['y']
        cnt = max(2, int(round((self.y1 - self.y0) / step)) + 1)
        self.ys = [self.y0 + (self.y1 - self.y0) * i / (cnt - 1) for i in range(cnt)]
        self.arr = {k: gauss_smooth([lerp_stations(stations, k, y) for y in self.ys], sigma / step) for k in ('half', 'bottom', 'top')}
    def at(self, key, y):
        ys = self.ys; a = self.arr[key]
        if y <= ys[0]: return a[0]
        if y >= ys[-1]: return a[-1]
        f = (y - ys[0]) / (ys[-1] - ys[0]) * (len(ys) - 1); i = int(f); t = f - i
        return a[i] + (a[min(i + 1, len(a) - 1)] - a[i]) * t
    def ntop(self, y):
        return self.n_top_fn(y) if self.n_top_fn else self.n_top
    def point(self, y, t, expand=0.0):
        half, bottom, top = self.at('half', y), self.at('bottom', y), self.at('top', y)
        c, s = math.cos(t), math.sin(t)
        n = self.ntop(y) if s >= 0 else self.n_bot
        sx = math.copysign(abs(c) ** (2.0 / n), c); sz = math.copysign(abs(s) ** (2.0 / n), s)
        zf = 0.5 + 0.5 * sz
        x = (half + expand) * (1.0 - self.tumble * zf) * sx
        z = bottom - expand * 0.5 + (top - bottom + expand) * zf
        return x, y, z
    def t_for_zf(self, zf, side=1, y=0.0):
        sz = 2 * zf - 1
        n = self.ntop(y) if sz >= 0 else self.n_bot
        s = math.copysign(abs(sz) ** (n / 2.0), sz)
        t = math.asin(max(-1, min(1, s)))
        return t if side == 1 else math.pi - t
    def t_for_sx(self, sx_target, y=0.0):
        c = math.copysign(abs(sx_target) ** (self.ntop(y) / 2.0), sx_target)
        return math.acos(max(-1, min(1, c)))

def loft(name, prof, material, segs=36, step=0.06, cap=True, expand=0.0):
    cnt = max(2, int(round((prof.y1 - prof.y0) / step)) + 1)
    ys = [prof.y0 + (prof.y1 - prof.y0) * i / (cnt - 1) for i in range(cnt)]
    ts = [2 * math.pi * i / segs for i in range(segs)]
    bm = bmesh.new(); rings = []
    for y in ys:
        rings.append([bm.verts.new(prof.point(y, t, expand)) for t in ts])
    for a, b in zip(rings, rings[1:]):
        for j in range(segs):
            j2 = (j + 1) % segs
            try: bm.faces.new((a[j], a[j2], b[j2], b[j]))
            except ValueError: pass
    if cap:
        for ring, rev in ((rings[0], True), (rings[-1], False)):
            try: bm.faces.new(list(reversed(ring)) if rev else ring)
            except ValueError: pass
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return new_obj(name, bm, material)

def patch(name, prof, material, t_range, y_range_fn, expand, nu=12, nv=6, thickness=0.0):
    """Surface patch on the profile. t_range=(t0,t1); y_range_fn(t)->(y0,y1) so edges can be raked."""
    bm = bmesh.new(); rows = []
    for i in range(nv):
        t = t_range[0] + (t_range[1] - t_range[0]) * i / (nv - 1)
        y0, y1 = y_range_fn(t)
        rows.append([bm.verts.new(prof.point(y0 + (y1 - y0) * j / (nu - 1), t, expand)) for j in range(nu)])
    for a, b in zip(rows, rows[1:]):
        for j in range(nu - 1):
            try: bm.faces.new((a[j], a[j + 1], b[j + 1], b[j]))
            except ValueError: pass
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    ob = new_obj(name, bm, material)
    if thickness > 0:
        s = ob.modifiers.new('Solidify', 'SOLIDIFY'); s.thickness = thickness; s.offset = -1
    return ob

def body_surface_x(prof, y, z):
    """x of the right flank at height z (bisection on t)."""
    lo, hi = -math.pi / 2 + 0.01, math.pi / 2 - 0.01
    for _ in range(36):
        mid = (lo + hi) / 2
        if prof.point(y, mid)[2] < z: lo = mid
        else: hi = mid
    return prof.point(y, (lo + hi) / 2)[0]

def body_surface_y(prof, x, z, front=True):
    """y where the plan outline at height z reaches |x| (nose if front, tail otherwise)."""
    ax = abs(x)
    if front:
        lo, hi = prof.y0, prof.y0 + 1.0
        if body_surface_x(prof, lo + 0.002, z) >= ax: return lo
        for _ in range(36):
            mid = (lo + hi) / 2
            if body_surface_x(prof, mid, z) < ax: lo = mid
            else: hi = mid
        return (lo + hi) / 2
    lo, hi = prof.y1 - 1.0, prof.y1
    if body_surface_x(prof, hi - 0.002, z) >= ax: return hi
    for _ in range(36):
        mid = (lo + hi) / 2
        if body_surface_x(prof, mid, z) < ax: hi = mid
        else: lo = mid
    return (lo + hi) / 2

def arch_lip(name, prof, ay, R, arch_r, minor, material, proud=0.004, steps=30, ring=6, z_min=0.22):
    """Tube following the arch opening on the body flank."""
    bm = bmesh.new(); rings = []
    a0 = math.asin(max(-1, min(1, (z_min - R) / arch_r))); a1 = math.pi - a0
    for i in range(steps + 1):
        a = a0 + (a1 - a0) * i / steps
        y, z = ay + arch_r * math.cos(a), R + arch_r * math.sin(a)
        x = body_surface_x(prof, y, z) + proud
        radial = Vector((0, math.cos(a), math.sin(a)))
        rings.append([bm.verts.new(Vector((x, y, z)) + minor * (math.cos(b) * radial + math.sin(b) * Vector((1, 0, 0)))) for b in [2 * math.pi * j / ring for j in range(ring)]])
    for r0, r1 in zip(rings, rings[1:]):
        for j in range(ring):
            bm.faces.new((r0[j], r0[(j + 1) % ring], r1[(j + 1) % ring], r1[j]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return new_obj(name, bm, material)

def export_glb(path):
    """Export the Fleet collection in the runtime frame (see scripts/assets/vehicle_frame.py)."""
    import vehicle_frame
    cars = list(root().all_objects)
    bpy.ops.object.select_all(action='DESELECT')
    for ob in cars: ob.select_set(True)
    vehicle_frame.export_runtime_frame(cars, lambda: bpy.ops.export_scene.gltf(
        filepath=path, use_selection=True, export_apply=True, export_yup=True,
        export_materials='EXPORT', export_format='GLB', export_animations=False, export_skins=False,
        export_texcoords=False, export_normals=True, export_extras=True))
    bpy.ops.object.select_all(action='DESELECT')
    return path

def tri_count():
    dg = bpy.context.evaluated_depsgraph_get(); total = 0
    for ob in root().all_objects:
        if ob.type != 'MESH': continue
        me = ob.evaluated_get(dg).data
        total += sum(len(p.vertices) - 2 for p in me.polygons)
    return total
