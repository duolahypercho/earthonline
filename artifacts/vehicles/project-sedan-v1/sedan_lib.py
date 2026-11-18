import bpy, bmesh, math
from mathutils import Vector, Matrix

# ---------------- dimensions (metres) ----------------
L, W, H = 4.72, 1.84, 1.44
WB, TRACK = 2.85, 1.56
R, WT = 0.335, 0.235
R_RIM = 0.245
FRONT_Y, REAR_Y = -1.47, -1.47 + WB
ROOT_NAME = 'Sedan'

# ---------------- detail level ----------------
# LOD1/LOD2 used to be a 30% decimate of LOD0, and the
# decimated panels shaded as creases at 16-70 m. Each level is now a CLEAN
# rebuild at its own resolution: `Q['scale']` divides every segment count and
# multiplies every loft step, so a 96-segment envelope becomes a 48- or
# 24-segment one with the same silhouette and no long triangles.
Q = dict(scale=1.0, level=0)
def set_quality(level):
    Q['level'] = level
    Q['scale'] = {0: 1.0, 1: 0.5, 2: 0.25}.get(level, 1.0)
def qs(n, floor=8):
    """segment count at the current quality"""
    return max(floor, int(round(n * Q['scale'])))
def qstep(step):
    return step / Q['scale']

# ---------------- materials ----------------
def mat(name, color, metallic=0.0, rough=0.5, coat=0.0, trans=0.0, emit=None, emit_str=0.0, alpha=1.0, spec=0.5):
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes['Principled BSDF']
    b.inputs['Base Color'].default_value = (*color, 1)
    b.inputs['Metallic'].default_value = metallic
    b.inputs['Roughness'].default_value = rough
    for k, v in (('Coat Weight', coat), ('Clearcoat', coat), ('Transmission Weight', trans), ('Transmission', trans),
                 ('Emission Strength', emit_str), ('Alpha', alpha), ('Specular IOR Level', spec), ('Specular', spec)):
        if k in b.inputs: b.inputs[k].default_value = v
    if emit is not None:
        for k in ('Emission Color', 'Emission'):
            if k in b.inputs: b.inputs[k].default_value = (*emit, 1)
    if trans > 0 or alpha < 1:
        try: m.blend_method = 'BLEND'
        except Exception: pass
        try: m.surface_render_method = 'BLENDED'
        except Exception: pass
    return m

