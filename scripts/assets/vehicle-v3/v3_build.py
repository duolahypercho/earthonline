"""Vehicle v3 assembly: one authored car from a body JSON.

build(cfg, lod) creates, in the current scene, the runtime node contract that
fleet-v2 established (Body, Door_XX pivots, Wheel_XX -> _Spin, Hood/Trunk lids
with hinge extras, lamps, glass, plates) with a feature-line body measured from a
reference trace. See bodies/*.json for the numbers.
"""
import json
import math
from pathlib import Path

import bmesh
import bpy
from mathutils import Vector

import v3_body as V
import v3_parts as P

LOD = {
    0: dict(step=0.021, spans=0, wseg=56, gap=0.0028, flange=0.014, lamp=True, spokes=True, brakes=True,
            skin=0.10, honey=1.0, bevel=True, tyre='full', lampStep=0.006, spokeCount=10, liners=True, handles=True),
    # mid and far tiers are rationed by the hero overlay (vehicle-hero-overlay.js
    # triangleBudget): about 22k per mid body and 6k per far body, cabin included
    1: dict(step=0.110, spans=1, wseg=18, gap=0.0, flange=0.0, lamp=True, spokes=True, brakes=False,
            skin=0.0, honey=0.0, bevel=False, tyre='lite', lampStep=0.025, spokeCount=5, liners=True, handles=True),
    2: dict(step=0.210, spans=2, wseg=9, gap=0.0, flange=0.0, lamp=False, spokes=False, brakes=False,
            skin=0.0, honey=0.0, bevel=False, tyre='lite', lampStep=0.0, spokeCount=0, liners=False, handles=False),
}

REG_BODY, REG_HOOD, REG_HATCH = 0, 1, 2
REG_DOOR = {'FL': 3, 'FR': 4, 'RL': 5, 'RR': 6}
REG_WS, REG_RW = 10, 11
REG_WIN = {'FL': 12, 'FR': 13, 'RL': 14, 'RR': 15}
REG_QTR = {'L': 16, 'R': 17}
REG_HEAD = {'L': 20, 'R': 21}
REG_TAIL = {'L': 22, 'R': 23}
REG_GRILLE, REG_INTAKE = 24, 25
REG_FOG = {'L': 26, 'R': 27}

SLOT = {'paint': 0, 'trim': 1, 'glass': 2, 'gloss': 3, 'cladding': 4}


def mirror_outline(pts):
    return [(-x, z) for x, z in reversed(pts)]


