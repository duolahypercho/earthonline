# build_fleet.py - authored vehicle bodies for the hero overlay, per class and per LOD.
#
#   blender -b --python build_fleet.py -- <out_dir> <class>[,<class>...] [lods=0,1,2] [render]
#
# Every class is an original parametric loft sized to the catalogue entry in
# src/vehicles/vehicle-catalogue.js (length, width, height, wheelbase, wheel
# radius). Frame: Blender -Y is the nose, exported Y-up so glTF +Z is the nose,
# origin on the ground under the wheelbase centre.
#
# Node naming is the runtime contract (src/vehicles/hero-sedan-glb.js bake):
#   Wheel_FL/FR/RL/RR     steer pivots, each holding Wheel_*_Spin with the wheel parts
#   PlateFront/PlateRear  plate anchors (the runtime draws its own registrations)
#   material names        VehiclePaint, VehicleGlass, ... (see fleet_lib.materials)
#
# Three detail levels are built as CLEAN lofts at their
# own resolution rather than decimated from LOD0 - decimation of a curved panel
# leaves long triangles whose shading reads as creases. No Bevel or
# WeightedNormal modifiers on the body: both smear boolean cut edges into
# spikes (see the sedan builder's notes). Cutters carry the PAINT material so
# every shut line and opening shows a painted jamb, not a black slot.
import bpy, bmesh, math, sys, os, time, importlib, json
from mathutils import Vector, Matrix
SP = os.path.dirname(os.path.abspath(__file__))
if SP not in sys.path: sys.path.insert(0, SP)
import fleet_lib as F
importlib.reload(F)
from fleet_lib import *

# ---------------------------------------------------------------------------
# detail levels
# ---------------------------------------------------------------------------
LOD = {
    0: dict(segs=72, step=0.025, gsegs=56, gstep=0.03, wsteps=48, cyl=32, nu=20, nv=10, lipsteps=48,
            interior=True, lampDetail=True, spokes=True, brakes=True, shut=True, charline=True, reduced=False, grille=True),
    1: dict(segs=40, step=0.05, gsegs=32, gstep=0.06, wsteps=24, cyl=16, nu=12, nv=6, lipsteps=24,
            interior=True, lampDetail=False, spokes=True, brakes=False, shut=True, charline=False, reduced=False, grille=True),
    2: dict(segs=20, step=0.12, gsegs=16, gstep=0.12, wsteps=12, cyl=8, nu=6, nv=4, lipsteps=12,
            interior=False, lampDetail=False, spokes=False, brakes=False, shut=False, charline=False, reduced=True, grille=False),
}
# LOD2 collapses the material set so the distant tier costs a handful of draw calls per body.
REDUCE = dict(darkalloy='trim', steel='trim', caliper='trim', gloss='trim', shut='trim', interior='trim', seat='trim',
              dash='trim', headliner='trim', carpet='trim', alloy='chrome', lens='drl', reverse='drl', sign='drl',
              liveryDark='trim', livery='paint')

# ---------------------------------------------------------------------------
# class configurations (metres). y is Blender y: negative = nose.
# ---------------------------------------------------------------------------
def st(y, bottom, top, half): return dict(y=y, bottom=bottom, top=top, half=half)

CLASSES = {}

def _sedan_like(L, W, H, WB, R, WT, frontAxle, rearAxle, belt, noseTop, tailTop, sill,
                cowl, roofFront, roofRear, deck, roofHalf=0.955, tumble=0.30, gh_n=3.0, tail_glass_len=None):
    h = W / 2
    yF, yR = -L / 2, L / 2
    body = [
        st(yF, sill + 0.13, noseTop - 0.05, h * 0.80),
        st(yF + 0.08, sill + 0.07, noseTop, h * 0.92),
        st(yF + 0.33, sill + 0.02, noseTop + 0.04, h * 0.975),
        st(frontAxle - 0.15, sill, belt - 0.04, h * 0.995),
        st(frontAxle + 0.55, sill, belt, h),
        st((frontAxle + rearAxle) / 2, sill, belt + 0.02, h),
        st(rearAxle - 0.35, sill, belt + 0.05, h),
        st(rearAxle + 0.35, sill + 0.01, tailTop, h * 0.985),
        st(yR - 0.30, sill + 0.05, tailTop + 0.01, h * 0.94),
        st(yR - 0.10, sill + 0.09, tailTop - 0.01, h * 0.88),
        st(yR, sill + 0.15, tailTop - 0.06, h * 0.80),
    ]
    green = [
        st(cowl, belt - 0.03, belt - 0.005, h * 0.98),
        st(cowl + 0.13, belt - 0.03, belt + 0.07, h * 0.98),
        st(roofFront - 0.22, belt - 0.01, H - 0.06, h * roofHalf * 1.01),
        st(roofFront, belt, H - 0.01, h * roofHalf),
        st((roofFront + roofRear) / 2, belt + 0.02, H, h * roofHalf),
        st(roofRear, belt + 0.03, H - 0.02, h * roofHalf),
        st(deck - 0.14, belt + 0.06, (H + tailTop) / 2 - 0.1 if tail_glass_len is None else tailTop + 0.16, h * (roofHalf + 0.01)),
        st(deck, belt + 0.07, tailTop + 0.02, h * 0.975),
    ]
    return dict(L=L, W=W, H=H, WB=WB, R=R, WT=WT, frontAxle=frontAxle, rearAxle=rearAxle, belt=belt, sill=sill,
                noseTop=noseTop, tailTop=tailTop, cowl=cowl, roofFront=roofFront, roofRear=roofRear, deck=deck,
                body=body, green=green, tumble=tumble, gh_n=gh_n, kind='car', track=W - 0.30)

# catalogue: sedan 4.85 x 1.84 x 1.45, wb 2.82, r 0.340
CLASSES['sedan'] = _sedan_like(4.85, 1.84, 1.45, 2.82, 0.34, 0.225, frontAxle=-1.465, rearAxle=1.355,
                               belt=0.84, noseTop=0.70, tailTop=0.93, sill=0.16,
                               cowl=-0.80, roofFront=0.05, roofRear=0.95, deck=1.80)
# taxi 4.92 x 1.86 x 1.50, wb 2.86
CLASSES['taxi'] = dict(_sedan_like(4.92, 1.86, 1.50, 2.86, 0.345, 0.225, frontAxle=-1.49, rearAxle=1.37,
                                   belt=0.86, noseTop=0.72, tailTop=0.95, sill=0.17,
                                   cowl=-0.80, roofFront=0.07, roofRear=0.98, deck=1.84), taxi=True)
# hatchback 4.06 x 1.78 x 1.47, wb 2.56, r 0.315
CLASSES['compactHatch'] = dict(_sedan_like(4.06, 1.78, 1.47, 2.56, 0.315, 0.205, frontAxle=-1.17, rearAxle=1.39,
                                           belt=0.85, noseTop=0.70, tailTop=0.97, sill=0.16,
                                           cowl=-0.60, roofFront=0.28, roofRear=1.10, deck=1.98, roofHalf=0.94,
                                           tail_glass_len=0.6), hatch=True)
# compact SUV 4.55 x 1.86 x 1.68, wb 2.68, r 0.360
CLASSES['compactSuv'] = dict(_sedan_like(4.55, 1.86, 1.64, 2.68, 0.36, 0.235, frontAxle=-1.355, rearAxle=1.325,
                                         belt=1.00, noseTop=0.86, tailTop=1.04, sill=0.24,
                                         cowl=-0.92, roofFront=-0.15, roofRear=1.55, deck=2.24, roofHalf=0.95,
                                         tumble=0.22, tail_glass_len=0.5), suv=True, rails=True)