def materials():
    M = {}
    M['paint'] = mat('VehiclePaint', (0.055, 0.12, 0.30), metallic=0.85, rough=0.30, coat=1.0)
    M['glass'] = mat('VehicleGlass', (0.10, 0.12, 0.14), rough=0.02, alpha=0.28, spec=0.9)
    M['seat'] = mat('VehicleSeat', (0.045, 0.040, 0.038), rough=0.85)
    M['seatInsert'] = mat('VehicleSeatInsert', (0.11, 0.10, 0.095), rough=0.9)
    M['carpet'] = mat('VehicleCarpet', (0.03, 0.028, 0.026), rough=1.0)
    M['dash'] = mat('VehicleDash', (0.035, 0.033, 0.032), rough=0.75)
    M['headliner'] = mat('VehicleHeadliner', (0.14, 0.135, 0.125), rough=0.95)
    M['screen'] = mat('VehicleScreen', (0.02, 0.03, 0.05), rough=0.1, emit=(0.15, 0.35, 0.6), emit_str=0.6)
    M['dial'] = mat('VehicleDial', (0.02, 0.02, 0.025), rough=0.2, emit=(0.9, 0.9, 1.0), emit_str=0.25)
    M['trim'] = mat('VehicleTrimBlack', (0.012, 0.012, 0.013), rough=0.5)
    M['gloss'] = mat('VehicleTrimGloss', (0.01, 0.01, 0.01), rough=0.15, coat=0.6)
    M['chrome'] = mat('VehicleChrome', (0.9, 0.9, 0.9), metallic=1.0, rough=0.12)
    M['rubber'] = mat('VehicleTyre', (0.02, 0.02, 0.02), rough=0.8)
    M['tread'] = mat('VehicleTread', (0.008, 0.008, 0.008), rough=0.95)
    M['alloy'] = mat('VehicleAlloy', (0.75, 0.76, 0.78), metallic=1.0, rough=0.18)
    M['darkalloy'] = mat('VehicleAlloyDark', (0.06, 0.06, 0.065), metallic=0.6, rough=0.7)
    M['steel'] = mat('VehicleSteel', (0.4, 0.4, 0.42), metallic=1.0, rough=0.55)
    M['headlamp'] = mat('VehicleHeadlamp', (0.05, 0.06, 0.08), rough=0.02, alpha=0.3, spec=0.9)
    M['lens'] = mat('VehicleProjector', (0.75, 0.8, 0.9), rough=0.1, metallic=0.4, emit=(1.0, 0.97, 0.9), emit_str=1.5)
    M['taillamp'] = mat('VehicleTaillamp', (0.45, 0.01, 0.01), rough=0.05, alpha=0.7, emit=(1.0, 0.05, 0.02), emit_str=0.15)
    M['amber'] = mat('VehicleIndicator', (0.9, 0.45, 0.05), rough=0.15, emit=(1.0, 0.5, 0.05), emit_str=0.2)
    # one material per light function so the runtime can switch them independently
    M['lowbeam'] = mat('VehicleLowBeam', (0.75, 0.8, 0.9), rough=0.1, metallic=0.4, emit=(1.0, 0.97, 0.9), emit_str=1.5)
    M['highbeam'] = mat('VehicleHighBeam', (0.75, 0.8, 0.9), rough=0.1, metallic=0.4, emit=(1.0, 0.98, 0.92), emit_str=0.5)
    M['drl'] = mat('VehicleDRL', (0.85, 0.88, 0.95), rough=0.1, emit=(1.0, 0.98, 0.95), emit_str=2.0)
    M['tail'] = mat('VehicleTailLight', (0.5, 0.02, 0.02), rough=0.1, emit=(1.0, 0.06, 0.02), emit_str=1.0)
    M['brake'] = mat('VehicleBrakeLamp', (0.55, 0.02, 0.02), rough=0.1, emit=(1.0, 0.05, 0.02), emit_str=0.3)
    M['reverse'] = mat('VehicleReverseLamp', (0.85, 0.85, 0.8), rough=0.1, emit=(1.0, 1.0, 0.95), emit_str=0.1)
    M['fog'] = mat('VehicleFogLamp', (0.8, 0.82, 0.85), rough=0.1, emit=(1.0, 0.95, 0.85), emit_str=0.2)
    M['dome'] = mat('VehicleDomeLight', (0.9, 0.9, 0.85), rough=0.3, emit=(1.0, 0.95, 0.85), emit_str=0.3)
    M['interior'] = mat('VehicleInterior', (0.05, 0.045, 0.045), rough=0.9)
    M['plate'] = mat('VehiclePlate', (0.92, 0.92, 0.9), rough=0.4)
    M['caliper'] = mat('VehicleCaliper', (0.55, 0.05, 0.04), rough=0.5)
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

def prim_cyl(name, radius, depth, loc, axis='X', segs=64, mat=None, do_smooth=True, r2=None):
    segs = qs(segs)
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, cap_tris=False, segments=segs, radius1=radius, radius2=radius if r2 is None else r2, depth=depth)
    if axis == 'X': bmesh.ops.rotate(bm, verts=bm.verts, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, 'Y'))
    elif axis == 'Y': bmesh.ops.rotate(bm, verts=bm.verts, cent=(0, 0, 0), matrix=Matrix.Rotation(math.pi / 2, 3, 'X'))
    ob = new_obj(name, bm, mat or bpy.data.materials['VehicleTrimBlack'], do_smooth)
    ob.location = loc
    return ob

def prim_box(name, size, loc, mat=None, bevel=0.0, bevel_segs=3, rot=(0, 0, 0)):
    bm = bmesh.new(); bmesh.ops.create_cube(bm, size=1.0); bmesh.ops.scale(bm, vec=size, verts=bm.verts)
    ob = new_obj(name, bm, mat or bpy.data.materials['VehicleTrimBlack'], do_smooth=bevel > 0)
    ob.location = loc; ob.rotation_euler = rot
    if bevel > 0:
        b = ob.modifiers.new('Bevel', 'BEVEL'); b.width = bevel; b.segments = bevel_segs; b.limit_method = 'NONE'
    return ob