def build(cfg, lod=0, paint_mat=None, interior_glb=None):
    D = dict(LOD[lod])
    if lod == 1 and cfg.get('midLampStep') is not None:
        D['lampStep'] = max(D['lampStep'], float(cfg['midLampStep']))
    M = P.materials(paint_mat)
    if cfg.get('utilityTwoTone'):
        if paint_mat is None:
            M['paint'] = P.mat('VehiclePaint', (0.91, 0.89, 0.84), metallic=0.12, rough=0.32, coat=0.7)
        M['cladding'] = P.mat('VehicleUtilityCladding', (0.055, 0.11, 0.14), rough=0.43)
        M['glass'] = P.mat('VehicleGlass', (0.11, 0.13, 0.15), rough=0.07,
                           alpha=0.24, transmission=0.0)
        M['headcover'] = P.mat('VehicleHeadlamp', (0.018, 0.025, 0.03), rough=0.16)
        M['drl'] = P.mat('VehicleDRL', (0.86, 0.93, 1.0), rough=0.15,
                          emit=(0.88, 0.94, 1.0), emit_str=1.0)
        M['tail'] = P.mat('VehicleTaillamp', (0.62, 0.015, 0.012), rough=0.09,
                           emit=(1.0, 0.025, 0.015), emit_str=0.75)
        M['alloy'] = P.mat('VehicleAlloy', (0.78, 0.79, 0.80), metallic=0.45, rough=0.26)
        M['interior'] = P.mat('VehicleInterior', (0.075, 0.08, 0.085), rough=0.78)
        M['doorcard'] = P.mat('VehicleDoorCard', (0.11, 0.115, 0.12), rough=0.76)
    lp = cfg.get('liveryPaint')
    if lp and paint_mat is None:
        # a livery body (taxi, patrol) is drawn in its own colour: the runtime
        # recolours only `VehiclePaint`, and its livery paint set is near-white
        M['paint'] = P.mat(lp['name'], tuple(lp['colour']), metallic=0.2, rough=0.28, coat=1.0)
    if cfg.get('roofDelta'):
        # a trim that sits its roof lower/higher: blend the delta in over the greenhouse
        cfg = json.loads(json.dumps(cfg))
        a_, b_ = cfg['cowl'], cfg['hatchBase']
        pts = []
        for y, z in cfg['curves']['top']:
            w = 0.0 if y <= a_ or y >= b_ else min(1.0, (y - a_) / 0.5, (b_ - y) / 0.5)
            pts.append([y, z + cfg['roofDelta'] * w])
        cfg['curves']['top'] = pts
    spec = V.BodySpec(cfg)
    dr = cfg['doors']
    d0, d1 = dr['front']
    r0, r1 = dr.get('rear', (99.0, 99.0))
    cowl, rf, rr, hb = cfg['cowl'], cfg['roofFront'], cfg['roofRear'], cfg['hatchBase']
    hoodFront = cfg.get('hoodFront', spec.y0 + 0.11)
    tailCut = cfg.get('tailCut', spec.y1 - 0.13)
    hatchZ = cfg.get('hatchBottomZ', 0.70)
    g = cfg['glass']
    forced = [v for v in [d0, d1, r1, cowl, rf, rr, hb] if abs(v) < 50] + [ hoodFront, tailCut, g['front'], g['bFront'], g['bRear'], g['rear'],
              g['qFront'], g['qRear'], cowl - 0.10]
    # longer bodies take proportionally coarser stations: the overlay rations every
    # body against the same per-tier triangle budget, whatever its length
    step = D['step'] * max(1.0, spec.L / 4.85)
    shell, KR, ys = V.build_shell(spec, 'Body', M['paint'], lod=lod, step=step, forced=forced)
    me = shell.data
    for m in (M['trim'], M['glass'], M['gloss']):
        me.materials.append(m)
    if cfg.get('utilityTwoTone'):
        me.materials.append(M['cladding'])

    # pristine copy of the uncut shell: every cut piece takes its shading from it
    pristine = bpy.data.objects.new('ShellPristine', me.copy())
    P.link(pristine)
    pristine.hide_render = True
    from mathutils.bvhtree import BVHTree
    global SHELL_BVH
    SHELL_BVH = BVHTree.FromObject(pristine, bpy.context.evaluated_depsgraph_get())
    bm = bmesh.new()
    bm.from_mesh(me)
    bm.normal_update()
    ring = bm.verts.layers.int.get('ring')
    reg = bm.faces.layers.int.new('reg')
    K5, K6, K4, K2 = KR[5], KR[6], KR[4], KR[2]

    def rr_of(f):
        rs = [v[ring] for v in f.verts]
        return min(rs), max(rs)

    side_f = lambda f: abs(centroid(f).x) > 0.35 and abs(f.normal.x) > 0.25
    # ---- exact cuts that are not station or ring lines
    dz = dr['bottomZ']
    P.bisect_outline(bm, [(d0, dz), (r1 if 'rear' in dr else d1, dz)], 'yz', side_f, closed=False)
    slant = dr.get('rearEdge', [(99.0, 0.0), (99.0, 9.0)])
    if 'rear' in dr:
        P.bisect_outline(bm, slant, 'yz', side_f, closed=False)

    if cfg.get('utilityTwoTone'):
        cladding_height = cfg.get('claddingHeight', 0.65)
        bmesh.ops.bisect_plane(bm, geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
                               dist=1e-6, plane_co=Vector((0, 0, cladding_height)),
                               plane_no=Vector((0, 0, 1)))
    face_reach = 0.20 if cfg.get('utilityTwoTone') else 0.40
    face_normal = 0.35 if cfg.get('utilityTwoTone') else -0.15
    front_f = lambda f: centroid(f).y < spec.y0 + face_reach and f.normal.y < -face_normal
    rear_f = lambda f: centroid(f).y > spec.y1 - face_reach and f.normal.y > face_normal
    lamps = cfg['lamps']
    outl = {}
    for key, flt, reg_pair in (('head', front_f, REG_HEAD), ('tail', rear_f, REG_TAIL), ('fog', front_f, REG_FOG)):
        if key not in lamps:
            continue
        right = [tuple(p) for p in lamps[key]]
        for s, poly in (('R', right), ('L', mirror_outline(right))):
            P.bisect_outline(bm, poly, 'xz', flt)
            outl[(key, s)] = (poly, flt, reg_pair[s])
    for key, rid in (('grille', REG_GRILLE), ('intake', REG_INTAKE)):
        if key in lamps:
            poly = [tuple(p) for p in lamps[key]]
            P.bisect_outline(bm, poly, 'xz', front_f)
            outl[(key, '')] = (poly, front_f, rid)
    hatch_poly = None
    if 'hatchOutline' in cfg:
        right = [tuple(p) for p in cfg['hatchOutline']]
        hatch_poly = [(-x, z) for x, z in reversed(right)] + right
        hatch_f = lambda f: centroid(f).y > ({'trunk': hb, 'tailgate': spec.y1 - 0.30, 'reardoor': spec.y1 - 0.12}.get(cfg.get('boot'), rr + 0.10)) and f.normal.y > 0.05
        P.bisect_outline(bm, hatch_poly, 'xz', hatch_f)
    ws_poly = None
    if 'windscreenOutline' in cfg:
        right = [tuple(p) for p in cfg['windscreenOutline']]
        ws_poly = [(-x, z) for x, z in reversed(right)] + right
        P.bisect_outline(bm, ws_poly, 'xz', lambda f: centroid(f).y < spec.y0 + 0.5 and f.normal.y < -0.25)
    rw_poly = None
    if 'rearWindowOutline' in cfg:
        right = [tuple(p) for p in cfg['rearWindowOutline']]
        rw_poly = [(-x, z) for x, z in reversed(right)] + right
        P.bisect_outline(bm, rw_poly, 'xz', lambda f: centroid(f).y > rr and f.normal.y > 0.25)
    quarter_poly = [tuple(p) for p in cfg.get('quarterWindowOutline', [])]
    if quarter_poly:
        P.bisect_outline(bm, quarter_poly, 'yz', side_f)
    lipz, difz = cfg.get('frontLipZ', 0.30), cfg.get('rearDiffuserZ', 0.34)
    P.bisect_outline(bm, [(-1.2, lipz), (1.2, lipz)], 'xz', lambda f: centroid(f).y < spec.y0 + 0.36, closed=False)
    P.bisect_outline(bm, [(-1.2, difz), (1.2, difz)], 'xz', lambda f: centroid(f).y > spec.y1 - 0.36, closed=False)
    bm.normal_update()

    def line_y(pts, z):
        """y of a side-view polyline at height z."""
        for (ya, za), (yb, zb) in zip(pts, pts[1:]):
            if min(za, zb) - 1e-6 <= z <= max(za, zb) + 1e-6 and abs(zb - za) > 1e-9:
                return ya + (yb - ya) * (z - za) / (zb - za)
        return pts[0][0] if z < pts[0][1] else pts[-1][0]

    # ---- classify
    for f in bm.faces:
        c = centroid(f)
        rmin, rmax = rr_of(f)
        s = 'R' if c.x > 0 else 'L'
        top = rmin >= K6
        gband = rmin >= K5 and rmax <= K6
        lower = rmax <= K5
        f[reg] = REG_BODY
        f.material_index = SLOT['paint']
        if cfg.get('utilityTwoTone') and c.z < cfg.get('claddingHeight', 0.65):
            f.material_index = SLOT['cladding']
        # glass first
        if ws_poly is not None and c.y < spec.y0 + 0.5 and f.normal.y < -0.25 and P.point_in_poly((c.x, c.z), ws_poly):
            f[reg] = REG_WS
            continue
        if top and cowl + 0.005 < c.y < rf:
            f[reg] = REG_WS
            continue
        if top and rr + 0.05 < c.y < hb and not cfg.get('utilityTwoTone'):
            f[reg] = REG_RW
            continue
        if rw_poly is not None and c.y > rr and f.normal.y > 0.25 and P.point_in_poly((c.x, c.z), rw_poly):
            f[reg] = REG_RW
            continue
        inner = rmin >= K5 + 1 and rmax <= K6 - 1
        if inner and g['front'] < c.y < g['bFront']:
            f[reg] = REG_WIN['F' + s]
            continue
        if inner and g['bRear'] < c.y < g['rear']:
            f[reg] = REG_WIN['R' + s]
            continue
        if inner and g['qFront'] < c.y < g['qRear'] and (not quarter_poly or P.point_in_poly((c.y, c.z), quarter_poly)):
            f[reg] = REG_QTR[s]
            continue
        if inner and any(a_ < c.y < b_ for a_, b_ in cfg.get('sideWindows', [])):
            f[reg] = REG_QTR[s]
            continue
        # lids
        if top and hoodFront < c.y < cowl - 0.10:
            f[reg] = REG_HOOD
            continue
        if top and cowl - 0.10 < c.y < cowl:
            f.material_index = SLOT['trim']  # scuttle panel under the wipers
            continue
        if cfg.get('boot') == 'none':
            pass
        elif cfg.get('boot') == 'tailgate':
            if hatch_poly is not None and c.y > spec.y1 - 0.30 and f.normal.y > 0.05 and P.point_in_poly((c.x, c.z), hatch_poly):
                f[reg] = REG_HATCH
                continue
        elif cfg.get('boot') == 'reardoor':
            # a van's top-hinged rear door: the rear face inside its outline
            if hatch_poly is not None and c.y > spec.y1 - 0.12 and f.normal.y > 0.3 and P.point_in_poly((c.x, c.z), hatch_poly):
                f[reg] = REG_HATCH
                continue
        elif cfg.get('boot') == 'trunk':
            if top and hb < c.y and (c.y < spec.y1 - 0.02):
                f[reg] = REG_HATCH
                continue
            if hatch_poly is not None and c.y > hb and f.normal.y > 0.05 and P.point_in_poly((c.x, c.z), hatch_poly):
                f[reg] = REG_HATCH
                continue
        elif hatch_poly is not None:
            if c.y > rr and (top or (f.normal.y > 0.05 and P.point_in_poly((c.x, c.z), hatch_poly))):
                f[reg] = REG_HATCH
                continue
        elif c.y > rr and ((top and c.y < tailCut + 0.2) or (c.y > tailCut and c.z > hatchZ and rmin >= K4)):
            f[reg] = REG_HATCH
            continue
        # doors
        if (lower or gband) and c.z > dz and s in dr.get('sides', 'LR'):
            if d0 < c.y < d1:
                f[reg] = REG_DOOR['F' + s]
            elif 'rear' in dr and r0 < c.y < line_y(slant, c.z) and c.y < g['qFront'] - 0.01:
                f[reg] = REG_DOOR['R' + s]
            if f[reg] != REG_BODY:
                if gband and (g['bFront'] - 0.004 < c.y < g['bRear'] + 0.004):
                    f.material_index = SLOT['trim']  # black B-pillar applique
                continue
        # sill and bumper lower trims
        if rmax <= K2 + 1 and spec.y0 + 0.5 < c.y < spec.y1 - 0.5:
            f.material_index = SLOT['cladding'] if cfg.get('utilityTwoTone') else SLOT['trim']
        if c.z < difz and c.y > spec.y1 - 0.36:
            f.material_index = SLOT['trim']
        if c.z < lipz and c.y < spec.y0 + 0.36:
            f.material_index = SLOT['trim']
    # lamps/grilles override whatever they landed on
    for (key, s), (poly, flt, rid) in outl.items():
        for f in P.faces_in_outline(bm, poly, 'xz', flt):
            f[reg] = rid

    # ---- split into region objects
    regions = sorted({f[reg] for f in bm.faces})
    shell_mats = list(me.materials)
    objs = {}
    for rid in regions:
        b2 = bm.copy()
        rl = b2.faces.layers.int.get('reg')
        bmesh.ops.delete(b2, geom=[f for f in b2.faces if f[rl] != rid], context='FACES')
        me2 = bpy.data.meshes.new(f'R{rid}')
        b2.to_mesh(me2)
        b2.free()
        for m in shell_mats:
            me2.materials.append(m)
        ob = bpy.data.objects.new(f'R{rid}', me2)
        P.link(ob)
        objs[rid] = ob
    bm.free()
    # release the source shell so the region objects can take the contract names
    bpy.data.objects.remove(shell, do_unlink=True)
    bpy.data.meshes.remove(me)

    parts = {}
    # body + panels
    names = {REG_BODY: 'Body', REG_HOOD: 'HoodSkin', REG_HATCH: 'TrunkSkin'}
    for s in ('FL', 'FR', 'RL', 'RR'):
        names[REG_DOOR[s]] = f'Panel_{s}_Body'
    for rid, nm in names.items():
        if rid in objs:
            ob = objs[rid]
            ob.name = ob.data.name = nm
            if D['gap'] > 0:
                P.rolled_edges(ob, gap=D['gap'], flange=D['flange'], flange_mat_index=SLOT['trim'])
            parts[nm] = ob
    if D['skin'] > 0:
        inner_skin(spec, cfg, M, g)
    if lod < 2:
        cabin_liner(spec, cfg, M)
    # glass
    gnames = {REG_WS: 'Windscreen', REG_RW: 'RearWindow', REG_QTR['L']: 'QuarterGlassL', REG_QTR['R']: 'QuarterGlassR'}
    for s in ('FL', 'FR', 'RL', 'RR'):
        gnames[REG_WIN[s]] = f'Window_{s}'
    for rid, nm in gnames.items():
        if rid in objs:
            ob = objs[rid]
            ob.name = ob.data.name = nm
            for p_ in ob.data.polygons:
                p_.material_index = SLOT['glass']
            P.offset_along_normals(ob, -0.006)
            if D['gap'] > 0:
                P.rolled_edges(ob, gap=0.004, flange=0.004)
            if cfg.get('dloTrim') and nm.startswith(('Window_', 'QuarterGlass')) and lod < 1:
                dlo_trim(ob, nm.replace('Window_', 'WindowTrim_').replace('QuarterGlass', 'QuarterTrim'),
                         M['chrome'] if cfg['dloTrim'] == 'chrome' else M['gloss'])
            # One physical 4 mm pane. Stacking tint layers made the utility
            # cabin effectively opaque and broke the 1:1 interior read.
            solid_pane(ob, layers=1, gap=0.004)
            ob['glassThicknessM'] = 0.004
            parts[nm] = ob
    # lamps, grille
    for (key, s), (poly, flt, rid) in outl.items():
        if rid not in objs:
            continue
        ob = objs[rid]
        tag = {'head': 'Headlamp', 'tail': 'Taillamp', 'fog': 'FogLamp', 'grille': 'Grille', 'intake': 'Intake'}[key] + s
        lamp_assembly(ob, key, tag, M, D, cfg, s)
        parts[tag] = ob

    live = {o.as_pointer() for o in bpy.data.objects}
    for nm, ob in list(parts.items()):
        try:
            alive = ob is not None and ob.as_pointer() in live and ob.type == 'MESH'
        except ReferenceError:
            alive = False
        if not alive:
            parts.pop(nm, None)
        elif nm == 'TrunkSkin' and cfg.get('utilityTwoTone'):
            for poly in ob.data.polygons:
                poly.use_smooth = False
        elif nm == 'Body' and cfg.get('utilityTwoTone'):
            for poly in ob.data.polygons:
                poly.use_smooth = not (
                    poly.center.y < spec.y0 + 0.16 and poly.normal.y < -0.6
                )
        elif not nm.startswith(('Headlamp', 'Taillamp')):
            transfer_normals(ob, pristine)
    for (key, s), _ in outl.items():
        for ob in bpy.data.objects:
            if ob.type == 'MESH' and ob.name.startswith({'head': 'Headlamp', 'tail': 'Taillamp', 'fog': 'FogLamp'}.get(key, '~')):
                transfer_normals(ob, pristine)
    # a near-planar pane (a cab-over's upright screen sits on the loft's nose cap,
    # whose fan bends the transferred normals into a star) keeps its own normals
    for gn in ('Windscreen', 'RearWindow'):
        ob = bpy.data.objects.get(gn)
        if ob is None or ob.type != 'MESH' or not ob.data.polygons:
            continue
        ref = max(ob.data.polygons, key=lambda p_: p_.area).normal.copy()
        # layered panes face both ways (compare directions, not orientations) and
        # carry thin rim faces between the layers (judge by area, not by face)
        total = sum(p_.area for p_ in ob.data.polygons)
        flat = sum(p_.area for p_ in ob.data.polygons if abs(p_.normal.dot(ref)) > math.cos(math.radians(15)))
        if flat > 0.95 * total:
            if ob.data.has_custom_normals:
                bpy.context.view_layer.objects.active = ob
                ob.select_set(True)
                bpy.ops.mesh.customdata_custom_splitnormals_clear()
                ob.select_set(False)
            for p_ in ob.data.polygons:
                p_.use_smooth = True  # its own (gently curved) geometry's normals
    bpy.data.objects.remove(pristine, do_unlink=True)
    wheels(cfg, M, D)
    utility_cabin(spec, cfg, M, lod)
    decor(cfg, M, D)
    details(cfg, spec, M, D, KR, parts)
    rig(cfg, spec, parts)
    # decor laid on a lid (boot-lid lamp segments, a chrome strip) opens with it
    for ob in list(bpy.data.objects):
        lid = ob.get('_lid')
        if lid is not None:
            pivot = bpy.data.objects.get(lid)
            if pivot is not None:
                P.parent_keep(ob, pivot)
            del ob['_lid']
    return parts