# pickup 5.89 x 2.03 x 1.93, wb 3.68, r 0.390
def _pickup():
    L, W, H, WB, R, WT = 5.89, 2.03, 1.93, 3.68, 0.39, 0.28
    h = W / 2; yF, yR = -L / 2, L / 2
    sill, belt = 0.30, 1.13
    body = [st(yF, 0.44, 0.92, h * 0.80), st(yF + 0.10, 0.36, 1.00, h * 0.94), st(yF + 0.40, 0.31, 1.07, h * 0.99),
            st(-2.0, sill, belt - 0.03, h), st(-1.2, sill, belt, h), st(0.5, sill, belt, h), st(1.7, sill, belt, h),
            st(yR - 0.25, sill, belt, h * 0.995), st(yR - 0.08, sill + 0.04, belt - 0.01, h * 0.97), st(yR, sill + 0.10, belt - 0.05, h * 0.92)]
    green = [st(-1.25, belt - 0.04, belt - 0.01, h * 0.99), st(-1.10, belt - 0.04, belt + 0.10, h * 0.99),
             st(-0.55, belt - 0.02, H - 0.15, h * 0.96), st(-0.30, belt, H - 0.01, h * 0.955), st(0.45, belt, H, h * 0.955),
             st(0.70, belt, H - 0.04, h * 0.955), st(0.86, belt + 0.02, belt + 0.10, h * 0.96)]
    return dict(L=L, W=W, H=H, WB=WB, R=R, WT=WT, frontAxle=-1.995, rearAxle=1.685, belt=belt, sill=sill, noseTop=1.00, tailTop=belt,
                cowl=-1.25, roofFront=-0.30, roofRear=0.45, deck=0.86, body=body, green=green, tumble=0.14, gh_n=4.0,
                kind='car', pickup=True, track=W - 0.34)
CLASSES['pickup'] = _pickup()
# panel van 5.53 x 2.03 x 2.55, wb 3.45, r 0.375
def _van():
    L, W, H, WB, R, WT = 5.53, 2.03, 2.55, 3.45, 0.375, 0.245
    h = W / 2; yF, yR = -L / 2, L / 2
    body = [st(yF, 0.42, 0.98, h * 0.84), st(yF + 0.07, 0.34, 1.06, h * 0.95), st(yF + 0.32, 0.28, 1.24, h * 0.99),
            st(yF + 0.60, 0.26, 1.58, h), st(yF + 0.95, 0.26, 2.10, h), st(yF + 1.25, 0.26, 2.42, h), st(yF + 1.60, 0.26, 2.53, h),
            st(0.5, 0.26, H, h), st(yR - 0.30, 0.26, H, h), st(yR - 0.08, 0.30, H - 0.01, h * 0.985), st(yR, 0.36, H - 0.06, h * 0.94)]
    return dict(L=L, W=W, H=H, WB=WB, R=R, WT=WT, frontAxle=-1.825, rearAxle=1.625, belt=1.30, sill=0.26, noseTop=1.06, tailTop=H,
                body=body, green=None, kind='onebox', van=True, track=W - 0.34,
                n_top_fn=lambda y: 3.2 + 3.0 * max(0.0, min(1.0, (y - (yF + 0.6)) / 1.2)))
CLASSES['deliveryVan'] = _van()
# transit bus 12.2 x 2.59 x 3.22, wb 6.1
def _bus():
    L, W, H, WB, R, WT = 12.2, 2.59, 3.20, 6.1, 0.53, 0.315
    h = W / 2; yF, yR = -L / 2, L / 2
    body = [st(yF, 0.48, 2.96, h * 0.88), st(yF + 0.10, 0.40, 3.10, h * 0.965), st(yF + 0.40, 0.37, 3.17, h * 0.995),
            st(yF + 1.0, 0.36, H, h), st(0.0, 0.36, H, h), st(yR - 1.0, 0.36, H, h), st(yR - 0.35, 0.36, H - 0.01, h * 0.995),
            st(yR - 0.10, 0.40, H - 0.04, h * 0.965), st(yR, 0.48, H - 0.14, h * 0.90)]
    return dict(L=L, W=W, H=H, WB=WB, R=R, WT=WT, frontAxle=-3.55, rearAxle=2.55, belt=1.45, sill=0.36, noseTop=3.0, tailTop=H,
                body=body, green=None, kind='onebox', bus=True, track=W - 0.44, n_top_fn=lambda y: 7.0)
CLASSES['cityBus'] = _bus()

# ---------------------------------------------------------------------------
def clip_to(ob, ref, scale=(1.0, 1.0, 1.0), dz=0.0):
    sh = ref.copy(); sh.data = ref.data.copy(); link(sh); sh.modifiers.clear(); sh.hide_render = False; sh.hide_viewport = False
    sh.scale = scale; sh.location.z = dz
    boolean_apply(ob, sh, 'INTERSECT')