def boolean_apply(target, cutter, op='DIFFERENCE', keep=False, solvers=('EXACT', 'EXACT_SELF')):
    """Apply a boolean with the EXACT solver (keeps quads). EXACT's inside/outside classification can flip
    with vertex order on these lofts and return an empty mesh; on an empty result from a non-empty target the
    target is restored and the op retried with self-intersection handling (EXACT_SELF), which is consistent
    but ~15x slower. MANIFOLD is avoided: it triangulates into shading fans."""
    backup = target.data.copy()
    for sv in solvers:
        mod = target.modifiers.new('Bool', 'BOOLEAN'); mod.operation = op; mod.object = cutter
        try:
            mod.solver = 'EXACT' if sv == 'EXACT_SELF' else sv
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

def apply_all(ob):
    bpy.context.view_layer.objects.active = ob
    for m in list(ob.modifiers):
        try: bpy.ops.object.modifier_apply(modifier=m.name)
        except Exception as e: print('apply fail', ob.name, m.name, e)

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
    """Smoothed longitudinal profile: half / bottom / top as functions of y."""
    def __init__(self, stations, n_top, n_bot, tumble=0.0, step=0.02, sigma=0.05):
        self.n_top_v, self.n_bot, self.tumble = n_top, n_bot, tumble
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
    def ntop(self, y=0.0):
        """top-half superellipse exponent; a callable lets the shoulder get squarer through the cabin"""
        return self.n_top_v(y) if callable(self.n_top_v) else self.n_top_v
    @property
    def n_top(self): return self.ntop(0.0)
    def point(self, y, t, expand=0.0):
        half, bottom, top = self.at('half', y), self.at('bottom', y), self.at('top', y)
        c, s = math.cos(t), math.sin(t)
        n = self.ntop(y) if s >= 0 else self.n_bot
        sx = math.copysign(abs(c) ** (2.0 / n), c); sz = math.copysign(abs(s) ** (2.0 / n), s)
        zf = 0.5 + 0.5 * sz
        x = (half + expand) * (1.0 - self.tumble * zf) * sx
        z = bottom - expand * 0.5 + (top - bottom + expand) * zf
        return x, y, z
    def t_for_zf(self, zf, side=1, y=None):
        """angle giving vertical fraction zf on the right (side=1) or left (side=-1) flank"""
        sz = 2 * zf - 1
        n = self.ntop(0.0 if y is None else y) if sz >= 0 else self.n_bot
        s = math.copysign(abs(sz) ** (n / 2.0), sz)
        t = math.asin(max(-1, min(1, s)))
        return t if side == 1 else math.pi - t
    def t_for_sx(self, sx_target, y=None):
        """angle on the top arc where horizontal fraction = sx_target (positive right)"""
        c = math.copysign(abs(sx_target) ** (self.ntop(0.0 if y is None else y) / 2.0), sx_target)
        return math.acos(max(-1, min(1, c)))

def loft(name, prof, material, segs=64, step=0.025, cap=True, expand=0.0, y_range=None):
    segs = qs(segs, 12); step = qstep(step)
    y0, y1 = y_range if y_range else (prof.y0, prof.y1)
    cnt = max(2, int(round((y1 - y0) / step)) + 1)
    ys = [y0 + (y1 - y0) * i / (cnt - 1) for i in range(cnt)]
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
            try:
                f = bm.faces.new(list(reversed(ring)) if rev else ring)
                for e in f.edges: e.smooth = False          # flat cap, crisp rim
            except ValueError: pass
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return new_obj(name, bm, material)

# ---------------- envelope loft: body + greenhouse as one clean surface ----------------
TWO_PI = 2 * math.pi