def centroid(f):
    return f.calc_center_median()


def dlo_trim(glass, name, material, width=0.014, proud=0.005):
    """A bright (or gloss-black) surround following the glass opening: a strip from
    the pane's boundary outward along the surface, standing just proud of the paint."""
    bm = bmesh.new()
    bm.from_mesh(glass.data)
    bm.normal_update()
    out = bmesh.new()
    for e in [e for e in bm.edges if e.is_boundary]:
        f = e.link_faces[0]
        mid = (e.verts[0].co + e.verts[1].co) / 2
        d = mid - f.calc_center_median()
        n = f.normal
        d = (d - n * d.dot(n)).normalized()
        q = [out.verts.new(e.verts[0].co + n * (proud + 0.006)), out.verts.new(e.verts[1].co + n * (proud + 0.006)),
             out.verts.new(e.verts[1].co + d * width + n * proud), out.verts.new(e.verts[0].co + d * width + n * proud)]
        f2 = out.faces.new(q)
        f2.normal_update()
        if f2.normal.dot(n) < 0:
            f2.normal_flip()
    bm.free()
    bmesh.ops.remove_doubles(out, verts=out.verts, dist=1e-5)
    return P.obj_from_bm(name, out, material, smooth=True)


def solid_pane(ob, layers=1, gap=0.004):
    """Outer surface faces out; `layers` inward-facing surfaces sit behind it (the
    pane's inner face, then tint-film layers). From outside a pane reads clear over
    the cabin; seen from inside, across the car, side glass reads tinted."""
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bm.normal_update()
    src_faces = list(bm.faces)
    base = [(v, v.co.copy(), v.normal.copy()) for v in bm.verts]
    for k in range(layers):
        d = gap * (k + 1)
        vmap = {v: bm.verts.new(co - n * d) for v, co, n in base}
        for f in src_faces:
            nf = bm.faces.new([vmap[v] for v in f.verts])
            nf.material_index = f.material_index
            nf.smooth = f.smooth
            nf.normal_flip()
    bm.to_mesh(ob.data)
    bm.free()


def transfer_normals(ob, src):
    """Custom split normals interpolated from the uncut shell: cut lines, slivers and
    rolled edges then shade exactly like the continuous surface they came from."""
    for p_ in ob.data.polygons:
        p_.use_smooth = True
    mod = ob.modifiers.new('ShellNormals', 'DATA_TRANSFER')
    mod.object = src
    mod.use_loop_data = True
    mod.data_types_loops = {'CUSTOM_NORMAL'}
    mod.loop_mapping = 'POLYINTERP_NEAREST'
    P.apply_mods(ob)


def cabin_liner(spec, cfg, M):
    """Headliner and (on a saloon) a parcel shelf: the cabin surfaces a viewer sees
    through the side glass. Built from the coarse shell's roof band, 45 mm inward."""
    ob, KR, _ = V.build_shell(spec, 'InteriorHeadlinerV3', M['interior'], lod=2, step=0.06)
    P.link(ob)
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    ring = bm.verts.layers.int.get('ring')
    K6 = KR[6]
    y0 = cfg['cowl'] + (0.70 if cfg.get('utilityTwoTone') else 0.12)
    y1 = (cfg['hatchBase'] if cfg.get('boot') == 'trunk' else cfg['roofRear'] - 0.03)
    bmesh.ops.delete(bm, geom=[f for f in bm.faces if not (min(v[ring] for v in f.verts) >= K6 - 1 and y0 < f.calc_center_median().y < y1)], context='FACES')
    bm.normal_update()
    for v in bm.verts:
        v.co -= v.normal * 0.045
    bmesh.ops.reverse_faces(bm, faces=bm.faces)
    bm.to_mesh(ob.data)
    bm.free()
    if cfg.get('boot') == 'tailgate' and 'rear' in cfg['doors']:
        # crew cab: the rear bench back stands against the cab back wall
        c = spec.c
        yb = cfg['doors']['rear'][1] - 0.18
        zt = c['belt'](yb) + 0.22
        P.box('RearBenchBackV3', (c['beltW'](yb) * 1.75, 0.10, zt - 0.55), (0, yb, (zt + 0.55) / 2), M['interior'])
    if cfg.get('boot') == 'trunk':
        c = spec.c
        yb = cfg['roofRear'] + 0.15
        z = c['belt'](yb) + 0.01
        P.box('ParcelShelfV3', (c['beltW'](yb) * 1.8, cfg['hatchBase'] - yb, 0.02), (0, (yb + cfg['hatchBase']) / 2, z), M['interior'])
        P.box('RearBulkheadV3', (c['beltW'](yb) * 1.75, 0.03, z - 0.35), (0, yb + 0.02, (z + 0.35) / 2 + 0.17), M['interior'])
    return ob


def inner_skin(spec, cfg, M, g):
    """A coarse, closed copy of the shell pushed 30 mm inward with every glass
    opening removed: whatever a shut line or panel gap reveals is dark body
    structure, never daylight, while the cabin stays visible through the glass."""
    ob, KR, _ = V.build_shell(spec, 'InnerSkin', M['trim'], lod=2, step=0.06)
    P.link(ob)
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    ring = bm.verts.layers.int.get('ring')
    K5, K6 = KR[5], KR[6]
    kill = []
    door_bottom = float(cfg.get('doors', {}).get('bottomZ', 0.35))
    door_spans = [tuple(span) for span in (
        cfg.get('doors', {}).get('front'),
        cfg.get('doors', {}).get('rear'),
    ) if span]
    hatch_max = max((p_[1] for p_ in cfg.get('hatchOutline', [])), default=0.0)
    for f in bm.faces:
        rs = [v[ring] for v in f.verts]
        c = f.calc_center_median()
        top = min(rs) >= K6
        band = min(rs) >= K5 and max(rs) <= K6
        side = abs(c.x) > 0.45 or abs(f.normal.x) > 0.25
        in_door = side and c.z > door_bottom + 0.035 and any(
            a_ < c.y < b_ and c.z < spec.c['belt'](c.y) - 0.02
            for a_, b_ in door_spans
        )
        in_rear_hatch = (
            cfg.get('boot') == 'reardoor'
            and c.y > spec.y1 - 0.52
            and cfg.get('hatchBottomZ', 0.7) < c.z < hatch_max + 0.06
            and abs(c.x) < spec.c['halfW'](c.y) * 0.96
        )
        if top and (cfg['cowl'] < c.y < cfg['roofFront'] or cfg['roofRear'] + 0.03 < c.y < cfg['hatchBase']):
            kill.append(f)
        elif band and g['front'] - 0.02 < c.y < g['qRear'] + 0.02:
            kill.append(f)
        elif in_door or in_rear_hatch:
            kill.append(f)
    bmesh.ops.delete(bm, geom=kill, context='FACES')
    bm.normal_update()
    for v in bm.verts:
        v.co -= v.normal * 0.03
    bmesh.ops.reverse_faces(bm, faces=bm.faces)
    bm.to_mesh(ob.data)
    bm.free()
    return ob


def utility_cabin(spec, cfg, M, lod=0):
    """Build a readable floor, cargo bay, and rear cabin separator."""
    if not cfg.get('utilityInterior') or lod > 1:
        return
    c = spec.c
    y0 = float(cfg.get('cargoFloorY0', 1.02))
    y1 = float(cfg.get('cargoFloorY1', 2.14))
    floor_z = float(cfg.get('cargoFloorZ', 0.43))
    width = float(cfg.get('cargoWidth', 1.58))
    P.box('CargoFloor', (width, y1 - y0, 0.06),
          (0, (y0 + y1) / 2, floor_z), M['interior'])
    P.box('CargoBulkhead', (width - 0.04, 0.07, 0.84),
          (0, y0 + 0.01, floor_z + 0.43), M['interior'])
    for sx, side in ((-1, 'L'), (1, 'R')):
        P.box(f'CargoSideTrim{side}', (0.05, y1 - y0 - 0.04, 0.72),
              (sx * (width / 2 - 0.03), (y0 + y1) / 2, floor_z + 0.42), M['interior'])
        P.box(f'CargoWheelHouse{side}', (0.50, 1.20, 0.70),
              (sx * (width / 2 - 0.25), cfg['rearAxle'], floor_z + 0.32),
              M['interior'], bevel=0.14 if lod == 0 else 0)
    P.box('CargoThreshold', (width - 0.06, 0.10, 0.08),
          (0, spec.y1 - 0.16, floor_z + 0.06), M['trim'])
    P.box('CargoCeiling', (width - 0.12, y1 - y0 - 0.10, 0.05),
          (0, (y0 + y1) / 2, c['top'](0.7) - 0.055), M['interior'])