def build(cls, lod=0):
    cfg = CLASSES[cls]; D = LOD[lod]
    clear(); M0 = materials()
    M = {k: (M0[REDUCE.get(k, k)] if D['reduced'] else v) for k, v in M0.items()}
    # painted cutters: every jamb and opening wall carries the paint material
    def cut(size, loc, rot=(0, 0, 0)): return prim_box('Cut', size, loc, M['paint'], rot=rot)
    L, W, H, R, WT = cfg['L'], cfg['W'], cfg['H'], cfg['R'], cfg['WT']
    FA, RA = cfg['frontAxle'], cfg['rearAxle']
    onebox = cfg['kind'] == 'onebox'
    BP = Profile(cfg['body'], n_top=4.2, n_bot=6.0, sigma=0.05, n_top_fn=cfg.get('n_top_fn'))
    GP = Profile(cfg['green'], n_top=cfg['gh_n'], n_bot=10.0, tumble=cfg['tumble'], sigma=0.04) if cfg['green'] else BP

    big = L >= 7
    segs = min(D['segs'], 40) if big else D['segs']
    body = loft('Body', BP, M['paint'], segs=segs, step=D['step'] * (1.6 if big else 1.0))
    green = None if onebox else loft('Greenhouse', GP, M['paint'], segs=D['gsegs'], step=D['gstep'])

    # ---- wheel wells ----
    ARCH_R = R + 0.055; DEPTH = 0.30
    for ay in (FA, RA):
        for sx in (-1, 1):
            cutter = prim_cyl('ArchCutter', ARCH_R, DEPTH + 0.1, (sx * (W / 2 - DEPTH / 2 + 0.05), ay, R), 'X', segs=max(16, D['cyl'] * 2), mat=M['trim'])
            boolean_apply(body, cutter)
    shell0 = body.copy(); shell0.data = body.data.copy(); link(shell0); shell0.name = 'ShellRef'; shell0.hide_render = True; shell0.hide_viewport = True

    # ---- lamps ----
    def recess_and_lamp(name, x, y, z, size, yaw, kind):
        front = y < 0
        for sx in (-1, 1):
            tag = ('L' if sx < 0 else 'R')
            boolean_apply(body, prim_box('Cut', size, (sx * x, y, z), M['trim'], rot=(0, 0, sx * yaw)))
            housing = prim_box(name + tag + '_Housing', (size[0] * 0.995, size[1] * 0.995, size[2] * 0.995), (sx * x, y, z), M['trim'], rot=(0, 0, sx * yaw))
            clip_to(housing, shell0, (0.994, 0.993, 0.994))
            def place(ob, px, pz, back):
                sy = body_surface_y(BP, px, pz, front)
                ob.location = (px, sy + (back if front else -back), pz)
                clip_to(ob, shell0, (0.997, 0.997, 0.997))
            hw = size[0] * 0.42
            if kind == 'head':
                if D['lampDetail']:
                    for k, ox in ((0, -0.12), (1, 0.06)):
                        rr = 0.046 if k == 0 else 0.036
                        px = sx * x + sx * ox * math.cos(yaw)
                        dish = prim_cyl(name + tag + f'_Dish{k}', rr, 0.03, (0, 0, 0), 'Y', segs=D['cyl'], mat=M['chrome'], r2=rr * 0.45)
                        place(dish, px, z - 0.005, 0.028)
                        lens = prim_cyl(name + tag + f'_Lens{k}', rr * 0.6, 0.012, (0, 0, 0), 'Y', segs=D['cyl'], mat=M['lens'])
                        place(lens, px, z - 0.005, 0.010)
                    for k in range(4):
                        px = sx * x + sx * (-hw + 2 * hw * (k + 0.5) / 4) * math.cos(yaw)
                        seg = prim_box(name + tag + f'_DRL{k}', (2 * hw / 4 + 0.004, 0.006, 0.012), (0, 0, 0), M['drl'], rot=(0, 0, sx * yaw))
                        place(seg, px, z + size[2] * 0.30, 0.012)
                    ind = prim_box(name + tag + '_Ind', (size[0] * 0.30, 0.008, 0.02), (0, 0, 0), M['amber'], rot=(0, 0, sx * yaw))
                    place(ind, sx * x + sx * 0.22 * math.cos(yaw), z - size[2] * 0.30, 0.012)
                else:
                    lens = prim_box(name + tag + '_Lens', (size[0] * 0.70, 0.008, size[2] * 0.45), (0, 0, 0), M['lens'], rot=(0, 0, sx * yaw))
                    place(lens, sx * x, z, 0.012)
                    seg = prim_box(name + tag + '_DRL', (2 * hw + 0.004, 0.006, 0.012), (0, 0, 0), M['drl'], rot=(0, 0, sx * yaw))
                    place(seg, sx * x, z + size[2] * 0.30, 0.012)
                if not D['reduced']:
                    cover = prim_box(name + tag, (size[0] * 0.90, size[1] * 0.95, size[2] * 0.80), (sx * x, y, z), M['headcover'], rot=(0, 0, sx * yaw))
                    clip_to(cover, shell0, (0.9985, 0.999, 0.9985))
                    inner = shell0.copy(); inner.data = shell0.data.copy(); link(inner); inner.hide_render = False; inner.hide_viewport = False
                    inner.scale = (0.9955, 0.9945, 0.9955)
                    boolean_apply(cover, inner, 'DIFFERENCE')
            else:
                if D['lampDetail']:
                    for k in range(4):
                        px = sx * x + sx * (-hw + 2 * hw * (k + 0.5) / 4) * math.cos(yaw)
                        bar = prim_box(name + tag + f'_Bar{k}', (2 * hw / 4 + 0.004, 0.01, size[2] * 0.42), (0, 0, 0), M['chrome'], rot=(0, 0, sx * yaw))
                        place(bar, px, z + size[2] * 0.08, 0.022)
                        seg = prim_box(name + tag + f'_Strip{k}', (2 * hw / 4 + 0.004, 0.008, size[2] * 0.30), (0, 0, 0), M['tail'], rot=(0, 0, sx * yaw))
                        place(seg, px, z + size[2] * 0.14, 0.010)
                    rev = prim_box(name + tag + '_Rev', (size[0] * 0.28, 0.008, size[2] * 0.22), (0, 0, 0), M['reverse'], rot=(0, 0, sx * yaw))
                    place(rev, sx * x - sx * 0.10 * math.cos(yaw), z - size[2] * 0.24, 0.010)
                    ind = prim_box(name + tag + '_Ind', (size[0] * 0.34, 0.008, size[2] * 0.22), (0, 0, 0), M['amber'], rot=(0, 0, sx * yaw))
                    place(ind, sx * x + sx * 0.14 * math.cos(yaw), z - size[2] * 0.24, 0.010)
                else:
                    seg = prim_box(name + tag + '_Strip', (2 * hw + 0.004, 0.008, size[2] * 0.36), (0, 0, 0), M['tail'], rot=(0, 0, sx * yaw))
                    place(seg, sx * x, z + size[2] * 0.12, 0.010)
                    if not D['reduced']:
                        ind = prim_box(name + tag + '_Ind', (size[0] * 0.34, 0.008, size[2] * 0.22), (0, 0, 0), M['amber'], rot=(0, 0, sx * yaw))
                        place(ind, sx * x + sx * 0.14 * math.cos(yaw), z - size[2] * 0.24, 0.010)

    yF, yR = -L / 2, L / 2
    if cfg.get('bus'):
        recess_and_lamp('Headlamp', W / 2 - 0.40, yF + 0.05, 0.90, (0.44, 0.24, 0.20), math.radians(-12), 'head')
        recess_and_lamp('Taillamp', W / 2 - 0.30, yR - 0.05, 1.20, (0.22, 0.20, 0.60), math.radians(8), 'tail')
    elif cfg.get('van'):
        recess_and_lamp('Headlamp', W / 2 - 0.36, yF + 0.06, 1.02, (0.46, 0.28, 0.30), math.radians(-24), 'head')
        recess_and_lamp('Taillamp', W / 2 - 0.16, yR - 0.05, 1.30, (0.16, 0.20, 0.70), math.radians(4), 'tail')
    elif cfg.get('pickup'):
        recess_and_lamp('Headlamp', W / 2 - 0.36, yF + 0.06, 0.88, (0.44, 0.30, 0.24), math.radians(-22), 'head')
        recess_and_lamp('Taillamp', W / 2 - 0.16, yR - 0.05, 0.92, (0.18, 0.22, 0.34), math.radians(4), 'tail')
    else:
        hz = cfg['noseTop'] - 0.12
        recess_and_lamp('Headlamp', W / 2 - 0.30, yF + 0.06, hz, (0.50, 0.30, 0.13), math.radians(-26), 'head')
        tz = cfg['tailTop'] - 0.16
        recess_and_lamp('Taillamp', W / 2 - 0.30, yR - 0.06, tz, (0.52, 0.26, 0.11), math.radians(22), 'tail')

    # ---- grille + intake ----
    gz = cfg['noseTop'] - 0.19 if not onebox else cfg['noseTop'] - 0.30
    gw = 0.74 if not big else 1.2
    boolean_apply(body, prim_box('Cut', (gw, 0.20, 0.15), (0, yF, gz), M['trim']))
    prim_box('GrilleBack', (gw, 0.16, 0.15), (0, yF + 0.06, gz), M['trim'])
    if D['grille']:
        for i in range(3):
            prim_box(f'GrilleSlat{i}', (gw - 0.02, 0.02, 0.016), (0, yF + 0.01, gz - 0.05 + i * 0.05), M['chrome'])
        prim_box('GrilleFrame_T', (gw, 0.02, 0.014), (0, yF + 0.008, gz + 0.08), M['chrome'])
        prim_box('GrilleFrame_B', (gw, 0.02, 0.014), (0, yF + 0.008, gz - 0.08), M['chrome'])
    boolean_apply(body, prim_box('Cut', (gw + 0.16, 0.20, 0.06), (0, yF, cfg['sill'] + 0.20), M['trim']))
    prim_box('IntakeBack', (gw + 0.16, 0.16, 0.06), (0, yF + 0.06, cfg['sill'] + 0.20), M['trim'])

    # ---- fender lips ----
    for ay in (FA, RA):
        for sx in (-1, 1):
            lip = arch_lip('FenderLip' + ('F' if ay < 0 else 'R') + ('L' if sx < 0 else 'R'), BP, ay, R, ARCH_R + 0.002, 0.012, M['paint'], proud=0.005, steps=D['lipsteps'], ring=8 if lod == 0 else 5)
            if sx < 0: lip.scale.x = -1
    body.data.materials.append(M['trim'])
    for p in body.data.polygons:
        c = p.center
        for ay in (FA, RA):
            if abs(c.x) > W / 2 - DEPTH - 0.02 and math.hypot(c.y - ay, c.z - R) < ARCH_R + 0.003 and c.z > 0.19:
                p.material_index = 1
    # (no Bevel / WeightedNormal on the body: both smear boolean cut edges into spikes)

    # ---- valances / rockers ----
    vz = cfg['sill'] + 0.12
    valF = prim_box('ValanceFront', (W + 0.2, 0.40, 0.14), (0, yF + 0.15, vz), M['trim']); clip_to(valF, shell0, (1.004, 1.0015, 1.004), -0.004)
    valR = prim_box('ValanceRear', (W + 0.2, 0.40, 0.14), (0, yR - 0.15, vz), M['trim']); clip_to(valR, shell0, (1.004, 1.0015, 1.004), -0.004)
    for sx in (-1, 1):
        rk = prim_box('Rocker' + ('L' if sx < 0 else 'R'), (0.3, (RA - FA) - 0.9, 0.07), (sx * (W / 2 - 0.02), (FA + RA) / 2, cfg['sill'] + 0.045), M['gloss']); clip_to(rk, shell0, (1.004, 1.0, 1.004), -0.004)
    if (cfg.get('suv') or cfg.get('pickup')) and not D['reduced']:
        for ay in (FA, RA):
            for sx in (-1, 1):
                clad = arch_lip('Cladding' + ('F' if ay < 0 else 'R') + ('L' if sx < 0 else 'R'), BP, ay, R, ARCH_R + 0.03, 0.02, M['trim'], proud=0.006, steps=D['lipsteps'], ring=6)
                if sx < 0: clad.scale.x = -1

    # ---- wheels ----
    TRACK = cfg['track']
    R_RIM = R * 0.72
    def tyre_mesh(name):
        bm = bmesh.new()
        r_in, sh = R_RIM - 0.006, 0.05
        n = 6 if lod == 0 else 3
        prof = [(-WT / 2, r_in), (-WT / 2, R - sh)]
        for i in range(1, n + 1):
            a = math.pi / 2 * i / n
            prof.append((-WT / 2 + sh - sh * math.cos(a), R - sh + sh * math.sin(a)))
        for i in range(0, n + 1):
            a = math.pi / 2 * i / n
            prof.append((WT / 2 - sh + sh * math.sin(a), R - sh + sh * math.cos(a)))
        prof.append((WT / 2, r_in))
        verts = [bm.verts.new((x, 0, r)) for x, r in prof]
        edges = [bm.edges.new((verts[i], verts[i + 1])) for i in range(len(verts) - 1)]
        edges.append(bm.edges.new((verts[-1], verts[0])))
        bmesh.ops.spin(bm, geom=verts + edges, cent=(0, 0, 0), axis=(1, 0, 0), angle=2 * math.pi, steps=D['wsteps'], use_merge=True)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        return new_obj(name, bm, M['rubber'])
    def make_wheel(name, loc, flip):
        steer = bpy.data.objects.new(name, None); steer.location = loc; link(steer); steer.empty_display_size = 0.1
        parent = bpy.data.objects.new(name + '_Spin', None); link(parent); parent.parent = steer; parent.empty_display_size = 0.08
        tyre = tyre_mesh(name + '_Tyre'); tyre.parent = parent
        face_x = flip * (WT / 2 - 0.02)
        if D['spokes']:
            barrel = prim_cyl(name + '_Barrel', R_RIM - 0.004, WT - 0.10, (-flip * 0.045, 0, 0), 'X', segs=D['cyl'], mat=M['darkalloy']); barrel.parent = parent
            lip = prim_cyl(name + '_Lip', R_RIM, 0.03, (face_x, 0, 0), 'X', segs=D['wsteps'], mat=M['alloy'])
            boolean_apply(lip, prim_cyl('Hole', R_RIM - 0.026, 0.08, (face_x, 0, 0), 'X', segs=D['wsteps']))
            lip.parent = parent
            hub = prim_cyl(name + '_Hub', R * 0.22, 0.05, (face_x, 0, 0), 'X', segs=D['cyl'], mat=M['alloy']); hub.parent = parent
            cap = prim_cyl(name + '_Cap', R * 0.10, 0.014, (face_x + flip * 0.028, 0, 0), 'X', segs=max(8, D['cyl'] // 2), mat=M['chrome']); cap.parent = parent
            ln = R_RIM - 0.03
            for i in range(5):
                ang = i * 2 * math.pi / 5
                sp = prim_box(name + f'_Spoke{i}', (0.040, 0.058, ln), (0, 0, 0), M['alloy'], bevel=0.008 if lod == 0 else 0.0, bevel_segs=1)
                sp.rotation_euler = (ang, 0, 0)
                sp.location = (face_x - flip * 0.010, -math.sin(ang) * ln / 2, math.cos(ang) * ln / 2)
                sp.parent = parent
        else:
            # distant tier: a single alloy disc reads as a wheel face at 70 m+
            disc = prim_cyl(name + '_Face', R_RIM, 0.02, (face_x, 0, 0), 'X', segs=D['wsteps'], mat=M['alloy']); disc.parent = parent
        if D['brakes']:
            disc = prim_cyl(name + '_Disc', R * 0.50, 0.026, (flip * 0.03, 0, 0), 'X', segs=D['cyl'], mat=M['steel']); disc.parent = parent
            cal = prim_box(name + '_Caliper', (0.06, 0.08, 0.16), (flip * 0.03, -0.11, 0.11), M['caliper'], bevel=0.01)
            cal.rotation_euler = (math.radians(-40), 0, 0); cal.parent = parent
        return parent
    for i, ay in enumerate((FA, RA)):
        for sx in (-1, 1):
            make_wheel(f'Wheel_{"F" if i == 0 else "R"}{"L" if sx < 0 else "R"}', (sx * TRACK / 2, ay, R), sx)

    # ---- glazing ----
    NU, NV = D['nu'], D['nv']
    def pane(name, prof, t_range, yfn, gasket_fn, nu=NU, nv=NV):
        t0, t1 = t_range; dt = (t1 - t0) * 0.06
        if not D['reduced']:
            patch(name + '_GasketLo', prof, M['gloss'], (t0, t0 + dt), gasket_fn, expand=0.003, nu=nu, nv=2)
            patch(name + '_GasketHi', prof, M['gloss'], (t1 - dt, t1), gasket_fn, expand=0.003, nu=nu, nv=2)
            patch(name + '_GasketA', prof, M['gloss'], t_range, lambda t: (gasket_fn(t)[0], gasket_fn(t)[0] + 0.03), expand=0.003, nu=2, nv=nv)
            patch(name + '_GasketB', prof, M['gloss'], t_range, lambda t: (gasket_fn(t)[1] - 0.03, gasket_fn(t)[1]), expand=0.003, nu=2, nv=nv)
        patch(name, prof, M['glass'], t_range, yfn, expand=0.006, nu=nu, nv=nv)
    def zf_of(prof, t, y=0.0):
        s = math.sin(t); n = prof.ntop(y) if s >= 0 else prof.n_bot
        return 0.5 + 0.5 * math.copysign(abs(s) ** (2 / n), s)

    if not onebox:
        cowl, rf, rr, deck, belt = cfg['cowl'], cfg['roofFront'], cfg['roofRear'], cfg['deck'], cfg['belt']
        t_top = GP.t_for_sx(0.80, (cowl + rf) / 2)
        wsY = (cowl + 0.05, rf - 0.06)
        pane('Windscreen', GP, (t_top, math.pi - t_top), lambda t: wsY, lambda t: (wsY[0] - 0.04, wsY[1] + 0.04), nu=max(6, NU // 2), nv=max(4, NV))
        rwY = (rr + 0.06, deck - 0.06)
        if cfg.get('hatch') or cfg.get('suv'):
            rwY = (rr + 0.05, deck - 0.05)
        pane('RearWindow', GP, (t_top, math.pi - t_top), lambda t: rwY, lambda t: (rwY[0] - 0.04, rwY[1] + 0.04), nu=max(6, NU // 2), nv=max(4, NV))
        bpil = (rf + rr) / 2 - 0.05 if not cfg.get('pickup') else (rf + rr) / 2 + 0.05
        for side in (1, -1):
            tag = 'R' if side > 0 else 'L'
            zlo, zhi = GP.t_for_zf(0.10, side), GP.t_for_zf(0.86, side)
            fy0, fy1 = cowl + 0.25, bpil - 0.03
            ry0, ry1 = bpil + 0.03, (rr + 0.55 if not cfg.get('pickup') else deck - 0.02)
            if cfg.get('hatch') or cfg.get('suv'): ry1 = deck - 0.30
            rake = rf - (cowl + 0.25)
            def front_fn(t, side=side): zf = zf_of(GP, t); return (fy0 + zf * rake * 0.85, fy1 - zf * 0.02)
            def front_g(t, side=side): zf = zf_of(GP, t); return (fy0 - 0.04 + zf * rake * 0.85, fy1 + 0.03 - zf * 0.02)
            crake = (deck - rr) * 0.4
            def rear_fn(t, side=side): zf = zf_of(GP, t); return (ry0 + zf * 0.02, ry1 - zf * crake)
            def rear_g(t, side=side): zf = zf_of(GP, t); return (ry0 - 0.03 + zf * 0.02, ry1 + 0.04 - zf * crake)
            pane('SideFront' + tag, GP, (zlo, zhi), front_fn, front_g, nu=max(4, NU // 2), nv=max(3, NV // 2 + 1))
            pane('SideRear' + tag, GP, (zlo, zhi), rear_fn, rear_g, nu=max(4, NU // 2), nv=max(3, NV // 2 + 1))
        # openings in the greenhouse so the panes are see-through (painted jambs)
        hw = cfg['W'] / 2
        boolean_apply(green, cut((hw * 1.30, wsY[1] - wsY[0] - 0.02, H), (0, (wsY[0] + wsY[1]) / 2, belt + 0.10 + H / 2)))
        boolean_apply(green, cut((hw * 1.30, rwY[1] - rwY[0] - 0.02, H), (0, (rwY[0] + rwY[1]) / 2, belt + 0.10 + H / 2)))
        for side in (1, -1):
            sx = side
            zc = belt + (H - belt) * 0.48; zh = (H - belt) * 0.70
            a = cut((0.6, fy1 - fy0 - 0.30, zh), (sx * hw * 0.9, (fy0 + fy1) / 2 + 0.12, zc))
            b = cut((0.7, 3.0, 2.0), (sx * hw * 0.9, (fy0 + fy1) / 2 + 0.9, zc), rot=(-math.atan2(rake * 0.85, H - belt), 0, 0))
            boolean_apply(a, b, 'INTERSECT'); boolean_apply(green, a)
            a = cut((0.6, ry1 - ry0 - 0.10, zh), (sx * hw * 0.9, (ry0 + ry1) / 2 - 0.05, zc))
            b = cut((0.7, 3.0, 2.0), (sx * hw * 0.9, (ry0 + ry1) / 2 - 1.3, zc), rot=(math.atan2(crake, H - belt), 0, 0))
            boolean_apply(a, b, 'INTERSECT'); boolean_apply(green, a)
        if D['interior']:
            # cabin liner: headliner + door cards so the cut greenhouse does not read hollow
            liner = loft('CabinLiner', GP, M['headliner'], segs=max(12, D['gsegs'] // 2), step=0.10, expand=-0.035)
            boolean_apply(liner, prim_box('Cut', (hw * 2.4, deck - cowl, 0.5), (0, (cowl + deck) / 2, belt + 0.05 + 0.25 * 0.8), M['headliner']))
            seatz = cfg['sill'] + 0.25
            for sx in (-0.38, 0.38):
                prim_box(f'SeatF{"L" if sx < 0 else "R"}', (0.50, 0.52, 0.22), (sx, cowl + 1.05, seatz + 0.11), M['seat'], bevel=0.03)
                prim_box(f'SeatFB{"L" if sx < 0 else "R"}', (0.50, 0.12, 0.62), (sx, cowl + 1.30, seatz + 0.50), M['seat'], bevel=0.03, rot=(math.radians(-12), 0, 0))
                prim_box(f'Headrest{"L" if sx < 0 else "R"}', (0.26, 0.10, 0.16), (sx, cowl + 1.40, seatz + 0.92), M['seat'], bevel=0.03)
            prim_box('SeatR', (1.30, 0.50, 0.22), (0, rr - 0.10, seatz + 0.11), M['seat'], bevel=0.03)
            prim_box('SeatRB', (1.30, 0.12, 0.60), (0, rr + 0.12, seatz + 0.48), M['seat'], bevel=0.03, rot=(math.radians(-18), 0, 0))
            for sx in (-0.40, 0.40):
                prim_box(f'HeadrestR{"L" if sx < 0 else "R"}', (0.26, 0.10, 0.15), (sx, rr + 0.22, seatz + 0.88), M['seat'], bevel=0.03)
            prim_box('Dash', (hw * 1.7, 0.50, 0.26), (0, cowl + 0.30, belt - 0.14), M['dash'], bevel=0.02)
            prim_box('Console', (0.30, 1.2, 0.20), (0, cowl + 1.0, seatz + 0.05), M['dash'])
            prim_box('FloorPan', (hw * 1.8, deck - cowl - 0.2, 0.02), (0, (cowl + deck) / 2, cfg['sill'] + 0.05), M['carpet'])
            if not (cfg.get('hatch') or cfg.get('suv') or cfg.get('pickup')):
                prim_box('RearShelf', (hw * 1.7, deck - rr - 0.10, 0.04), (0, (rr + deck) / 2 + 0.02, belt + 0.03), M['carpet'])
            sw = prim_cyl('Steering', 0.19, 0.03, (-0.38, cowl + 0.68, belt + 0.12), 'Y', segs=D['cyl'], mat=M['dash'])
            sw.rotation_euler = (math.radians(-25), 0, 0)
        else:
            # distant tier: a dark cabin block so the glass reads as a volume, not a hole.
            # Sized to sit INSIDE the glasshouse. The old block
            # (0.8 W wide, cowl-to-deck long, 55% of the glass height) stood proud of
            # the raked windscreen and rear window by ~0.19 m and of the tumblehome
            # side glass at its top corners, so at 40-80 m every far body showed a
            # black slab where its glass should reflect the sky - the "far row reads
            # as unlit black silhouettes" rejection. The block now ends 0.55 m short
            # of the cowl and the deck, reaches 40% of the glass height and is 0.6 W
            # wide, which the pane rake and tumblehome of every class clear.
            prim_box('CabinBlock', (hw * 1.2, max(0.6, deck - cowl - 1.1), (H - belt) * 0.40), (0, (cowl + deck) / 2 + 0.1, belt + (H - belt) * 0.24), M['interior'])
        prim_box('Underbody', (W * 0.70, (RA - FA) + 0.4, 0.05), (0, (FA + RA) / 2, cfg['sill'] + 0.03), M['trim'])
    else:
        hw = W / 2
        if cfg.get('bus'):
            wsY = (yF - 0.002, yF + 0.16)
            prim_box('Windscreen', (W * 0.86, 0.012, 1.55), (0, yF - 0.004, 2.05), M['glass'])
            prim_box('Windscreen_Gasket', (W * 0.90, 0.010, 1.62), (0, yF + 0.002, 2.05), M['gloss'])
            prim_box('DestSign', (W * 0.70, 0.06, 0.30), (0, yF + 0.02, 2.95), M['sign'])
            y0, y1 = yF + 1.3, yR - 0.7
            n = int((y1 - y0) / 1.40)
            for k in range(n):
                wy0 = y0 + k * (y1 - y0) / n + 0.06; wy1 = y0 + (k + 1) * (y1 - y0) / n - 0.06
                for side in (1, -1):
                    tag = 'R' if side > 0 else 'L'
                    zlo, zhi = BP.t_for_zf(0.46, side), BP.t_for_zf(0.86, side)
                    pane(f'Side{k}{tag}', BP, (zlo, zhi), lambda t, a=wy0, b=wy1: (a, b), lambda t, a=wy0, b=wy1: (a - 0.03, b + 0.03), nu=3, nv=4)
                boolean_apply(body, cut((W + 0.4, wy1 - wy0, 1.25), (0, (wy0 + wy1) / 2, 2.10)))
            for dy in (yF + 0.45, 0.3):
                prim_box(f'Door{int(dy)}', (0.02, 1.10, 2.2), ((hw + 0.004), dy + 0.55, 1.45), M['glass'])
                prim_box(f'DoorFrame{int(dy)}', (0.016, 1.16, 2.26), ((hw + 0.002), dy + 0.55, 1.45), M['gloss'])
            if D['interior']:
                liner = loft('CabinLiner', BP, M['headliner'], segs=16, step=0.30, expand=-0.05)
                boolean_apply(liner, prim_box('Cut', (W + 0.4, y1 - y0 + 0.5, 1.4), (0, (y0 + y1) / 2, 2.10), M['headliner']))
                for k in range(6):
                    prim_box(f'SeatRow{k}', (W * 0.80, 0.45, 0.9), (0, yF + 2.2 + k * 1.4, 1.20), M['seat'])
        else:
            # van: windscreen on the sloped top, front door windows on the flanks
            t_top = BP.t_for_sx(0.84, yF + 0.9)
            wsY = (yF + 0.62, yF + 1.22)
            pane('Windscreen', BP, (t_top, math.pi - t_top), lambda t: wsY, lambda t: (wsY[0] - 0.04, wsY[1] + 0.04), nu=max(6, NU // 2), nv=max(4, NV))
            boolean_apply(body, cut((hw * 1.4, wsY[1] - wsY[0] - 0.02, 2.2), (0, (wsY[0] + wsY[1]) / 2, 2.6)))
            fy0, fy1 = yF + 1.45, yF + 2.45
            for side in (1, -1):
                tag = 'R' if side > 0 else 'L'
                zlo, zhi = BP.t_for_zf(0.50, side, fy0), BP.t_for_zf(0.82, side, fy0)
                pane('SideFront' + tag, BP, (zlo, zhi), lambda t: (fy0 + zf_of(BP, t, fy0) * 0.30, fy1), lambda t: (fy0 - 0.04 + zf_of(BP, t, fy0) * 0.30, fy1 + 0.04), nu=max(4, NU // 2), nv=max(3, NV // 2 + 1))
            boolean_apply(body, cut((W + 0.4, fy1 - fy0 - 0.20, 0.72), (0, (fy0 + fy1) / 2 + 0.08, 1.75)))
            if D['interior']:
                liner = loft('CabinLiner', BP, M['headliner'], segs=16, step=0.20, expand=-0.04)
                boolean_apply(liner, prim_box('Cut', (W + 0.4, 2.3, 1.2), (0, yF + 1.6, 1.9), M['headliner']))
                for sx in (-0.42, 0.42):
                    prim_box(f'SeatF{"L" if sx < 0 else "R"}', (0.52, 0.55, 0.24), (sx, yF + 2.0, 0.95), M['seat'], bevel=0.03)
                    prim_box(f'SeatFB{"L" if sx < 0 else "R"}', (0.52, 0.12, 0.66), (sx, yF + 2.26, 1.38), M['seat'], bevel=0.03, rot=(math.radians(-10), 0, 0))
                    prim_box(f'Headrest{"L" if sx < 0 else "R"}', (0.26, 0.10, 0.16), (sx, yF + 2.36, 1.82), M['seat'], bevel=0.03)
                prim_box('Dash', (hw * 1.7, 0.55, 0.30), (0, yF + 1.25, 1.18), M['dash'], bevel=0.02)
                sw = prim_cyl('Steering', 0.20, 0.03, (-0.42, yF + 1.60, 1.30), 'Y', segs=D['cyl'], mat=M['dash']); sw.rotation_euler = (math.radians(-30), 0, 0)
                # cargo bulkhead behind the cab
                prim_box('Bulkhead', (W - 0.10, 0.03, H - 0.60), (0, yF + 2.75, H / 2 + 0.05), M['interior'])
            else:
                prim_box('CabinBlock', (W - 0.2, 1.6, 1.0), (0, yF + 1.9, 1.5), M['interior'])
            if D['shut']:
                for sx in (-1, 1):
                    for yy in (fy1 + 0.12, fy1 + 1.55):
                        shut_line_obj(BP, M, f'ShutSlide{int(yy*100)}', yy, cfg['sill'] + 0.10, H - 0.35, sx)
                prim_box('RearSeam', (0.004, 0.006, H - 0.6), (0, yR + 0.002, H / 2 + 0.1), M['shut'])
            prim_box('Underbody', (W * 0.70, (RA - FA) + 0.4, 0.05), (0, (FA + RA) / 2, cfg['sill'] + 0.03), M['trim'])

    # ---- exterior details ----
    mirror_y = (cfg['cowl'] + 0.24) if not onebox else (yF + 1.35 if cfg.get('van') else yF + 0.35)
    mirror_z = (cfg['belt'] + 0.06) if not onebox else (1.55 if cfg.get('van') else 2.0)
    for sx in (-1, 1):
        mx = body_surface_x(BP, mirror_y, mirror_z) if not cfg.get('bus') else hw
        prim_box('MirrorArm' + ('L' if sx < 0 else 'R'), (0.16, 0.05, 0.03), (sx * (mx + 0.05), mirror_y, mirror_z), M['gloss'], bevel=0.006 if lod == 0 else 0)
        prim_box('Mirror' + ('L' if sx < 0 else 'R'), (0.11, 0.19, 0.11) if not cfg.get('bus') else (0.10, 0.25, 0.42), (sx * (mx + 0.14), mirror_y - 0.01, mirror_z + 0.03), M['paint'] if not cfg.get('bus') else M['trim'], bevel=0.02 if lod < 2 else 0, bevel_segs=2, rot=(0, 0, sx * math.radians(-8)))
        if not D['reduced']:
            prim_box('MirrorGlass' + ('L' if sx < 0 else 'R'), (0.09, 0.005, 0.085) if not cfg.get('bus') else (0.08, 0.005, 0.36), (sx * (mx + 0.14), mirror_y - 0.01 + 0.10, mirror_z + 0.03), M['chrome'], rot=(0, 0, sx * math.radians(-8)))
    if not onebox:
        cowl, rf, rr, deck, belt = cfg['cowl'], cfg['roofFront'], cfg['roofRear'], cfg['deck'], cfg['belt']
        bpil = (rf + rr) / 2 - 0.05 if not cfg.get('pickup') else (rf + rr) / 2 + 0.05
        doors = [(cowl + 0.18, bpil)] if cfg.get('pickup') else [(cowl + 0.18, bpil), (bpil, rr + 0.60 if not (cfg.get('hatch') or cfg.get('suv')) else deck - 0.35)]
        if not D['reduced']:
            for sx in (-1, 1):
                for (d0, d1) in doors:
                    hy = d0 + (d1 - d0) * 0.60
                    hx = body_surface_x(BP, hy, belt - 0.16)
                    prim_box(f'Handle{"L" if sx < 0 else "R"}{int((hy+5)*100)}', (0.022, 0.15, 0.03), (sx * (hx + 0.008), hy, belt - 0.16), M['paint'], bevel=0.006 if lod == 0 else 0)
        if D['shut']:
            for k, (d0, d1) in enumerate(doors):
                for sx in (-1, 1):
                    shut_line_obj(BP, M, f'ShutDoor{k}A', d0, cfg['sill'] + 0.08, belt - 0.02, sx)
                    shut_line_obj(BP, M, f'ShutDoor{k}B', d1, cfg['sill'] + 0.08, belt - 0.02, sx)
            tb2 = BP.t_for_sx(0.90)
            patch('BumperSeamF', BP, M['shut'], (tb2, math.pi - tb2), lambda t: (yF + 0.30, yF + 0.31), expand=0.0015, nu=2, nv=16)
            patch('BumperSeamR', BP, M['shut'], (tb2, math.pi - tb2), lambda t: (yR - 0.31, yR - 0.30), expand=0.0015, nu=2, nv=16)
            tb = BP.t_for_sx(0.86)
            patch('BonnetLineL', BP, M['shut'], (math.pi - tb - 0.03, math.pi - tb + 0.03), lambda t: (yF + 0.22, cowl - 0.03), expand=0.0015, nu=14, nv=2)
            patch('BonnetLineR', BP, M['shut'], (tb - 0.03, tb + 0.03), lambda t: (yF + 0.22, cowl - 0.03), expand=0.0015, nu=14, nv=2)
            patch('CowlLine', BP, M['shut'], (tb, math.pi - tb), lambda t: (cowl - 0.035, cowl - 0.025), expand=0.0015, nu=2, nv=14)
            if not (cfg.get('hatch') or cfg.get('suv') or cfg.get('pickup')):
                patch('BootLine', BP, M['shut'], (tb, math.pi - tb), lambda t: (deck + 0.045, deck + 0.055), expand=0.0015, nu=2, nv=14)
                patch('BootLineL', BP, M['shut'], (math.pi - tb - 0.03, math.pi - tb + 0.03), lambda t: (deck + 0.05, yR - 0.30), expand=0.0015, nu=6, nv=2)
                patch('BootLineR', BP, M['shut'], (tb - 0.03, tb + 0.03), lambda t: (deck + 0.05, yR - 0.30), expand=0.0015, nu=6, nv=2)
        if cfg.get('pickup'):
            boolean_apply(body, cut((W - 0.34, yR - deck - 0.35, 0.70), (0, (deck + 0.22 + yR - 0.13) / 2, belt + 0.02 - 0.35 + 0.30)))
            prim_box('BedLiner', (W - 0.36, yR - deck - 0.37, 0.04), (0, (deck + 0.22 + yR - 0.13) / 2, belt - 0.42), M['interior'])
            for sx in (-1, 1):
                prim_box(f'BedWallLiner{"L" if sx < 0 else "R"}', (0.03, yR - deck - 0.37, 0.44), (sx * (W / 2 - 0.19), (deck + 0.22 + yR - 0.13) / 2, belt - 0.20), M['interior'])
            prim_box('CabBack', (W - 0.30, 0.03, 0.44), (0, deck + 0.22, belt - 0.20), M['interior'])
            if D['shut']: prim_box('TailgateSeamT', (W - 0.30, 0.006, 0.006), (0, yR - 0.13, belt + 0.004), M['shut'])
        if D['charline']:
            for side in (1, -1):
                segs_ = [(yF + 0.35, cowl - 0.08), (cowl + 0.20, bpil - 0.02), (bpil + 0.02, doors[-1][1] - 0.02), (doors[-1][1] + 0.02, yR - 0.35)]
                for k, (ya, yb) in enumerate(segs_):
                    if yb - ya < 0.15: continue
                    patch(f'CharLine{"R" if side > 0 else "L"}{k}', BP, M['paint'], (BP.t_for_zf(0.60, side), BP.t_for_zf(0.72, side)), lambda t, ya=ya, yb=yb: (ya, yb), expand=0.006, nu=16, nv=4)
        if not D['reduced']:
            for sx in (-1, 1):
                prim_box('Wiper' + ('L' if sx < 0 else 'R'), (0.02, 0.48, 0.012), (sx * 0.36, cowl + 0.16, belt + 0.10), M['trim'], rot=(math.radians(33), 0, math.radians(sx * -14)))
            prim_box('Antenna', (0.06, 0.18, 0.05), (0, rr - 0.05, H - 0.01), M['paint'], bevel=0.015 if lod == 0 else 0, bevel_segs=2)
        if cfg.get('rails'):
            for sx in (-1, 1):
                prim_box('RoofRail' + ('L' if sx < 0 else 'R'), (0.05, rr - rf + 0.6, 0.05), (sx * (hw * 0.80), (rf + rr) / 2, H + 0.02), M['trim'], bevel=0.012 if lod == 0 else 0)
        if cfg.get('taxi'):
            prim_box('TaxiSign', (0.52, 0.20, 0.13), (0, (rf + rr) / 2, H + 0.065), M['sign'], bevel=0.015 if lod == 0 else 0)
            prim_box('TaxiSignBase', (0.30, 0.16, 0.03), (0, (rf + rr) / 2, H + 0.005), M['trim'])
            if not D['reduced']:
                for side in (1, -1):
                    patch(f'LiveryBand{"R" if side > 0 else "L"}', BP, M['liveryDark'], (BP.t_for_zf(0.40, side), BP.t_for_zf(0.50, side)), lambda t: (cowl + 0.15, deck + 0.2), expand=0.002, nu=12, nv=2)
    else:
        if not D['reduced']:
            for sx in (-1, 1):
                prim_box('Wiper' + ('L' if sx < 0 else 'R'), (0.02, 0.65 if cfg.get('bus') else 0.55, 0.012), (sx * 0.45, yF + (0.02 if cfg.get('bus') else 0.75), 1.30 if cfg.get('bus') else 1.62), M['trim'], rot=(math.radians(60) if cfg.get('bus') else math.radians(40), 0, math.radians(sx * -12)))
    # plates (anchors only; the runtime draws its registrations here)
    pz_f = (cfg['sill'] + 0.34) if not cfg.get('bus') else 0.75
    pz_r = (cfg['tailTop'] - 0.34) if not onebox else (1.05 if cfg.get('van') else 0.75)
    prim_box('PlateFront', (0.52, 0.01, 0.10), (0, yF - 0.006, pz_f), M['plate'])
    prim_box('PlateRear', (0.52, 0.01, 0.10), (0, yR + 0.004, pz_r), M['plate'])
    if not D['reduced']:
        for sx in ((-1, 1) if not cfg.get('bus') else (1,)):
            prim_cyl('Exhaust' + ('L' if sx < 0 else 'R'), 0.035, 0.14, (sx * (W * 0.30), yR - 0.06, cfg['sill'] + 0.13), 'Y', segs=max(8, D['cyl'] // 2), mat=M['chrome'])
            prim_cyl('ExhaustInner' + ('L' if sx < 0 else 'R'), 0.028, 0.15, (sx * (W * 0.30), yR - 0.06, cfg['sill'] + 0.13), 'Y', segs=max(8, D['cyl'] // 2), mat=M['trim'])
    bpy.data.objects.remove(shell0, do_unlink=True)
    return body

def shut_line_obj(BP, M, name, y, z0, z1, sx):
    bm = bmesh.new(); n = 12; vs = []
    for i in range(n):
        z = z0 + (z1 - z0) * i / (n - 1)
        x = body_surface_x(BP, y, z) + 0.001
        vs.append((bm.verts.new((sx * x, y - 0.0025, z)), bm.verts.new((sx * x, y + 0.0025, z))))
    for a, b in zip(vs, vs[1:]):
        bm.faces.new((a[0], a[1], b[1], b[0]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    ob = new_obj(name + ('L' if sx < 0 else 'R'), bm, M['shut'])
    s = ob.modifiers.new('Solid', 'SOLIDIFY'); s.thickness = 0.002
    return ob

def render(path, az_deg, el_deg, dist=7.5, target=(0, 0, 0.65), lens=50):
    sc = bpy.context.scene; cam = bpy.data.objects['Cam']; cam.data.lens = lens
    az, el = math.radians(az_deg), math.radians(el_deg)
    pos = Vector((dist * math.cos(el) * math.sin(az), -dist * math.cos(el) * math.cos(az), dist * math.sin(el) + target[2]))
    cam.location = pos
    cam.rotation_euler = (Vector(target) - pos).to_track_quat('-Z', 'Y').to_euler()
    sc.render.filepath = path
    bpy.ops.render.render(write_still=True)

def setup_review_scene():
    sc = bpy.context.scene
    for name in ('Cam', 'Sun', 'Fill', 'Ground'):
        if name in bpy.data.objects: continue
        if name == 'Cam':
            ob = bpy.data.objects.new('Cam', bpy.data.cameras.new('Cam')); sc.collection.objects.link(ob); sc.camera = ob
        elif name == 'Sun':
            ob = bpy.data.objects.new('Sun', bpy.data.lights.new('Sun', 'SUN')); sc.collection.objects.link(ob)
            ob.data.energy = 4.0; ob.rotation_euler = (math.radians(50), math.radians(15), math.radians(-35))
        elif name == 'Fill':
            ob = bpy.data.objects.new('Fill', bpy.data.lights.new('Fill', 'AREA')); sc.collection.objects.link(ob)
            ob.data.energy = 600; ob.data.size = 6; ob.location = (-5, 4, 4); ob.rotation_euler = (math.radians(55), 0, math.radians(-130))
        else:
            me = bpy.data.meshes.new('Ground'); bm = bmesh.new(); bmesh.ops.create_grid(bm, x_segments=1, y_segments=1, size=30); bm.to_mesh(me); bm.free()
            ob = bpy.data.objects.new('Ground', me); sc.collection.objects.link(ob)
            gm = bpy.data.materials.new('GroundMat'); gm.use_nodes = True
            gm.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value = (0.25, 0.25, 0.26, 1)
            gm.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value = 0.8; me.materials.append(gm)
    wd = sc.world or bpy.data.worlds.new('World'); sc.world = wd; wd.use_nodes = True
    bg = wd.node_tree.nodes.get('Background')
    if bg: bg.inputs['Color'].default_value = (0.55, 0.62, 0.72, 1); bg.inputs['Strength'].default_value = 0.6
    engines = [e.identifier for e in bpy.types.RenderSettings.bl_rna.properties['engine'].enum_items]
    sc.render.engine = 'BLENDER_EEVEE_NEXT' if 'BLENDER_EEVEE_NEXT' in engines else 'BLENDER_EEVEE'
    sc.render.resolution_x = 1280; sc.render.resolution_y = 720; sc.render.image_settings.file_format = 'PNG'

if __name__ == '__main__':
    import logging
    logging.getLogger('glTFExporter').setLevel(logging.WARNING)
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    OUT = argv[0] if argv else SP
    classes = (argv[1] if len(argv) > 1 else 'sedan').split(',')
    lods = [0, 1, 2]
    do_render = False
    for a in argv[2:]:
        if a.startswith('lods='): lods = [int(x) for x in a[5:].split(',')]
        if a == 'render': do_render = True
    os.makedirs(OUT, exist_ok=True)
    setup_review_scene()
    report = {}
    for cls in classes:
        for lod in lods:
            t0 = time.time()
            build(cls, lod)
            tris = tri_count()
            path = export_glb(f'{OUT}/{cls}-lod{lod}.glb')
            report[f'{cls}-lod{lod}'] = dict(triangles=tris, bytes=os.path.getsize(path), seconds=round(time.time() - t0, 1))
            print(f'[fleet] {cls} lod{lod}: {tris} tris, {os.path.getsize(path)} bytes, {time.time()-t0:.1f}s', flush=True)
            if do_render and lod == 0:
                dist = 7.5 * max(1.0, CLASSES[cls]['L'] / 4.85)
                render(f'{OUT}/{cls}_front34.png', -35, 12, dist=dist, target=(0, 0, CLASSES[cls]['H'] * 0.45))
                render(f'{OUT}/{cls}_rear34.png', 145, 12, dist=dist, target=(0, 0, CLASSES[cls]['H'] * 0.45))
            if do_render and lod == 2:
                render(f'{OUT}/{cls}_lod2_front34.png', -35, 12, dist=7.5 * max(1.0, CLASSES[cls]['L'] / 4.85), target=(0, 0, CLASSES[cls]['H'] * 0.45))
    with open(f'{OUT}/build-report.json', 'w') as f: json.dump(report, f, indent=1)
    print('[fleet] done', json.dumps(report), flush=True)