def _ray_table(points, c, nbins=720):
    """Segments of a closed section polyline (about c=(x,z)), bucketed by the angle range they cover."""
    pts = [(x - c[0], z - c[1]) for x, _, z in points]
    n = len(pts); bins = [[] for _ in range(nbins)]
    for i in range(n):
        (x0, z0), (x1, z1) = pts[i], pts[(i + 1) % n]
        if abs(x1 - x0) < 1e-9 and abs(z1 - z0) < 1e-9: continue
        a0, a1 = math.atan2(z0, x0) % TWO_PI, math.atan2(z1, x1) % TWO_PI
        lo, hi = min(a0, a1), max(a0, a1)
        ranges = [(hi, TWO_PI), (0.0, lo)] if hi - lo > math.pi else [(lo, hi)]
        seg = (x0, z0, x1 - x0, z1 - z0)
        for r0, r1 in ranges:
            for b in range(int(r0 / TWO_PI * nbins), min(nbins - 1, int(r1 / TWO_PI * nbins)) + 1): bins[b].append(seg)
    return bins

def _r_far(bins, ang):
    """Farthest intersection of the ray from c at `ang` with the section, or None if it misses.
    For the body (c inside) that is the single crossing; for the glasshouse (c below it) the exit point."""
    a = ang % TWO_PI; dx, dz = math.cos(a), math.sin(a)
    best = None
    for x0, z0, ex, ez in bins[int(a / TWO_PI * len(bins))]:
        den = dx * ez - dz * ex
        if abs(den) < 1e-12: continue
        s = (x0 * dz - z0 * dx) / den
        r = (x0 * ez - z0 * ex) / den
        if -1e-9 <= s <= 1 + 1e-9 and r > 0 and (best is None or r > best): best = r
    return best

def envelope_loft(name, body_prof, green_prof, material, segs=96, step=0.015, k_top=48, theta_default=math.radians(20), samples=512, blend=0.012):
    segs = qs(segs, 16); k_top = segs // 2; step = qstep(step)
    """One closed loft whose section is the outer envelope of the body and the greenhouse, blended with a
    soft maximum (~`blend` fillet). The glasshouse/shoulder crossing sits on fixed vertex columns (index 0
    on the right, k_top on the left) that drift only slowly along the car, so the strips never shear."""
    cnt = max(2, int(round((body_prof.y1 - body_prof.y0) / step)) + 1)
    ys = [body_prof.y0 + (body_prof.y1 - body_prof.y0) * i / (cnt - 1) for i in range(cnt)]
    k_bot = segs - k_top
    ts = [TWO_PI * i / samples for i in range(samples)]
    # pass 1: per-ring ray tables and the raw flank crossing angle (None where there is no flank crossing)
    per = []
    for y in ys:
        c = (0.0, 0.5 * (body_prof.at('bottom', y) + body_prof.at('top', y)))
        tb = _ray_table([body_prof.point(y, t) for t in ts], c)
        tg = None
        if green_prof.y0 + 0.01 <= y <= green_prof.y1 - 0.01 and green_prof.at('top', y) - green_prof.at('bottom', y) > 0.02:
            tg = _ray_table([green_prof.point(y, t) for t in ts], c)
        theta_c = None
        if tg is not None and -0.70 <= y <= 1.45:      # follow the flank crossing only along the doors;
            n_scan = 200; a0, a1 = -0.1, math.radians(35); found = None   # cowl/deck tips blend without a column
            for i in range(n_scan + 1):
                a = a0 + (a1 - a0) * i / n_scan
                rg, rb = _r_far(tg, a), _r_far(tb, a)
                if rg is not None and rb is not None and rg > rb + 1e-5: found = a; break
            if found is not None:
                lo, hi = found - (a1 - a0) / n_scan, found
                for _ in range(30):
                    mid = 0.5 * (lo + hi); rg, rb = _r_far(tg, mid), _r_far(tb, mid)
                    if rg is not None and rb is not None and rg > rb: hi = mid
                    else: lo = mid
                theta_c = 0.5 * (lo + hi)
        per.append((c, tb, tg, theta_c))
    # pass 2: a slowly drifting crease column (gaps filled from neighbours, <= 0.5 deg per ring)
    thetas = [t for _, _, _, t in per]
    first = next((t for t in thetas if t is not None), theta_default); last = first
    for i, t in enumerate(thetas):
        if t is None: thetas[i] = last
        else: last = t
    max_step = math.radians(0.5)
    for _ in range(3):
        for i in range(1, len(thetas)): thetas[i] = max(thetas[i - 1] - max_step, min(thetas[i - 1] + max_step, thetas[i]))
        for i in range(len(thetas) - 2, -1, -1): thetas[i] = max(thetas[i + 1] - max_step, min(thetas[i + 1] + max_step, thetas[i]))
    # pass 3: rings
    bm = bmesh.new(); rings = []
    for y, (c, tb, tg, _), theta_c in zip(ys, per, thetas):
        def r_env(ang):
            rb = _r_far(tb, ang)
            if tg is None: return rb
            rg = _r_far(tg, ang)
            if rg is None: return rb
            return 0.5 * (rb + rg + math.sqrt((rb - rg) ** 2 + blend * blend))   # soft max
        ring = []
        for k in range(k_top):                        # right crossing over the roof to the left crossing
            ang = theta_c + (math.pi - 2 * theta_c) * k / k_top
            r = r_env(ang); ring.append(bm.verts.new((c[0] + r * math.cos(ang), y, c[1] + r * math.sin(ang))))
        for k in range(k_bot):                        # left crossing under the floor back to the right crossing
            ang = (math.pi - theta_c) + (2 * theta_c + math.pi) * k / k_bot
            r = r_env(ang); ring.append(bm.verts.new((c[0] + r * math.cos(ang), y, c[1] + r * math.sin(ang))))
        rings.append(ring)
    for a, b in zip(rings, rings[1:]):
        for j in range(segs):
            j2 = (j + 1) % segs
            try: bm.faces.new((a[j], a[j2], b[j2], b[j]))
            except ValueError: pass
    for ring, rev in ((rings[0], True), (rings[-1], False)):
        try:
            f = bm.faces.new(list(reversed(ring)) if rev else ring)
            for e in f.edges: e.smooth = False              # flat cap, crisp rim
        except ValueError: pass
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return new_obj(name, bm, material)