# ---------------------------------------------------------------- lamps
def lamp_assembly(cover, key, tag, M, D, cfg, s):
    """Turn a cut region into: a housing tub behind it, emitter parts, and a clear
    (or smoked) cover in the original surface."""
    depth = {'head': 0.06, 'tail': 0.045, 'fog': 0.035, 'grille': 0.07, 'intake': 0.09}[key]
    if key in ('grille', 'intake', 'fog', 'head'):
        P.tub(cover, tag + '_Housing', depth, M['trim'], floor_scale=0.94)
    bb = [cover.matrix_world @ Vector(c) for c in cover.bound_box]
    lo = Vector((min(v.x for v in bb), min(v.y for v in bb), min(v.z for v in bb)))
    hi = Vector((max(v.x for v in bb), max(v.y for v in bb), max(v.z for v in bb)))
    front = key != 'tail'
    face_y = lo.y if front else hi.y
    back = (1 if front else -1)
    sx = 1 if s == 'R' else -1
    exposed = bool(cfg.get('exposedLampInserts'))
    if key == 'head':
        cover.name = cover.data.name = tag
        cover.data.materials.clear()
        cover.data.materials.append(M['headcover'])
        poly = lamp_poly(cfg, key, s)
        band = lambda a_, b_: P.clip_poly(poly, 1, lo.z + (hi.z - lo.z) * a_, lo.z + (hi.z - lo.z) * b_)
        if not D['lampStep']:
            return
        ls = D['lampStep']
        P.projected_patch(tag + '_Reflector', SHELL_BVH, poly, M['reflector'], -0.026, 1, step=max(ls, 0.012))
        P.projected_patch(tag + '_DRL0', SHELL_BVH, band(0.70, 0.94), M['drl'],
                          0.008 if exposed else -0.008, 1, step=ls)
        upper = cfg['lamps'].get('headDRL')
        if upper and exposed:
            x0 = min(p[0] for p in upper)
            x1 = max(p[0] for p in upper)
            z0 = min(p[1] for p in upper)
            z1 = max(p[1] for p in upper)
            P.box(tag + '_DRLBezel', (x1 - x0 + 0.026, 0.018, z1 - z0 + 0.024),
                  (sx * (x0 + x1) / 2, cfg['plates']['front'][0] + 0.022, (z0 + z1) / 2), M['gloss'])
            P.box(tag + '_DRLUpper', (x1 - x0, 0.018, z1 - z0),
                  (sx * (x0 + x1) / 2, cfg['plates']['front'][0] + 0.015, (z0 + z1) / 2), M['drl'])
        elif upper:
            upper = [tuple(p) for p in upper]
            if s == 'L':
                upper = mirror_outline(upper)
            P.projected_patch(tag + '_DRLUpper', SHELL_BVH, upper, M['drl'],
                              0.010 if exposed else -0.006, 1, step=max(ls, 0.008))
        ind = P.clip_poly(band(0.06, 0.30), 0, *((lo.x + (hi.x - lo.x) * 0.70, hi.x) if sx > 0 else (lo.x, lo.x + (hi.x - lo.x) * 0.30)))
        P.projected_patch(tag + '_Ind', SHELL_BVH, ind, M['amber'],
                          0.012 if exposed else -0.010, 1, step=ls)
        w = hi.x - lo.x
        zc = lo.z + (hi.z - lo.z) * 0.45
        if exposed:
            P.box(tag + '_Projector', (min(0.13, w * 0.66), 0.018, 0.065),
                  ((lo.x + hi.x) / 2, face_y - 0.013, zc), M['lens'], bevel=0.005)
        else:
            for k, fx in enumerate((0.34, 0.60)):
                x = lo.x + w * (fx if sx > 0 else 1 - fx)
                yy = surface_y(cover, x, zc, front) + back * 0.024
                r = min(0.024, (hi.z - lo.z) * 0.26)
                P.cyl(f'{tag}_Dish{k}', r * 1.3, 0.016, (x, yy + back * 0.008, zc), M['chrome'], axis='Y', segs=24, r2=r * 0.55)
                P.cyl(f'{tag}_Lens{k}', r, 0.012, (x, yy, zc), M['lens'], axis='Y', segs=24)
    elif key == 'tail':
        cover.name = cover.data.name = tag
        cover.data.materials.clear()
        # The far tier keeps the physical tail cover but omits the detailed
        # insert geometry. Bind that existing cover to the canonical tail
        # material so every LOD retains a runtime-addressable brake/tail lens.
        cover.data.materials.append(M['tail'] if not D['lampStep'] else M['tailSmoke'])
        poly = lamp_poly(cfg, key, s)
        band = lambda a_, b_: P.clip_poly(poly, 1, lo.z + (hi.z - lo.z) * a_, lo.z + (hi.z - lo.z) * b_)
        xs = lambda a_, b_, pp: P.clip_poly(pp, 0, *((lo.x + (hi.x - lo.x) * a_, lo.x + (hi.x - lo.x) * b_) if sx > 0 else (lo.x + (hi.x - lo.x) * (1 - b_), lo.x + (hi.x - lo.x) * (1 - a_))))
        if not D['lampStep']:
            return
        ls = D['lampStep']
        layout = cfg.get('tailStyle', {}).get('layout', 'bands')
        if layout == 'vertical' and exposed:
            x = (lo.x + hi.x) / 2
            width = hi.x - lo.x
            y = cfg['plates']['rear'][0] + 0.012
            P.box(tag + '_Housing', (width, 0.014, hi.z - lo.z),
                  (x, y - 0.015, (lo.z + hi.z) / 2), M['trim'])
            P.box(tag + '_Spine', (width - 0.012, 0.014, hi.z - lo.z - 0.012),
                  (x, y - 0.002, (lo.z + hi.z) / 2), M['tail'])
            for name, z0, z1, material in (
                ('Bar0', 1.35, 1.61, M['tail']),
                ('Bar1', 1.04, 1.29, M['tail']),
                ('Rev', 0.78, 0.97, M['reverse']),
            ):
                P.box(tag + '_' + name, (width - 0.012, 0.018, z1 - z0),
                      (x, y, (z0 + z1) / 2), material)
        else:
            P.projected_patch(tag + '_Housing', SHELL_BVH, poly, M['trim'], -0.030, -1, step=max(ls, 0.012))
        if layout == 'vertical' and not exposed:
            # a tall corner lamp: stacked stop/tail blocks over a reverse lamp
            P.projected_patch(tag + '_Bar0', SHELL_BVH, band(0.66, 0.96), M['tail'],
                              0.010 if exposed else -0.012, -1, step=ls)
            P.projected_patch(tag + '_Bar1', SHELL_BVH, band(0.36, 0.60), M['tail'],
                              0.010 if exposed else -0.016, -1, step=ls)
            P.projected_patch(tag + '_Rev', SHELL_BVH, band(0.05, 0.30), M['reverse'],
                              0.010 if exposed else -0.016, -1, step=ls)
        elif layout == 'ring':
            # an LED outline around a dark lens (a light bar)
            w_ = cfg['tailStyle'].get('ringW', 0.018)
            area = sum(poly[i][0] * poly[(i + 1) % len(poly)][1] - poly[(i + 1) % len(poly)][0] * poly[i][1] for i in range(len(poly)))
            sg = 1 if area > 0 else -1
            for i in range(len(poly)):
                (ax, az), (bx, bz) = poly[i], poly[(i + 1) % len(poly)]
                L_ = math.hypot(bx - ax, bz - az) or 1
                nx, nz = -sg * (bz - az) / L_ * w_, sg * (bx - ax) / L_ * w_  # inward
                quad = [(ax, az), (bx, bz), (bx + nx, bz + nz), (ax + nx, az + nz)]
                P.projected_patch(f'{tag}_Ring{i}', SHELL_BVH, quad, M['tail'], -0.010, -1, step=max(ls, 0.006))
            P.projected_patch(tag + '_Rev', SHELL_BVH, xs(0.40, 0.62, band(0.30, 0.70)), M['reverse'], -0.016, -1, step=ls)
        elif layout != 'vertical':
            P.projected_patch(tag + '_Bar0', SHELL_BVH, band(0.36, 0.80), M['tail'], -0.014, -1, step=ls)
            P.projected_patch(tag + '_Strip0', SHELL_BVH, band(0.86, 0.97), M['tail'], -0.008, -1, step=ls)
            P.projected_patch(tag + '_Rev', SHELL_BVH, xs(0.05, 0.32, band(0.08, 0.30)), M['reverse'], -0.016, -1, step=ls)
            P.projected_patch(tag + '_Ind', SHELL_BVH, xs(0.36, 0.62, band(0.08, 0.30)), M['amber'], -0.016, -1, step=ls)
    elif key == 'fog':
        cover.data.materials.clear()
        cover.data.materials.append(M['trim'])
        cover.name = cover.data.name = tag + '_Bezel'
        P.offset_along_normals(cover, -0.01)
        if exposed and D['lampStep']:
            P.box(tag + '_Lens', ((hi.x - lo.x) * 0.48, 0.018, (hi.z - lo.z) * 0.35),
                  ((lo.x + hi.x) / 2, face_y - 0.012, (lo.z + hi.z) / 2), M['lens'], bevel=0.004)
    else:  # grille / intake: a honeycomb-like mesh of slats inside the opening
        cover.data.materials.clear()
        cover.data.materials.append(M['gloss'])
        cover.name = cover.data.name = tag + '_Surround'
        bpy.data.objects.remove(cover, do_unlink=True)
        style = cfg.get(key + 'Style', {})
        if D['honey'] > 0 and style.get('insert', 'honey') == 'honey':
            honeycomb(tag + '_Mesh', poly_of(key, cfg), lo, hi, face_y + (0.03 if key == 'grille' else 0.045),
                      (0.022 if key == 'grille' else 0.030) * D['honey'], M['gloss'] if key == 'grille' else M['trim'])
        if style and D['lampStep']:
            grille_dress(tag, poly_of(key, cfg), style, M, D)


def grille_dress(tag, poly, style, M, D):
    """Bars, frame and crossbars laid in the original surface over a grille or
    intake opening (SHELL_BVH is the uncut shell, so offset 0 is the skin)."""
    ls = D['lampStep']
    mat = lambda k: M[{'paint': 'paint', 'chrome': 'chrome', 'gloss': 'gloss', 'trim': 'trim', 'alloy': 'alloy'}[k]]
    zs = [p[1] for p in poly]
    z0, z1 = min(zs), max(zs)
    n = style.get('bars', 0)
    if n:
        fill = style.get('barFill', 0.42)
        pitch = (z1 - z0) / n
        for k in range(n):
            za = z0 + pitch * (k + (1 - fill) / 2)
            band = P.clip_poly(poly, 1, za, za + pitch * fill)
            P.projected_patch(f'{tag}_Bar{k}', SHELL_BVH, band, mat(style.get('barMat', 'gloss')),
                              style.get('barInset', -0.012), 1, step=max(ls, 0.008))
    for k, c in enumerate(style.get('cross', [])):
        band = P.clip_poly(P.clip_poly(poly, 1, c['z'][0], c['z'][1]), 0, *c.get('x', (-9, 9)))
        P.projected_patch(f'{tag}_Cross{k}', SHELL_BVH, band, mat(c.get('mat', 'paint')), c.get('inset', -0.002), 1, step=max(ls, 0.008))
    if style.get('frame'):
        w = style.get('frameW', 0.02)
        area = sum(poly[i][0] * poly[(i + 1) % len(poly)][1] - poly[(i + 1) % len(poly)][0] * poly[i][1] for i in range(len(poly)))
        sgn = 1 if area > 0 else -1  # outward normal of a CCW polygon is (dy, -dx)
        for i in range(len(poly)):
            (ax, az), (bx, bz) = poly[i], poly[(i + 1) % len(poly)]
            L = math.hypot(bx - ax, bz - az) or 1
            nx, nz = sgn * (bz - az) / L * w, -sgn * (bx - ax) / L * w
            quad = [(ax - nx * 0.25, az - nz * 0.25), (bx - nx * 0.25, bz - nz * 0.25), (bx + nx, bz + nz), (ax + nx, az + nz)]
            P.projected_patch(f'{tag}_Frame{i}', SHELL_BVH, quad, mat(style['frame']), 0.0015, 1, step=max(ls, 0.006))