def patch(name, prof, material, t_range, y_range_fn, expand, nu=24, nv=12, thickness=0.0, zf_range=None, side=1):
    nu = qs(nu, 3); nv = qs(nv, 2)
    """Surface patch on the profile. t_range=(t0,t1); y_range_fn(t)->(y0,y1) so edges can be raked.
    With zf_range=(zf0,zf1) the rows follow a constant vertical fraction instead (y_range_fn receives zf)."""
    bm = bmesh.new(); rows = []
    for i in range(nv):
        f = i / (nv - 1)
        if zf_range is not None:
            zf = zf_range[0] + (zf_range[1] - zf_range[0]) * f
            y0, y1 = y_range_fn(zf)
            ys = [y0 + (y1 - y0) * j / (nu - 1) for j in range(nu)]
            rows.append([bm.verts.new(prof.point(yj, prof.t_for_zf(zf, side, yj), expand)) for yj in ys])
            continue
        t = t_range[0] + (t_range[1] - t_range[0]) * f
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

# ---------------- body profiles ----------------
BODY_ST = [
    dict(y=-2.36, bottom=0.28, top=0.635, half=0.78),
    dict(y=-2.28, bottom=0.235, top=0.665, half=0.855),
    dict(y=-2.05, bottom=0.19, top=0.70, half=0.89),
    dict(y=-1.60, bottom=0.17, top=0.75, half=0.905),
    dict(y=-1.00, bottom=0.17, top=0.80, half=0.92),
    dict(y=-0.70, bottom=0.17, top=0.83, half=0.92),
    dict(y=0.00, bottom=0.17, top=0.875, half=0.92),
    dict(y=1.00, bottom=0.17, top=0.90, half=0.92),
    dict(y=1.55, bottom=0.17, top=0.93, half=0.91),
    dict(y=1.95, bottom=0.19, top=0.955, half=0.875),
    dict(y=2.22, bottom=0.24, top=0.94, half=0.83),
    dict(y=2.36, bottom=0.30, top=0.89, half=0.76),
]
GREEN_ST = [
    dict(y=-0.92, bottom=0.77, top=0.785, half=0.84),    # cowl/deck tips taper in plan: only the roof skin
    dict(y=-0.80, bottom=0.78, top=0.86, half=0.865),    # emerges from the body there, never a tangent flank
    dict(y=-0.10, bottom=0.81, top=1.36, half=0.90),
    dict(y=0.12, bottom=0.815, top=1.42, half=0.895),
    dict(y=0.40, bottom=0.82, top=1.44, half=0.89),
    dict(y=0.95, bottom=0.835, top=1.435, half=0.885),
    dict(y=1.22, bottom=0.845, top=1.40, half=0.885),
    dict(y=1.75, bottom=0.88, top=0.98, half=0.845),
    dict(y=1.88, bottom=0.895, top=0.91, half=0.82),
]