def decor(cfg, M, D):
    """Free front/rear-view features projected onto the skin: chrome bars that run
    into the lamps, bumper blades, skid plates, badges-free brand plates."""
    if not D['lampStep'] or SHELL_BVH is None:
        return
    mats = {'paint': M['paint'], 'chrome': M['chrome'], 'gloss': M['gloss'], 'trim': M['trim'], 'alloy': M['alloy'],
            'drl': M['drl'], 'tail': M['tail'], 'reflector': M['reflector']}
    if 'cladding' in M:
        mats['cladding'] = M['cladding']
    for side, key, direction in (('Front', 'frontDecor', 1), ('Rear', 'rearDecor', -1)):
        for k, d in enumerate(cfg.get(key, [])):
            right = [tuple(p) for p in d['poly']]
            polys = [('R', right)] + ([('L', mirror_outline(right))] if d.get('mirror') else [])
            for s_, poly in polys:
                ob = P.projected_patch(f'{side}Decor{k}{s_}', SHELL_BVH, poly, mats[d.get('mat', 'chrome')],
                                       d.get('offset', 0.002), direction, step=max(D['lampStep'], d.get('step', 0.008)))
                if ob is not None:
                    # a ray that missed the end face runs on to the flank or the
                    # underbody: keep only faces within reach of this end
                    me_ = ob.data
                    end = cfg['L'] / 2
                    far = [p_.index for p_ in me_.polygons
                           if abs((ob.matrix_world @ p_.center).y * direction + end) > d.get('reach', 0.35)]
                    if far:
                        bm_ = bmesh.new(); bm_.from_mesh(me_); bm_.faces.ensure_lookup_table()
                        bmesh.ops.delete(bm_, geom=[bm_.faces[i] for i in far], context='FACES')
                        bm_.to_mesh(me_); bm_.free()
                    if not me_.polygons:
                        bpy.data.objects.remove(ob, do_unlink=True)
                        continue
                if ob is not None and d.get('on'):
                    ob['_lid'] = {'trunk': 'Trunk', 'hood': 'Hood'}[d['on']]


SHELL_BVH = None


def lamp_poly(cfg, key, s):
    pts = [tuple(p) for p in cfg['lamps'][key]]
    return pts if s != 'L' else mirror_outline(pts)


def poly_of(key, cfg):
    return [tuple(p) for p in cfg['lamps'][key]]


def honeycomb(name, poly, lo, hi, y, cell, material):
    """Hexagonal mesh insert clipped to a front-view outline, facing -Y."""
    bm = bmesh.new()
    rw = cell * 0.30  # rib half-width
    dx = cell * 1.5
    dz = cell * math.sqrt(3) / 2
    i = 0
    x = lo.x
    while x <= hi.x:
        z = lo.z + (dz if i % 2 else 0)
        while z <= hi.z:
            if P.point_in_poly((x, z), poly):
                outer = [bm.verts.new((x + cell * math.cos(a), y, z + cell * math.sin(a))) for a in [k * math.pi / 3 for k in range(6)]]
                inner = [bm.verts.new((x + (cell - rw) * math.cos(a), y, z + (cell - rw) * math.sin(a))) for a in [k * math.pi / 3 for k in range(6)]]
                back_o = [bm.verts.new((v.co.x, y + 0.02, v.co.z)) for v in outer]
                for k in range(6):
                    k2 = (k + 1) % 6
                    bm.faces.new((outer[k], outer[k2], inner[k2], inner[k]))
                    bm.faces.new((inner[k], inner[k2], back_o[k2] if False else bm.verts.new((inner[k2].co.x, y + 0.02, inner[k2].co.z)), bm.verts.new((inner[k].co.x, y + 0.02, inner[k].co.z))))
            z += 2 * dz
        x += dx
        i += 1
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return P.obj_from_bm(name, bm, material, smooth=False)


def layer(src, name, offset, material, band=None, xband=None):
    """Copy of a lamp patch pushed `offset` along its normals, optionally limited to a
    normalised height band / width band of the patch."""
    ob = src.copy()
    ob.data = src.data.copy()
    ob.name = ob.data.name = name
    P.link(ob)
    ob.data.materials.clear()
    ob.data.materials.append(material)
    bm = bmesh.new()
    bm.from_mesh(ob.data)
    bm.normal_update()
    zs = [v.co.z for v in bm.verts]
    xs = [v.co.x for v in bm.verts]
    z0, z1, x0, x1 = min(zs), max(zs), min(xs), max(xs)
    kill = []
    for f in bm.faces:
        c = f.calc_center_median()
        tz = (c.z - z0) / max(1e-6, z1 - z0)
        tx = (c.x - x0) / max(1e-6, x1 - x0)
        if band and not (band[0] <= tz <= band[1]):
            kill.append(f)
        elif xband and not (xband[0] <= tx <= xband[1]):
            kill.append(f)
    bmesh.ops.delete(bm, geom=kill, context='FACES')
    for v in bm.verts:
        v.co += v.normal * offset
    for f in bm.faces:
        f.material_index = 0
    bm.to_mesh(ob.data)
    bm.free()
    return ob


def surface_y(ob, x, z, front=True):
    best, by = 1e9, 0.0
    for v in ob.data.vertices:
        d = (v.co.x - x) ** 2 + (v.co.z - z) ** 2
        if d < best:
            best, by = d, v.co.y
    return by


# ---------------------------------------------------------------- wheels
WHEEL_STYLE = {'twin5': dict(n=5, offs=(-0.11, 0.11), root=0.22, w=1.0),
               'multi10': dict(n=10, offs=(0.0,), root=0.18, w=0.62),
               'spoke5': dict(n=5, offs=(0.0,), root=0.20, w=1.35),
               'utility5': dict(n=5, offs=(0.0,), root=0.20, w=2.3),
               'steel': dict(n=0, offs=(), root=0.0, w=1.0)}


def wheels(cfg, M, D):
    R = cfg['R']
    WT = cfg.get('tyreWidth', 0.195)
    rim_r = R * cfg.get('rimRatio', 0.64)
    for name, x, y in (('FL', -1, cfg['frontAxle']), ('FR', 1, cfg['frontAxle']),
                       ('RL', -1, cfg['rearAxle']), ('RR', 1, cfg['rearAxle'])):
        tr = cfg['track'] if name[0] == 'F' else cfg.get('trackRear', cfg['track'])
        pivot = P.empty(f'Wheel_{name}', (x * tr / 2, y, R))
        spin = P.empty(f'Wheel_{name}_Spin', (0, 0, 0))
        spin.parent = pivot
        Mw = dict(M)
        if cfg.get('wheelFinish') == 'dark':
            Mw['alloy'] = M['darkalloy']
        parts = wheel_parts(name, x, R, WT, rim_r, Mw, D, WHEEL_STYLE[cfg.get('wheelStyle', 'twin5')])
        if cfg.get('dualRear') and name[0] == 'R':
            # the inner tyre of a dual pair: same spin pivot, one tread width inboard
            inner = P.lathe(f'Wheel_{name}_TyreInner', [(rim_r - 0.004, -WT / 2 + 0.012), (R - 0.01, -WT / 2 + 0.01), (R, -WT / 2 + 0.04), (R, WT / 2 - 0.04), (R - 0.01, WT / 2 - 0.01), (rim_r - 0.004, WT / 2 - 0.012)], D['wseg'], M['rubber'])
            inner.location = (-x * (WT + 0.03), 0, 0)
            parts.append(inner)
        for p_ in parts:
            p_.parent = spin