def _smooth(a, b, x):
    t = min(1.0, max(0.0, (x - a) / (b - a))); return t * t * (3 - 2 * t)
def body_ntop(y):
    """soft nose (4.4), square shoulders through the cabin (9), moderately square tail (6.2)"""
    return 4.4 + 4.6 * _smooth(-1.55, -0.95, y) * (1 - 0.6 * _smooth(1.85, 2.36, y))
def body_profile(): return Profile(BODY_ST, n_top=body_ntop, n_bot=6.0, sigma=0.045)
def green_profile(): return Profile(GREEN_ST, n_top=3.0, n_bot=10.0, tumble=0.31, sigma=0.035)

def body_surface_x(prof, y, z):
    """x of the right flank at height z (bisection on t)."""
    lo, hi = -math.pi / 2 + 0.01, math.pi / 2 - 0.01
    for _ in range(40):
        mid = (lo + hi) / 2
        if prof.point(y, mid)[2] < z: lo = mid
        else: hi = mid
    return prof.point(y, (lo + hi) / 2)[0]

def torus_x(name, major, minor, loc, material, steps=96, ring=12):
    steps = qs(steps, 12); ring = qs(ring, 5)
    """Torus whose axis is X (for fender lips / wheel details)."""
    bm = bmesh.new()
    vs = [bm.verts.new((minor * math.cos(a), 0, major + minor * math.sin(a))) for a in [2 * math.pi * i / ring for i in range(ring)]]
    es = [bm.edges.new((vs[i], vs[(i + 1) % ring])) for i in range(ring)]
    bmesh.ops.spin(bm, geom=vs + es, cent=(0, 0, 0), axis=(1, 0, 0), angle=2 * math.pi, steps=steps, use_merge=True)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    ob = new_obj(name, bm, material); ob.location = loc
    return ob

def arch_lip(name, prof, ay, arch_r, minor, material, proud=0.004, steps=72, ring=10, z_min=0.22):
    steps = qs(steps, 12); ring = qs(ring, 5)
    """Tube following the arch opening on the body flank (x from the analytic profile)."""
    bm = bmesh.new(); rings = []
    a0, a1 = math.asin((z_min - R) / arch_r), math.pi - math.asin((z_min - R) / arch_r)
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

def setup_world():
    sc = bpy.context.scene
    wd = sc.world or bpy.data.worlds.new('World'); sc.world = wd; wd.use_nodes = True
    nt = wd.node_tree
    for n in list(nt.nodes): nt.nodes.remove(n)
    out = nt.nodes.new('ShaderNodeOutputWorld'); bg = nt.nodes.new('ShaderNodeBackground'); sky = nt.nodes.new('ShaderNodeTexSky')
    sky.sky_type = 'MULTIPLE_SCATTERING'; sky.sun_elevation = math.radians(38); sky.sun_rotation = math.radians(150); sky.sun_intensity = 0.15; sky.altitude = 50
    try: sky.sun_disc = False
    except Exception: pass
    bg.inputs['Strength'].default_value = 0.35
    nt.links.new(sky.outputs[0], bg.inputs['Color']); nt.links.new(bg.outputs[0], out.inputs['Surface'])