def wheel_parts(name, sx, R, WT, rim_r, M, D, style=WHEEL_STYLE['twin5']):
    seg = D['wseg']
    h = WT / 2
    # tyre: sidewall bulge, rounded shoulder, tread with two circumferential grooves
    prof = [(rim_r - 0.004, -h + 0.012), (rim_r + 0.012, -h), (R - 0.07, -h - 0.006), (R - 0.03, -h + 0.004),
            (R - 0.008, -h + 0.02), (R, -h + 0.04), (R, -0.045), (R - 0.008, -0.04), (R - 0.008, -0.03), (R, -0.025),
            (R, 0.025), (R - 0.008, 0.03), (R - 0.008, 0.04), (R, 0.045), (R, h - 0.04), (R - 0.008, h - 0.02),
            (R - 0.03, h - 0.004), (R - 0.07, h + 0.006), (rim_r + 0.012, h), (rim_r - 0.004, h - 0.012)]
    if D['tyre'] == 'lite':
        prof = [(rim_r - 0.004, -h + 0.012), (rim_r + 0.012, -h), (R - 0.05, -h - 0.004), (R - 0.01, -h + 0.02),
                (R, -h + 0.045), (R, h - 0.045), (R - 0.01, h - 0.02), (R - 0.05, h + 0.004), (rim_r + 0.012, h),
                (rim_r - 0.004, h - 0.012)]
    prof = [(r, a * sx) for r, a in prof]
    out = [P.lathe(f'Wheel_{name}_Tyre', prof, seg, M['rubber'])]
    if not D['spokes']:
        out.append(P.cyl(f'Wheel_{name}_Face', rim_r, 0.02, (sx * (h - 0.03), 0, 0), M['alloy'], axis='X', segs=seg))
        return out
    # rim barrel + outer lip
    face = sx * (h - 0.018)
    lip = [(rim_r - 0.03, face - sx * 0.01), (rim_r + 0.004, face), (rim_r + 0.008, face - sx * 0.012),
           (rim_r - 0.004, -sx * (h - 0.02)), (rim_r - 0.03, -sx * (h - 0.02))]
    out.append(P.lathe(f'Wheel_{name}_Lip', lip, seg, M['alloy']))
    # dished face with a centre bore; spokes join the face to the lip
    dish = [(0.0, face - sx * 0.030), (R * 0.10, face - sx * 0.030), (R * 0.19, face - sx * 0.022), (R * 0.24, face - sx * 0.020), (R * 0.27, face - sx * 0.030)]
    out.append(P.lathe(f'Wheel_{name}_Hub', dish, max(16, seg // 2), M['alloy']))
    out.append(P.cyl(f'Wheel_{name}_Cap', R * 0.085, 0.008, (face - sx * 0.026, 0, 0), M['chrome'], axis='X', segs=20 if D['brakes'] else 10))
    out.append(P.lathe(f'Wheel_{name}_Barrel', [(rim_r - 0.03, face - sx * 0.02), (rim_r - 0.035, -sx * (h - 0.03))], max(16, seg // 2), M['darkalloy']))
    if D['spokes']:
        # five twin spokes, twisted, tapering to the lip
        n = style['n'] if D['spokeCount'] >= 10 else min(5, style['n'])
        offs = style['offs'] if D['spokeCount'] >= 10 else (0.0,)
        if style['n'] == 0:
            out.append(P.cyl(f'Wheel_{name}_Face', rim_r * 0.96, 0.02, (face - sx * 0.02, 0, 0), M['steel'], axis='X', segs=seg))
        for i in range(n):
            for k, off in enumerate(offs):
                a = 2 * math.pi * i / n + off
                sp = spoke(f'SpokeBlade_{name}{i * len(offs) + k}', R * style['root'], rim_r - 0.006, sx, face, M['alloy'], off, style['w'])
                sp.rotation_euler = (a, 0, 0)
                out.append(sp)
        for i in (range(5) if D['brakes'] else ()):
            a = 2 * math.pi * (i + 0.5) / 5
            out.append(P.cyl(f'Lug_{name}{i}', 0.011, 0.018, (face - sx * 0.03, math.sin(a) * R * 0.14, math.cos(a) * R * 0.14), M['chrome'], axis='X', segs=6))
    else:
        out.append(P.cyl(f'Wheel_{name}_Face', rim_r, 0.02, (face - sx * 0.02, 0, 0), M['alloy'], axis='X', segs=seg))
    if D['brakes']:
        out.append(P.lathe(f'Wheel_{name}_Disc', [(R * 0.24, -sx * 0.01), (R * 0.52, -sx * 0.01), (R * 0.52, -sx * 0.035), (R * 0.24, -sx * 0.035)], 32, M['steel']))
        cal = P.box(f'Wheel_{name}_Caliper', (0.05, 0.07, 0.13), (-sx * 0.022, -0.02, R * 0.40), M['caliper'], bevel=0.012)
        out.append(cal)
    return out


def spoke(name, r0, r1, sx, face, material, twist, wscale=1.0):
    """A tapered, slightly twisted spoke blade along +Z in wheel space."""
    bm = bmesh.new()
    n = 6
    rows = []
    for i in range(n + 1):
        t = i / n
        r = r0 + (r1 - r0) * t
        w = (0.036 - 0.014 * t) * wscale
        d = 0.030 - 0.010 * t
        x = face - sx * (0.03 - 0.018 * t) - sx * 0.004 * math.sin(math.pi * t)
        tw = -twist * 0.35 * t
        pts = [(-w / 2, 0), (w / 2, 0), (w / 2 * 0.8, -d), (-w / 2 * 0.8, -d)]
        ring = []
        for u, dd in pts:
            yy = u * math.cos(tw)
            ring.append(bm.verts.new((x + sx * dd, yy, r)))
        rows.append(ring)
    for a, b in zip(rows, rows[1:]):
        for j in range(4):
            bm.faces.new((a[j], a[(j + 1) % 4], b[(j + 1) % 4], b[j]))
    bm.faces.new(rows[0][::-1])
    bm.faces.new(rows[-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return P.obj_from_bm(name, bm, material, smooth=False)


# ---------------------------------------------------------------- details
def shut_strip(name, bvh, line, material, side, width=0.006, step=0.03):
    """A dark 6 mm shut line painted onto a flank along a side-view polyline, for
    panels that do not articulate (sliding and barn doors, hatches on vans)."""
    pts = []
    for (ya, za), (yb, zb) in zip(line, line[1:]):
        n = max(1, int(math.hypot(yb - ya, zb - za) / step))
        for i in range(n + (1 if (yb, zb) == tuple(line[-1]) else 0)):
            t = i / n
            pts.append((ya + (yb - ya) * t, za + (zb - za) * t))
    bm = bmesh.new()
    prev = None
    for i, (y, z) in enumerate(pts):
        hit, nrm, _, _ = bvh.ray_cast(Vector((side * 5.0, y, z)), Vector((-side, 0, 0)))
        if hit is None:
            prev = None
            continue
        j = min(i + 1, len(pts) - 1)
        k = max(i - 1, 0)
        tan = Vector((0, pts[j][0] - pts[k][0], pts[j][1] - pts[k][1])).normalized()
        off = tan.cross(nrm).normalized() * (width / 2)
        cur = (bm.verts.new(hit + nrm * 0.0015 + off), bm.verts.new(hit + nrm * 0.0015 - off))
        if prev:
            f = bm.faces.new((prev[0], prev[1], cur[1], cur[0]))
            f.normal_update()
            if f.normal.x * side < 0:
                f.normal_flip()
        prev = cur
    if not bm.faces:
        bm.free()
        return None
    return P.obj_from_bm(name, bm, material, smooth=False)


def V_surface_x(c, spec, y, z):
    """Half-width of the body section at (y, z), sampled from the section spline."""
    sec = V.section(spec, y, 2)
    best, bx = 1e9, 0.0
    for p in sec:
        if abs(p.y - z) < best:
            best, bx = abs(p.y - z), p.x
    return bx


def details(cfg, spec, M, D, KR, parts):
    c = spec.c
    L = spec.L
    if cfg.get('utilityTwoTone') and SHELL_BVH is not None:
        # The glazing is cut from the shell, but the narrow fixed posts still
        # need their black exterior finish across all three LODs.
        posts = (
            ('A', [(-1.075, 1.29), (-0.17, 1.93)], 0.055),
            ('B', [(0.20, 1.30), (0.20, 1.94)], 0.105),
            ('C', [(1.51, 1.30), (1.54, 1.92)], 0.075),
        )
        for sx, side in ((-1, 'L'), (1, 'R')):
            for label, line, width in posts:
                shut_strip(f'WindowPost{label}{side}', SHELL_BVH, line,
                           M['gloss'], sx, width=width,
                           step=0.07 if D['spans'] == 2 else 0.025)
    # arch liners: black half-shells inside every wheel opening
    for ay, zc, r in (cfg['arches'] if D['liners'] else []):
        for sx in (-1, 1):
            hw = c['halfW'](ay)
            prof = []
            for i in range(19):
                a = math.pi * i / 18
                prof.append((0, 0))
            bm = bmesh.new()
            rows = []
            for i in range(25):
                a = math.radians(-10 + 200 * i / 24)
                yy = ay + (r - 0.01) * math.cos(a)
                zz = zc + (r - 0.01) * math.sin(a)
                if zz < 0.18:
                    continue
                rows.append([bm.verts.new((sx * (hw - 0.02), yy, zz)), bm.verts.new((sx * (hw - 0.33), yy, zz))])
            for a_, b_ in zip(rows, rows[1:]):
                bm.faces.new((a_[0], a_[1], b_[1], b_[0]))
            # inner wall closing the well, so the far side never shows through
            inner = [r_[1] for r_ in rows]
            base = [bm.verts.new((v.co.x, v.co.y, max(0.20, min(v.co.z, c['bottom'](v.co.y) + 0.01)))) for v in inner]
            for i in range(len(inner) - 1):
                bm.faces.new((inner[i], inner[i + 1], base[i + 1], base[i]))
            bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
            P.obj_from_bm(f'ArchLiner{"F" if ay < 0 else "R"}{"L" if sx < 0 else "R"}', bm, M['trim'])
    if cfg.get('cladding'):
        # black wheel-arch flares standing proud of the paint (crossover cladding)
        for ay, zc, r in cfg['arches']:
            for sx in (-1, 1):
                bm = bmesh.new()
                rows = []
                n = 28 if D['bevel'] else 12
                for i in range(n + 1):
                    a = math.radians(4 + 172 * i / n)
                    yy, zz = ay + r * math.cos(a), zc + r * math.sin(a)
                    if zz < max(0.22, c['bottom'](yy) + 0.03):
                        continue
                    x = V_surface_x(c, spec, yy, zz)
                    radial = Vector((0, math.cos(a), math.sin(a)))
                    base = Vector((sx * (x + 0.004), yy, zz))
                    rows.append([bm.verts.new(base), bm.verts.new(base + radial * 0.055 + Vector((sx * 0.012, 0, 0))),
                                 bm.verts.new(base + radial * 0.075 - Vector((sx * 0.004, 0, 0)))])
                for a_, b_ in zip(rows, rows[1:]):
                    for j in range(2):
                        bm.faces.new((a_[j], a_[j + 1], b_[j + 1], b_[j]))
                bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
                P.obj_from_bm(f'ArchCladding{"F" if ay < 0 else "R"}{"L" if sx < 0 else "R"}',
                              bm, M.get('cladding', M['trim']), smooth=True)
    # mirrors on the front doors
    my, mz = cfg.get('mirrorY', cfg['doors']['front'][0] + 0.10), c['belt'](cfg.get('mirrorY', cfg['doors']['front'][0] + 0.10)) + 0.06
    for sx, s in ((-1, 'L'), (1, 'R')):
        bw = c['beltW'](my)
        cap = P.box(f'Mirror{s}', (0.10, 0.20, 0.12), (sx * (bw + 0.12), my + 0.01, mz), M['gloss'] if cfg.get('mirrorFinish') == 'gloss' else M['paint'], bevel=0.04 if D['bevel'] else 0, segs=4 if D['bevel'] else 1, rot=(0, 0, sx * math.radians(-6)))
        glass = P.box(f'MirrorGlass{s}', (0.085, 0.004, 0.10), (sx * (bw + 0.12), my + 0.112, mz), M['chrome'], rot=(0, 0, sx * math.radians(-6)))
        arm = P.box(f'MirrorArm{s}', (0.10, 0.07, 0.05), (sx * (bw + 0.04), my + 0.02, mz - 0.02), M['trim'], bevel=0.012 if D['bevel'] else 0)
        parts[f'Mirror{s}'], parts[f'MirrorGlass{s}'], parts[f'MirrorArm{s}'] = cap, glass, arm
    # door pull handles on the shoulder
    hz = cfg.get('handleZ', 0.86)
    hset = [('FL', cfg['doors']['front'][1] - 0.22), ('FR', cfg['doors']['front'][1] - 0.22)]
    if 'rear' in cfg['doors']:
        hset += [('RL', cfg['doors']['rear'][1] - 0.14), ('RR', cfg['doors']['rear'][1] - 0.14)]
    hset = [h_ for h_ in hset if h_[0][1] in cfg['doors'].get('sides', 'LR')] if not cfg.get('noHandles') else []
    for s, hy in hset if D['handles'] else ():
        sx = -1 if s.endswith('L') else 1
        x = c['halfW'](hy) - 0.002
        parts[f'Handle_{s}'] = P.box(f'Handle_{s}', (0.028, 0.17, 0.034), (sx * (x + 0.006), hy, hz), M['paint'], bevel=0.012 if D['bevel'] else 0, segs=3)
        if D['bevel']:
            parts[f'HandleRecess_{s}'] = P.box(f'HandleRecess_{s}', (0.01, 0.12, 0.05), (sx * (x - 0.001), hy, hz - 0.012), M['trim'], bevel=0.004)
    # wipers
    cowl = cfg['cowl']
    wy = cowl + 0.07
    nose = -cfg['L'] / 2
    if wy < nose + 0.02:
        # cab-over: the cowl sits ahead of an upright windscreen, so the wipers
        # park at the screen's base on the near-vertical face instead
        zb = cfg['windscreenOutline'][0][1] + 0.04
        lo_, hi_ = nose, nose + 0.6
        for _ in range(40):
            mid = (lo_ + hi_) / 2
            lo_, hi_ = (mid, hi_) if c['top'](mid) < zb else (lo_, mid)
        wy = lo_ - 0.012
        wz = zb
    else:
        wz = c['top'](wy) + 0.018
    slope = math.atan2(c['top'](wy + 0.02) - c['top'](wy - 0.02), 0.04)
    for sx, s in ((-1, 'L'), (1, 'R')):
        P.box(f'Wiper{s}', (0.50, 0.014, 0.012), (sx * 0.30 - 0.06, wy, wz), M['trim'], rot=(-slope, 0, math.radians(6 if sx < 0 else 4)))
    # shark-fin antenna (opt-out for low-roof utility designs)
    ay = cfg['roofRear'] - 0.12
    if cfg.get('roofRails'):
        roof_rails(cfg, c, M, D)
    if cfg.get('roofPod'):
        # roof equipment pod and a lit destination sign over the windscreen
        P.box('RoofPod', (c['railW'](0) * 1.5, spec.L * 0.32, 0.20), (0, -spec.L * 0.12, c['top'](0) + 0.09), M['paint'], bevel=0.05 if D['bevel'] else 0, segs=2)
        sign = P.mat('VehicleSign', (0.95, 0.60, 0.10), rough=0.3, emit=(1.0, 0.55, 0.05), emit_str=0.6)
        P.box('DestSign', (c['halfW'](spec.y0 + 0.3) * 1.5, 0.03, 0.26), (0, spec.y0 + 0.07, c['top'](spec.y0 + 0.3) - 0.24), sign)
    cb = cfg.get('cargoBox')
    if cb:
        cargo_box(cb, cfg, M, D)
    if cfg.get('taxiSign') and not cfg.get('taxiAdBoard'):
        ys_ = (cfg['roofFront'] + cfg['roofRear']) / 2
        zt = c['top'](ys_)
        sign = P.mat('VehicleSign', (0.95, 0.93, 0.80), rough=0.3, emit=(1.0, 0.95, 0.75), emit_str=0.3)
        P.box('TaxiSignBase', (0.36, 0.20, 0.03), (0, ys_, zt + 0.012), M['trim'])
        P.box('TaxiSign', (0.56, 0.16, 0.13), (0, ys_, zt + 0.09), sign, bevel=0.02 if D['bevel'] else 0, segs=2)
    for k_, line in enumerate(cfg.get('shutLines', []) if (D['lampStep'] and SHELL_BVH is not None) else []):
        for sx_ in (-1, 1):
            shut_strip(f'ShutLine{"L" if sx_ < 0 else "R"}{k_}', SHELL_BVH, line, M['shut'], sx_)
    if cfg.get('policeKit'):
        # emergency light bar and push bar (node + material names are the v2 runtime contract)
        ys_ = (cfg['roofFront'] + cfg['roofRear']) / 2 - 0.05
        zt = c['top'](ys_)
        red = P.mat('VehicleEmergencyRed', (0.8, 0.02, 0.02), rough=0.2, emit=(1.0, 0.05, 0.05), emit_str=0.4)
        blue = P.mat('VehicleEmergencyBlue', (0.02, 0.08, 0.8), rough=0.2, emit=(0.1, 0.25, 1.0), emit_str=0.4)
        wbar = min(1.24, c['railW'](ys_) * 1.9)
        P.box('EmergencyBarBase', (wbar, 0.26, 0.04), (0, ys_, zt + 0.03), M['trim'], bevel=0.01 if D['bevel'] else 0)
        P.box('EmergencyBarRed', (wbar / 2 - 0.02, 0.22, 0.08), (-wbar / 4, ys_, zt + 0.09), red, bevel=0.02 if D['bevel'] else 0)
        P.box('EmergencyBarBlue', (wbar / 2 - 0.02, 0.22, 0.08), (wbar / 4, ys_, zt + 0.09), blue, bevel=0.02 if D['bevel'] else 0)
        yf = spec.y0 - 0.06
        for sx_ in (-1, 1):
            P.box(f'PushBarUpright{sx_}', (0.05, 0.06, 0.46), (sx_ * 0.32, yf, 0.62), M['trim'], bevel=0.01 if D['bevel'] else 0)
        P.box('PushBarCross', (0.74, 0.06, 0.06), (0, yf, 0.72), M['trim'], bevel=0.01 if D['bevel'] else 0)
    band = cfg.get('liveryBand')
    if band and D['lampStep'] and SHELL_BVH is not None:
        # a livery band along each flank (checker squares or a plain stripe)
        y0b, y1b = band['y']
        zc_, h_ = band['z'], band['h']
        # the overlay shares a prepared material by NAME across every body, so
        # each livery colour needs its own name
        light = P.mat(band.get('lightName', 'VehicleLiveryYellow'), tuple(band.get('light', (0.94, 0.72, 0.12))), rough=0.45)
        dark = P.mat('VehicleLiveryDark', (0.02, 0.02, 0.022), rough=0.5)
        cells = band.get('checker', 0)
        for sx_ in (-1, 1):
            if cells:
                w_ = (y1b - y0b) / cells
                for k_ in range(cells):
                    for r_ in range(2):
                        yy0 = y0b + k_ * w_
                        zz0 = zc_ - h_ / 2 + r_ * h_ / 2
                        mat_ = dark if (k_ + r_) % 2 else light
                        P.side_patch(f'Livery{"L" if sx_ < 0 else "R"}{k_}_{r_}', SHELL_BVH,
                                     [(yy0, zz0), (yy0 + w_, zz0), (yy0 + w_, zz0 + h_ / 2), (yy0, zz0 + h_ / 2)],
                                     mat_, 0.0015, sx_, step=min(0.03, w_ / 2))
            else:
                P.side_patch(f'Livery{"L" if sx_ < 0 else "R"}', SHELL_BVH,
                             [(y0b, zc_ - h_ / 2), (y1b, zc_ - h_ / 2), (y1b, zc_ + h_ / 2), (y0b, zc_ + h_ / 2)],
                             dark if band.get('dark') else light, 0.0015, sx_, step=0.03)
    if cfg.get('taxiAdBoard'):
        ys_ = (cfg['roofFront'] + cfg['roofRear']) / 2
        zt = c['top'](ys_)
        sign = P.mat('VehicleSign', (0.95, 0.93, 0.80), rough=0.3, emit=(1.0, 0.95, 0.75), emit_str=0.3)
        P.box('TaxiSignBase', (0.10, 0.9, 0.04), (0, ys_, zt + 0.015), M['trim'])
        P.box('TaxiSign', (0.12, 1.02, 0.36), (0, ys_, zt + 0.21), sign, bevel=0.015 if D['bevel'] else 0, segs=2)
    if cfg.get('spoiler') and cfg.get('boot') == 'trunk':
        # a lip on the trailing edge of the deck, in body colour
        ys_ = spec.y1 - cfg.get('spoilerInset', 0.10)
        zs_ = c['top'](ys_)
        w_ = c['railW'](ys_) * 1.7
        P.box('DeckSpoiler', (w_, 0.07, 0.022), (0, ys_, zs_ + 0.008), M['paint'], bevel=0.008 if D['bevel'] else 0, rot=(math.radians(-8), 0, 0))
    if not cfg.get('noAntenna'):
        P.box('Antenna', (0.05, 0.18, 0.06), (0, ay, c['top'](ay) + 0.02), M['paint'], bevel=0.02 if D['bevel'] else 0, segs=3)
    # plates
    pf = cfg['plates']
    P.box('PlateFront', (0.52, 0.012, 0.11), (0, pf['front'][0], pf['front'][1]),
          M['trim'] if cfg.get('utilityTwoTone') else M['plate'])
    P.box('PlateRear', (0.72, 0.004, 0.20) if cfg.get('utilityTwoTone') else (0.52, 0.012, 0.11),
          (0, pf['rear'][0] - 0.008 if cfg.get('utilityTwoTone') else pf['rear'][0], pf['rear'][1]),
          M['paint'] if cfg.get('utilityTwoTone') else M['plate'])
    if cfg.get('utilityTwoTone'):
        P.box('FrontFasciaInsert', (1.10, 0.006, 0.17),
              (0, spec.y0 - 0.004, 0.94), M['paint'],
              bevel=0.002 if D['bevel'] else 0)
    # exhaust (hidden under the diffuser on this class)
    ex = cfg.get('exhaust')
    if ex:
        P.cyl('ExhaustR', 0.03, 0.12, (ex[0], ex[1], ex[2]), M['chrome'], axis='Y', segs=16)
    # underbody so the arches and sills never show daylight
    ua, ub = cfg['frontAxle'] - 0.25, cfg['rearAxle'] + 0.25
    P.box('Underbody', (c['halfW'](0) * 1.6, ub - ua, 0.03), (0, (ua + ub) / 2, min(0.20, c['bottom'](0) - 0.02)), M['trim'])


def cargo_box(cb, cfg, M, D):
    """Separate cargo box of a truck: a panelled body on its own floor, with
    corner posts, rub rails and a rear roller door frame."""
    y0, y1, z0, z1, hw = cb['y0'], cb['y1'], cb['z0'], cb['z1'], cb['hw']
    P.box('CargoBox', (2 * hw, y1 - y0, z1 - z0), (0, (y0 + y1) / 2, (z0 + z1) / 2), M['paint'], bevel=0.02 if D['bevel'] else 0, segs=2)
    if D['bevel']:
        for sx in (-1, 1):
            for yy in (y0 + 0.03, y1 - 0.03):
                P.box(f'CargoPost{"L" if sx < 0 else "R"}{int(yy * 10)}', (0.05, 0.07, z1 - z0 + 0.01), (sx * (hw + 0.005), yy, (z0 + z1) / 2), M['steel'])
            for zz in (z0 + 0.12, z0 + (z1 - z0) * 0.5):
                P.box(f'CargoRail{"L" if sx < 0 else "R"}{int(zz * 10)}', (0.03, y1 - y0 - 0.1, 0.05), (sx * (hw + 0.012), (y0 + y1) / 2, zz), M['steel'])
        P.box('CargoRoofCap', (2 * hw + 0.02, y1 - y0 + 0.02, 0.05), (0, (y0 + y1) / 2, z1), M['steel'])
        # riveted panel joints every ~0.6 m and amber side markers along the top rail
        n_ = max(2, int((y1 - y0) / 0.6))
        for sx in (-1, 1):
            for k in range(1, n_):
                yy = y0 + (y1 - y0) * k / n_
                P.box(f'CargoSeam{"L" if sx < 0 else "R"}{k}', (0.006, 0.022, z1 - z0 - 0.06), (sx * (hw + 0.003), yy, (z0 + z1) / 2), M['steel'])
            for k in range(3):
                yy = y0 + 0.3 + k * (y1 - y0 - 0.6) / 2
                P.box(f'CargoMarker{"L" if sx < 0 else "R"}{k}', (0.02, 0.07, 0.035), (sx * (hw + 0.01), yy, z1 - 0.10), M['amber'])
        P.box('RollerDoorFrame', (2 * hw - 0.08, 0.02, z1 - z0 - 0.10), (0, y1 + 0.006, (z0 + z1) / 2), M['trim'])
        P.box('RollerDoor', (2 * hw - 0.20, 0.02, z1 - z0 - 0.22), (0, y1 + 0.012, (z0 + z1) / 2 + 0.02), M['paint'])
        for k in range(8):
            zz = z0 + 0.2 + k * (z1 - z0 - 0.35) / 8
            P.box(f'RollerSlat{k}', (2 * hw - 0.22, 0.006, 0.012), (0, y1 + 0.024, zz), M['shut'])
    # chassis: frame rails, rear bumper with lamps, fuel tank, side guards
    fr = cb.get('frameZ', 0.62)
    for sx in (-1, 1):
        ya_, yb_ = cfg['frontAxle'] + 0.2, y1 - 0.16
        P.box(f'FrameRail{"L" if sx < 0 else "R"}', (0.08, yb_ - ya_, 0.22), (sx * 0.42, (ya_ + yb_) / 2, fr), M['trim'])
    P.box('RearBumper', (2 * hw - 0.2, 0.10, 0.12), (0, y1 - 0.12, fr - 0.12), M['trim'])
    for sx in (-1, 1):
        P.box(f'Taillamp{"L" if sx < 0 else "R"}_Bar0', (0.26, 0.03, 0.10), (sx * (hw - 0.30), y1 - 0.06, fr - 0.12), P.mat('VehicleTaillamp', (0.42, 0.015, 0.012), rough=0.06, emit=(1.0, 0.05, 0.02), emit_str=0.2))
    P.box('FuelTank', (0.30, 0.75, 0.34), (-(hw - 0.55), (cfg['frontAxle'] + cfg['rearAxle']) / 2 - 0.4, fr - 0.1), M['steel'], bevel=0.04 if D['bevel'] else 0)
    if D['bevel']:
        for sx in (-1, 1):
            P.box(f'SideGuard{"L" if sx < 0 else "R"}', (0.03, cfg['rearAxle'] - cfg['frontAxle'] - 1.4, 0.10), (sx * (hw - 0.08), (cfg['frontAxle'] + cfg['rearAxle']) / 2 + 0.2, fr - 0.15), M['trim'])


def roof_rails(cfg, c, M, D):
    """Two longitudinal rails on low feet, following the roof line."""
    y0, y1 = cfg['roofFront'] + 0.12, cfg['roofRear'] - 0.02
    top_inset = cfg.get('roofRailTopInsetM')
    n = 24 if D['bevel'] else 8
    for sx in (-1, 1):
        bm = bmesh.new()
        rows = []
        for i in range(n + 1):
            y = y0 + (y1 - y0) * i / n
            x = sx * (c['railW'](y) * 0.86)
            z = (c['top'](y) - top_inset if top_inset is not None
                 else c['top'](y) - c['crown'](y) * 0.6 + 0.045)
            prof = [(-0.013, -0.012), (0.013, -0.012), (0.011, 0.012), (-0.011, 0.012)]
            rows.append([bm.verts.new((x + px, y, z + pz)) for px, pz in prof])
        for a_, b_ in zip(rows, rows[1:]):
            for j in range(4):
                bm.faces.new((a_[j], a_[(j + 1) % 4], b_[(j + 1) % 4], b_[j]))
        bm.faces.new(rows[0][::-1])
        bm.faces.new(rows[-1])
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        P.obj_from_bm(f'RoofRail{"L" if sx < 0 else "R"}', bm, M['chrome'] if cfg.get('dloTrim') == 'chrome' else M['gloss'], smooth=False)
        for y in (y0 + 0.03, y1 - 0.03):
            foot_z = (c['top'](y) - top_inset - 0.0155 if top_inset is not None
                      else c['top'](y) - c['crown'](y) * 0.6 + 0.02)
            P.box(f'RoofRailFoot{"L" if sx < 0 else "R"}{int(y * 100)}', (0.03, 0.10, 0.045),
                  (sx * c['railW'](y) * 0.86, y, foot_z), M['trim'])


# ---------------------------------------------------------------- rig
def rig(cfg, spec, parts):
    c = spec.c
    d0 = cfg['doors']['front'][0]
    r0 = cfg['doors'].get('rear', (99.0,))[0]
    doors_ = [('FL', d0), ('FR', d0)] + ([('RL', r0), ('RR', r0)] if 'rear' in cfg['doors'] else [])
    doors_ = [d_ for d_ in doors_ if d_[0][1] in cfg['doors'].get('sides', 'LR')]
    for s, y in doors_:
        sx = -1 if s.endswith('L') else 1
        pivot = P.empty(f'Door_{s}', (sx * (c['halfW'](y) - cfg.get('hingeInset', 0.0)), y + 0.01, 0.55))
        for nm in (f'Panel_{s}_Body', f'Window_{s}', f'Handle_{s}', f'HandleRecess_{s}', f'WindowTrim_{s}'):
            o_ = parts.get(nm) or bpy.data.objects.get(nm)
            if o_:
                P.parent_keep(o_, pivot)
        if s.startswith('F') and cfg.get('mirrorOnDoor', False):
            for nm in ('Mirror', 'MirrorGlass', 'MirrorArm'):
                o = parts.get(nm + s[1])
                if o:
                    P.parent_keep(o, pivot)
    if 'HoodSkin' in parts:
        hood = P.empty('Hood', (0, cfg['cowl'] - 0.08, c['top'](cfg['cowl'] - 0.08) - 0.01))
        hood['hingeAxis'], hood['openSign'], hood['openLimitRadians'] = 'X', -1, 1.1
        P.parent_keep(parts['HoodSkin'], hood)
    boot = cfg.get('boot', 'hatch')
    if boot == 'none':
        trunk = None
    elif boot == 'reardoor':
        # a van rear door hinged along its top edge, lifting out and up
        ztop = max(p_[1] for p_ in cfg['hatchOutline'])
        trunk = P.empty('Trunk', (0, spec.y1 - 0.02, ztop))
        trunk['hingeAxis'], trunk['openSign'], trunk['openLimitRadians'] = 'X', 1, 1.3
    elif boot == 'tailgate':
        # a pickup tailgate: hinged along its bottom edge, drops open to level
        tg = cfg.get('tailgate', {})
        trunk = P.empty('Trunk', (0, spec.y1 - 0.02, tg.get('z0', 0.9)))
        trunk['hingeAxis'], trunk['openSign'], trunk['openLimitRadians'] = 'X', -1, 1.55
    else:
        hy = cfg['roofRear'] if boot == 'hatch' else cfg['hatchBase'] + 0.02
        trunk = P.empty('Trunk', (0, hy, c['top'](hy) - 0.01))
        trunk['hingeAxis'], trunk['openSign'], trunk['openLimitRadians'] = 'X', 1, 1.1 if boot == 'hatch' else 1.25
    for nm in ({'hatch': ('TrunkSkin', 'RearWindow', 'PlateRear'), 'reardoor': ('TrunkSkin', 'RearWindow', 'PlateRear'),
                'tailgate': ('TrunkSkin',)}.get(boot, ('TrunkSkin', 'PlateRear')) if trunk else ()):
        o = parts.get(nm) or bpy.data.objects.get(nm)
        if o:
            P.parent_keep(o, trunk)

    if cfg.get('openableLampPods'):
        # One front and one rear service pod, each hinged along its lower
        # transverse seam. The pod owns every housing, reflector, emitter and
        # lens for that end, so opening it exposes real hardware rather than a
        # decorative cover floating away from the light.
        for pod_name, prefix, front, sign, channel, limit in (
            ('HeadlampPodL', 'HeadlampL', True, 1, 'head', 0.58),
            ('HeadlampPodR', 'HeadlampR', True, 1, 'head', 0.58),
            ('TaillampPodL', 'TaillampL', False, -1, 'tail', 0.62),
            ('TaillampPodR', 'TaillampR', False, -1, 'tail', 0.62),
        ):
            # The outer lens is the moving service cover. Its housing,
            # reflector, projector and emitters stay fixed underneath, so the
            # open pose exposes real hardware without multiplying the runtime
            # draw pool by every small lamp component.
            members = [o for o in bpy.context.scene.objects
                       if o.type == 'MESH' and o.name == prefix]
            if not members:
                continue
            points = [o.matrix_world @ Vector(corner)
                      for o in members for corner in o.bound_box]
            face_y = min(p.y for p in points) if front else max(p.y for p in points)
            hinge_z = min(p.z for p in points)
            pivot = P.empty(pod_name, (0, face_y, hinge_z), size=0.06)
            pivot['hingeAxis'] = 'X'
            pivot['openSign'] = sign
            pivot['openLimitRadians'] = limit
            pivot['installedLampChannel'] = channel
            for ob in members:
                P.parent_keep(ob, pivot)
            parts[pod_name] = pivot