def body_surface_y(prof, x, z, front=True):
    """y where the body's plan outline at height z reaches |x| (nose if front, tail otherwise)."""
    ax = abs(x)
    if front:
        lo, hi = prof.y0, prof.y0 + 0.8   # outline half grows from lo to hi
        if body_surface_x(prof, lo + 0.002, z) >= ax: return lo
        for _ in range(40):
            mid = (lo + hi) / 2
            if body_surface_x(prof, mid, z) < ax: lo = mid
            else: hi = mid
        return (lo + hi) / 2
    lo, hi = prof.y1 - 0.8, prof.y1
    if body_surface_x(prof, hi - 0.002, z) >= ax: return hi
    for _ in range(40):
        mid = (lo + hi) / 2
        if body_surface_x(prof, mid, z) < ax: hi = mid
        else: lo = mid
    return (lo + hi) / 2

def export_glb(path):
    """Export the Sedan collection as glTF binary, Y-up, modifiers applied."""
    import os, sys
    sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', '..', 'scripts', 'assets'))
    import vehicle_frame
    cars = list(root().all_objects)
    bpy.ops.object.select_all(action='DESELECT')
    for ob in cars: ob.select_set(True)
    # Runtime frame: the recipe's "X right, -Y nose" frame is mirrored (vehicle_frame.py).
    vehicle_frame.export_runtime_frame(cars, lambda: bpy.ops.export_scene.gltf(
        filepath=path, use_selection=True, export_apply=True, export_yup=True,
        export_materials='EXPORT', export_format='GLB', export_animations=False, export_skins=False,
        export_texcoords=True, export_normals=True, export_extras=True))
    bpy.ops.object.select_all(action='DESELECT')
    return path

def tri_count():
    dg = bpy.context.evaluated_depsgraph_get(); total = 0
    for ob in root().all_objects:
        if ob.type != 'MESH': continue
        me = ob.evaluated_get(dg).data
        total += sum(len(p.vertices) - 2 for p in me.polygons)
    return total

def export_lod(path, ratio=0.35, drop_prefixes=('Headlamp_', 'Taillamp_', 'Indicator_', 'BrakeLamp_', 'ReverseLamp_', 'FogLamp_', 'HighMountStop', 'DomeLight', 'SunVisor', 'Seat', 'Steering', 'Dash', 'Floor', 'Glovebox', 'Console', 'Gear', 'Pedal', 'FuelCap', 'FuelWell', 'Wheel_FL_Groove', 'Wheel_FR_Groove', 'Wheel_RL_Groove', 'Wheel_RR_Groove', 'Wheel_FL_Disc', 'Wheel_FR_Disc', 'Wheel_RL_Disc', 'Wheel_RR_Disc', 'Wheel_FL_Caliper', 'Wheel_FR_Caliper', 'Wheel_RL_Caliper', 'Wheel_RR_Caliper', 'Shut', 'BonnetLine', 'BootLine', 'BumperSeam', 'DiffuserFin', 'Wiper', 'Mirror_L_Glass', 'Mirror_R_Glass')):
    """Export a reduced LOD: decimate the large lofts, drop small interior/lamp detail parts."""
    keep_lamp = ('_Housing', '_Cover', '_Bar', '_Tail')   # lamp cluster parts that survive LOD1
    added = []
    for ob in list(root().all_objects):
        if ob.type != 'MESH': continue
        name = ob.name
        drop = any(name.startswith(p) for p in drop_prefixes) and not any(k in name for k in keep_lamp)
        if drop:
            ob.hide_render = True; ob.hide_viewport = True; ob.select_set(False); ob['lod_hidden'] = True
            continue
        if len(ob.data.polygons) > 400:
            d = ob.modifiers.new('LODDecimate', 'DECIMATE'); d.ratio = ratio; added.append((ob, d))
    bpy.ops.object.select_all(action='DESELECT')
    for ob in root().all_objects:
        if not ob.get('lod_hidden'): ob.select_set(True)
    bpy.ops.export_scene.gltf(filepath=path, use_selection=True, export_apply=True, export_yup=True,
                              export_materials='EXPORT', export_format='GLB', export_animations=False, export_skins=False)
    # restore
    for ob, d in added: ob.modifiers.remove(d)
    for ob in list(root().all_objects):   # snapshot: mutating ID props invalidates the live iterator
        if ob.get('lod_hidden'):
            ob.hide_render = False; ob.hide_viewport = False; del ob['lod_hidden']
    bpy.ops.object.select_all(action='DESELECT')
    return path
