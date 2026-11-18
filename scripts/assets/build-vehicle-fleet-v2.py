"""Deterministic authored fleet recipe. Run with Blender --background --python.

Arguments after --: --output DIR [--bodies sedan,wagon] [--render].
Source scenes and review renders stay outside the repository. Promotion is a
separate, validated step; this script never writes production assets.
"""
import argparse
import copy
import hashlib
import json
import math
import re
import shutil
import sys
from pathlib import Path

import bpy
from mathutils import Matrix, Vector

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'artifacts/vehicles/fleet-v1'))
import build_fleet as B
import fleet_lib as F
sys.path.insert(0, str(ROOT / 'artifacts/vehicles/project-sedan-v1'))
import build_sedan as S
import sedan_lib as SL
from headless_lod_lists import LOD1_DROP, LOD2_DROP, LOD2_KEEP, drop_parts

ROSTER = {
    'compactHatch': ('Metro hatch', 4.06, .315, ['compactHatch']),
    'sedan': ('Executive sedan', 4.72, .335, ['sedan']),
    'wagon': ('Touring estate', 4.76, .335, ['wagon']),
    'coupe': ('Sport coupe', 4.50, .345, ['coupe']),
    'compactSuv': ('Electric fastback crossover', 4.55, .36, ['compactSuv']),
    'pickup': ('Utility crew pickup', 5.89, .39, ['pickup']),
    'deliveryVan': ('High-roof cargo van', 5.53, .375, ['deliveryVan']),
    'boxTruck': ('Urban box truck', 7.62, .46, ['boxTruck']),
    'cityBus': ('Low-floor transit bus', 12.2, .53, ['cityBus']),
    'taxi': ('City taxi', 4.92, .345, ['taxi']),
    'supercar': ('Mid-engine supercar', 4.50, .35, ['supercar']),
    'roadster': ('Open roadster', 4.50, .34, ['roadster']),
}

# Trim variants of the highest-share bodies, and the two long-roof body styles
# the street had no silhouette for: (parent, label, lengthM, typeIds, trim).
# Each changes silhouette or finish with the shared material names only, so a
# variant costs geometry, never a new material graph. A 'platform' variant is
# its own class recipe built on the parent's footprint, wheelbase, track,
# wheel radius and height (the catalogue's `platformOf`), so it can share the
# parent's traffic slot without changing the simulation.
TRIMS = {
    'minivan': ('compactSuv', 'Compact minivan', 4.55, ['minivan'], 'platform'),
    'fullSizeSuv': ('pickup', 'Extended full-size utility', 5.89, ['fullSizeSuv'], 'platform'),
    'sedanSport': ('sedan', 'Executive sedan sport trim', 4.72, ['sedanSport'], 'sport'),
    'sedanLuxe': ('sedan', 'Executive sedan luxury trim', 4.72, ['sedanLuxe'], 'luxe'),
    'compactHatchSport': ('compactHatch', 'Metro hatch sport trim', 4.09, ['compactHatchSport'], 'sport'),
    'compactHatchCross': ('compactHatch', 'Metro hatch raised trim', 4.06, ['compactHatchCross'], 'cross'),
    'taxiCheckered': ('taxi', 'City taxi checker trim', 4.92, ['taxiCheckered'], 'checker'),
    'taxiAdBoard': ('taxi', 'City taxi roof-board trim', 4.92, ['taxiAdBoard'], 'adboard'),
}


def configure():
    B.CLASSES['wagon'] = dict(B._sedan_like(
        4.76, 1.83, 1.51, 2.79, .335, .225, -1.44, 1.35,
        .89, .73, 1.02, .18, -.86, -.05, 1.64, 2.30,
        roofHalf=.94, tumble=.22, tail_glass_len=.4), hatch=True, rails=True)
    B.CLASSES['coupe'] = dict(B._sedan_like(
        4.50, 1.85, 1.34, 2.72, .345, .235, -1.33, 1.39,
        .88, .76, .93, .17, -.57, .20, .91, 1.82,
        roofHalf=.90, tumble=.34), coupe=True)
    B.CLASSES['compactSuv'] = dict(B._sedan_like(
        4.55, 1.86, 1.68, 2.68, .36, .235, -1.355, 1.325,
        .98, .78, 1.03, .24, -.91, -.13, .95, 2.15,
        roofHalf=.94, tumble=.28, tail_glass_len=.6), suv=True, electric=True)
    # The cab and cargo box have separate silhouettes, not a stretched van.
    cfg = B._sedan_like(7.62, 2.26, 2.58, 4.32, .46, .30,
        -2.65, 1.67, 1.45, 1.29, .75, .46, -2.33, -1.65,
        -.95, -.55, roofHalf=.97, tumble=.12)
    cfg['body'] = [B.st(-3.81,.58,1.26,1.00), B.st(-3.64,.48,1.38,1.10),
        B.st(-2.8,.46,1.45,1.13), B.st(-.60,.46,1.45,1.13),
        B.st(-.48,.46,.68,1.03), B.st(3.70,.46,.68,1.03), B.st(3.81,.48,.68,1.03)]
    cfg.update(truck=True, pickup=True, track=1.98)
    B.CLASSES['boxTruck'] = cfg
    # A compact, tall passenger cabin distinguishes the taxi from the sedan.
    cfg = B.CLASSES['taxi']
    cfg['green'][2]['y'] -= .12
    cfg['green'][3]['y'] -= .12
    cfg['green'][5]['y'] += .14
    # SUPERCAR. Mid-engine proportions, which is what separates the class from
    # the coupe rather than styling: the cabin sits forward of centre (cowl
    # -.35 against the coupe's -.57), the run behind it is engine cover and not
    # a boot, and the roof is 140 mm below the coupe's at 1.20 m. Body 2.00 m
    # wide on a .80 roofHalf gives the wide-haunch / narrow-canopy read; the
    # coupe's .90 on 1.85 m cannot produce it at any length.
    B.CLASSES['supercar'] = dict(B._sedan_like(
        4.50, 2.00, 1.20, 2.65, .35, .265, -1.325, 1.325,
        .74, .60, .98, .14, -.35, .18, .70, 1.55,
        roofHalf=.80, tumble=.46), coupe=True)
    # ROADSTER. Front-engine and long-bonneted: the cowl sits 200 mm further
    # back than the supercar's, so the dash-to-axle run reads as engine ahead
    # of the driver, and the deck closes early behind a two-seat cabin. Sill
    # and belt stay near the coupe's because an open car keeps its door depth.
    B.CLASSES['roadster'] = dict(B._sedan_like(
        4.50, 1.95, 1.24, 2.55, .34, .245, -1.30, 1.25,
        .80, .68, .92, .16, -.15, .30, .95, 1.70,
        roofHalf=.86, tumble=.38), coupe=True)
    # Cleanly sampled surfaces: the near tier remains below the former fleet
    # triangle bill; no post-export decimation can damage panel boundaries.
    B.LOD[0].update(segs=48, step=.04, gsegs=40, gstep=.05, wsteps=40,
        cyl=24, nu=16, nv=8, lipsteps=32)
    B.LOD[1].update(segs=40, step=.06, gsegs=32, gstep=.06, wsteps=28,
        cyl=16, nu=12, nv=6, lipsteps=24)
    B.LOD[2].update(segs=16, step=.18, gsegs=12, gstep=.18, wsteps=12,
        cyl=8, nu=6, nv=4, lipsteps=10)
    configure_new_classes()


def configure_new_classes():
    # Long flat roof to a steep hatch, three glazed rows, on the crossover's
    # platform (4.55 x 1.86 x 1.68 m, 2.68 m wheelbase, 0.36 m wheels). The
    # declared B-pillar keeps a real front door length; the roof midpoint
    # would give long front doors.
    B.CLASSES['minivan'] = dict(B._sedan_like(
        4.55, 1.86, 1.68, 2.68, .36, .235, -1.355, 1.325,
        .96, .85, 1.04, .22, -.845, -.265, 1.805, 2.18,
        roofHalf=.95, tumble=.18, tail_glass_len=.5), hatch=True, rows3=True, minivan=True, bpillar=.14)
    # Upright three-row utility on the full-size pickup frame (5.89 x 2.03 x
    # 1.93 m, 3.68 m wheelbase, 0.39 m wheels): tall belt, near-vertical
    # tailgate glass, a long cargo bay where the pickup has its bed.
    B.CLASSES['fullSizeSuv'] = dict(B._sedan_like(
        5.89, 2.03, 1.93, 3.68, .39, .28, -1.995, 1.685,
        1.15, 1.10, 1.28, .36, -1.30, -.46, 2.16, 2.74,
        roofHalf=.96, tumble=.14, tail_glass_len=.45), suv=True, rails=True, rows3=True, fullSize=True, bpillar=.15)
    for key, (parent, _label, _length, _types, trim) in TRIMS.items():
        if parent == 'sedan' or trim == 'platform': continue
        cfg = copy.deepcopy(B.CLASSES[parent])
        cfg['trim'] = trim
        if trim == 'cross':
            cfg['rails'] = True
        if trim in ('adboard', 'checker'):
            # A taller or lower cabin roof: stations 2-5 are the roof itself.
            dz = .04 if trim == 'adboard' else -.02
            for station in cfg['green'][2:6]: station['top'] += dz
            cfg['H'] += dz
        B.CLASSES[key] = cfg


def bpillar(cfg):
    """Door split station: declared by long-roof classes, else the roof midpoint."""
    if 'bpillar' in cfg: return cfg['bpillar']
    return (cfg['roofFront'] + cfg['roofRear']) / 2 - .05 + (.10 if cfg.get('pickup') else 0)


def author_hatch_lid(cfg):
    """Give a hatch a Trunk the runtime can swing. Without this node the tailgate
    stays welded into Body, so the four side doors open and the hatch does not.
    The hinge sits on the roof seam. The painted tail is cut behind the rear
    doors so those panels stay on their own hinges. Glass, plate and tail lamps
    aft of that seam ride the lid. The bumper valance stays on the body.
    """
    if not cfg.get('hatch'):
        return
    body = bpy.data.objects.get('Body')
    if body is None or bpy.data.objects.get('Trunk') is not None:
        return
    door_end = min(cfg['deck'] - .08, bpillar(cfg) + 1.02)
    pivot = empty('Trunk', (0, cfg['roofRear'], cfg['H'] - .03))
    pivot['hingeAxis'] = 'X'
    pivot['openSign'] = 1
    pivot['openLimitRadians'] = 1.05
    y0, y1 = door_end + .04, cfg['L'] / 2 - .01
    z0, z1 = cfg['sill'] + .18, cfg['H'] + .02
    if y1 > y0 + .05 and z1 > z0:
        cut_panel(
            body, 'TrunkSkin',
            (cfg['W'] * .92, y1 - y0, z1 - z0),
            (0, (y0 + y1) / 2, (z0 + z1) / 2),
            pivot,
        )
    bpy.context.view_layer.update()
    for ob in list(F.root().all_objects):
        if ob == pivot or ob.parent is not None:
            continue
        aft = ob.matrix_world.translation.y > cfg['roofRear'] - .02
        if ob.name in ('RearWindow', 'PlateRear') or (ob.name.startswith('Taillamp') and aft):
            parent_keep(ob, pivot)
    skin = bpy.data.objects.get('TrunkSkin')
    print('HATCH_LID', 'skin_faces', 0 if skin is None else len(skin.data.polygons),
          'children', len(pivot.children), flush=True)


def parent_keep(ob, parent):
    bpy.context.view_layer.update()
    world = ob.matrix_world.copy()
    ob.parent = parent
    ob.matrix_world = world


def empty(name, location):
    ob = bpy.data.objects.new(name, None)
    F.link(ob)
    ob.location = location
    ob.empty_display_size = .12
    return ob


def cut_panel(shell, name, size, loc, pivot):
    """Extract actual shell geometry, leaving a matching open aperture."""
    panel = shell.copy()
    panel.data = shell.data.copy()
    F.link(panel)
    panel.name = name
    F.boolean_apply(panel, F.prim_box('PanelVolume', size, loc), 'INTERSECT')
    F.boolean_apply(shell, F.prim_box('PanelAperture', size, loc))
    if not len(panel.data.polygons):
        bpy.data.objects.remove(panel, do_unlink=True)
        return
    parent_keep(panel, pivot)
    return panel


def cabin(cfg, lod):
    """Continuous roof rails and glazing built in a common surface frame."""
    if cfg['kind']=='onebox': return
    for ob in list(F.root().all_objects):
        if ob.name.startswith(('Greenhouse','SideFront','SideRear','Windscreen','RearWindow','CabinLiner','CabinBlock','Wiper','Headrest','SeatR','RearShelf','Antenna','RoofRail','Handle','ShutDoor')):
            bpy.data.objects.remove(ob,do_unlink=True)
    M=F.materials(); W=cfg['W']; H=cfg['H']; belt=cfg['belt']-.045
    cowl,rf,rr,deck=[cfg[k] for k in ('cowl','roofFront','roofRear','deck')]
    gp=F.Profile(cfg['green'],n_top=cfg['gh_n'],n_bot=10,tumble=cfg['tumble'])
    def roof_z(y): return gp.at('top',y)
    def half(y): return W*.5*(.94-.16*max(0,min(1,(roof_z(y)-belt)/max(.15,H-belt))))
    def surf(name,fn,mat,nu=12,nv=5):
        import bmesh
        bm=bmesh.new(); rows=[]
        for j in range(nv+1): rows.append([bm.verts.new(fn(i/nu,j/nv)) for i in range(nu+1)])
        for a,b in zip(rows,rows[1:]):
            for i in range(nu): bm.faces.new((a[i],a[i+1],b[i+1],b[i]))
        bmesh.ops.recalc_face_normals(bm,faces=bm.faces)
        ob=F.new_obj(name,bm,mat)
        solid=ob.modifiers.new('PanelThickness','SOLIDIFY'); solid.thickness=.008
        return ob
    def roofpoint(y,q): return (q*half(y),y,roof_z(y)-.025*q*q)
    def strip(name,a,b,mat):
        return surf(name,lambda u,v:roofpoint(a+(b-a)*v,2*u-1),mat,16 if lod<2 else 8,12 if lod==0 else 5)
    strip('Windscreen',cowl+.045,rf-.035,M['glass'])
    strip('RoofPanel',rf,rr,M['paint'])
    strip('RearWindow',rr+.035,deck-.035,M['glass'])
    def beam(name,points,mat,radius):
        curve=bpy.data.curves.new(name,'CURVE');curve.dimensions='3D';curve.resolution_u=1
        curve.bevel_depth=radius;curve.bevel_resolution=2 if lod==0 else 0
        sp=curve.splines.new('POLY');sp.points.add(len(points)-1)
        for p,co in zip(sp.points,points): p.co=(*co,1)
        ob=bpy.data.objects.new(name,curve);F.link(ob);ob.data.materials.append(mat)
        bpy.context.view_layer.objects.active=ob;ob.select_set(True)
        bpy.ops.object.convert(target='MESH');ob.select_set(False)
        return ob
    mid=bpillar(cfg)
    for sx,tag in ((-1,'L'),(1,'R')):
        base=lambda y:(sx*W*.479,y,belt)
        top=lambda y:roofpoint(y,sx)
        # Four corner panes share the exact rail and belt endpoints. There is
        # no separate approximate glass loft to drift outside the aperture.
        panes=[('Front',cowl+.17,mid-.03,rf+.025,mid-.03),
               ('Rear',mid+.03,min(deck-.14,mid+1.0),mid+.03,rr-.035)]
        if cfg.get('coupe'):
            panes=[('Front',cowl+.17,min(cowl+1.31,deck-.25),rf+.03,rr-.10)]
        for row,a,b,ta,tb in panes:
            def win(u,v,a=a,b=b,ta=ta,tb=tb):
                low=Vector(base(a+(b-a)*u));high=Vector(top(ta+(tb-ta)*u))
                return low.lerp(high,v)
            surf('Side'+row+tag,win,M['glass'],10 if lod<2 else 4,5 if lod<2 else 2)
            beam('Side'+row+tag+'_Seal',[win(0,0),win(1,0),win(1,1),win(0,1),win(0,0)],M['trim'],.012)
        end=panes[-1][2]
        if deck-.10-end>.10:
            low0=Vector(base(end+.018));low1=Vector(base(deck-.10));high=Vector(top(rr))
            surf('QuarterGlass'+tag,lambda u,v:low0.lerp(low1,u).lerp(high,v),M['glass'],4,4)
        beam('PillarA'+tag,[base(cowl+.14),top(rf)],M['paint'],.025)
        if not cfg.get('coupe'):
            beam('PillarB'+tag,[base(mid),top(mid)],M['trim'],.024)
        beam('PillarC'+tag,[top(rr),base(deck-.10)],M['paint'],.04)
        beam('Cantrail'+tag,[top(cowl+(deck-cowl)*i/32) for i in range(33)],M['paint'],.023)
        beam('BeltMoulding'+tag,[base(cowl+.14),base(deck-.10)],M['trim'],.015)
        if cfg.get('rails'):
            for y in (rf+.12,rr-.12):
                p=Vector(roofpoint(y,sx*.84))
                beam('RailFoot'+tag,[p,p+Vector((0,0,.06))],M['trim'],.027)
            beam('RoofRail'+tag,[Vector(roofpoint(rf+.12+(rr-rf-.24)*i/20,sx*.84))+Vector((0,0,.06)) for i in range(21)],M['trim'],.021)
        for name,dx,dy in [('Mirror',.105,0),('MirrorArm',.025,0),('MirrorGlass',.105,.1)]:
            ob=bpy.data.objects.get(name+tag)
            if ob:
                ob.location.x=sx*(W*.5+dx)
                ob.location.y=cowl+.25+dy
                ob.location.z=belt+.10
    # Close the narrow windshield/roof and backlight/roof seal lines.
    for y in (cowl+.03,rf-.018,rr+.018,deck-.025):
        beam('GlassCrossSeal',[roofpoint(y,-1+2*i/16) for i in range(17)],M['trim'],.012)
    for ob in list(F.root().all_objects):
        if ob.name.startswith('SeatFB'):
            for v in ob.data.vertices: v.co.z*=.78


def articulate(cfg, lod):
    body = bpy.data.objects['Body']
    green = bpy.data.objects.get('Greenhouse')
    liner = bpy.data.objects.get('CabinLiner')
    if liner:
        bpy.data.objects.remove(liner, do_unlink=True)
    W, H, belt, sill = cfg['W'], cfg['H'], cfg['belt'], cfg['sill']
    if cfg.get('bus'):
        for ob in list(F.root().all_objects):
            if ob.name.startswith(('DoorFrame','Door-','Door0')): bpy.data.objects.remove(ob,do_unlink=True)
        F.boolean_apply(body,F.prim_box('FrontGlazingAperture',(W*.83,.55,1.48),
            (0,-cfg['L']/2+.12,2.05)))
        F.boolean_apply(body,F.prim_box('PassengerCabin',(W-.16,cfg['L']-.55,H-sill-.32),
            (0,0,(H+sill)/2+.01)))
        # The two passenger entrance leaves use the existing right-side keys.
        spans = [('FR', -cfg['L']/2 + .45, -cfg['L']/2 + 1.53), ('RR', .30, 1.38)]
    elif cfg['kind'] == 'onebox':
        a = -cfg['L']/2 + 1.43
        spans = [(key, a, a + 1.10) for key in ('FL','FR')]
    else:
        a = cfg['cowl'] + .16
        mid = bpillar(cfg)
        end = min(cfg['deck'] - .08, mid + 1.02)
        spans = [(key, a, mid-.015) for key in ('FL','FR')]
        if cfg.get('coupe'):
            spans = [(key, a, min(a+1.15, end)) for key in ('FL','FR')]
        elif not cfg.get('truck'):
            spans += [(key, mid+.015, end) for key in ('RL','RR')]
    # Hollow the cabin through the painted lower shell and greenhouse. The
    # remaining sidewall thickness is real, so opening a door reveals seats.
    a, b = min(s[1] for s in spans), max(s[2] for s in spans)
    if cfg.get('bus'): a, b = -cfg['L']/2+.35, cfg['L']/2-.3
    F.boolean_apply(body, F.prim_box('CabinVoid',
        (W-.16, b-a+.10, max(.15,belt-sill)), (0,(a+b)/2,(belt+sill)/2+.06)))
    if green:
        # Follow the raked roof rather than cutting a rectangular void through
        # the A/C pillars. Retain an offset inner skin under the entire loft.
        prof=F.Profile(cfg['green'],n_top=cfg['gh_n'],n_bot=10.0,tumble=cfg['tumble'],sigma=.04)
        inner=F.loft('RoofVoid',prof,bpy.data.materials['VehicleInterior'],
            segs=B.LOD[lod]['gsegs'],step=B.LOD[lod]['gstep'])
        inner.scale.x=.88
        for v in inner.data.vertices: v.co.z=belt+(v.co.z-belt)*.86-.025
        F.boolean_apply(green,inner)
    for key, y0, y1 in spans:
        sx = -1 if key.endswith('L') else 1
        pivot = empty('Door_'+key, (sx*(W/2-.035), y0, sill+.20))
        pivot['open_degrees'] = 0.0
        pivot['hinge_axis'] = 'Z (source), Y (runtime)'
        top=H-.32 if cfg.get('bus') else H
        size = (W*.38, y1-y0-.006, top-sill-.12)
        loc = (sx*W*.48, (y0+y1)/2, (top+sill)/2-.01)
        for shell in (body, green):
            if shell: cut_panel(shell, 'Panel_'+key+'_'+shell.name, size, loc, pivot)
        tag = 'L' if sx < 0 else 'R'
        source = ('SideFront' if key.startswith('F') else 'SideRear') + tag
        for ob in list(F.root().all_objects):
            if ob.name.startswith(source):
                if ob.name == source: ob.name = 'Window_'+key
                parent_keep(ob, pivot)
            elif key.startswith('F') and ob.name in ('Mirror'+tag,'MirrorArm'+tag,'MirrorGlass'+tag):
                parent_keep(ob, pivot)
        if not bpy.data.objects.get('Window_'+key):
            glass = F.prim_box('Window_'+key, (.014, (y1-y0)*.74, 1.5 if cfg.get('bus') else min(.58,H-belt-.10)),
                (sx*(W/2+.007),(y0+y1)/2,1.95 if cfg.get('bus') else belt+.32), bpy.data.materials['VehicleGlass'])
            parent_keep(glass, pivot)
        if lod < 2:
            card = F.prim_box('DoorCard_'+key, (.045,(y1-y0)*.80,max(.24,belt-sill-.20)),
                (sx*(W/2-.105),(y0+y1)/2,(belt+sill)/2), bpy.data.materials['VehicleInterior'], bevel=.015)
            for ay in (cfg['frontAxle'],cfg['rearAxle']):
                if y0-cfg['R']<ay<y1+cfg['R']:
                    F.boolean_apply(card,F.prim_cyl('CardArchClearance',cfg['R']+.065,W,
                        (sx*W/2,ay,cfg['R']),'X',segs=32 if lod==0 else 16))
            parent_keep(card,pivot)
            handle = F.prim_box('Handle_'+key,(.04,.15,.028),
                (sx*(W/2+.01),y1-.16,belt-.065),bpy.data.materials['VehicleChrome'],bevel=.009)
            parent_keep(handle,pivot)
        driver = pivot.driver_add('rotation_euler',2).driver
        var = driver.variables.new(); var.name='opening'; var.targets[0].id=pivot
        var.targets[0].data_path='["open_degrees"]'
        driver.expression=f'opening * {(-sx)*math.pi/180}'
    return [s[0] for s in spans]


def details(cfg, lod, index):
    M = F.materials()
    glass=M['glass'].node_tree.nodes['Principled BSDF']
    glass.inputs['Base Color'].default_value=(.025,.045,.055,1)
    glass.inputs['Alpha'].default_value=.68
    glass.inputs['Roughness'].default_value=.12
    W, L = cfg['W'], cfg['L']
    if cfg.get('truck'):
        F.prim_box('CargoBox',(2.44,4.28,2.51),(0,1.62,2.095),M['paint'],bevel=.025)
        F.prim_box('RollerShutter',(2.25,.022,2.18),(0,3.773,2.10),M['steel'])
        for j in range(16 if lod==0 else 8):
            F.prim_box('ShutterSlat'+str(j),(2.25,.028,.012),(0,3.79,1.08+j*(2.04/(15 if lod==0 else 7))),M['trim'])
        for sx in (-1,1):
            F.prim_box('CargoRail'+str(sx),(.035,4.28,.045),(sx*1.225,1.62,.86),M['chrome'])
    if cfg.get('electric'):
        for ob in list(F.root().all_objects):
            if ob.name.startswith(('Exhaust','GrilleSlat')): bpy.data.objects.remove(ob,do_unlink=True)
        F.prim_box('RearLightBridge',(W*.76,.012,.025),(0,L/2-.065,cfg['tailTop']-.075),M['tail'])
    if cfg.get('coupe'):
        F.prim_box('DeckLip',(W*.78,.10,.032),(0,L/2-.24,cfg['tailTop']+.01),M['paint'],bevel=.012)
    if lod == 0:
        # Split-spoke machining, centre bolts and brake ventilation remain
        # geometry and survive export without procedural shader dependencies.
        for key in ('FL','FR','RL','RR'):
            spin = bpy.data.objects.get('Wheel_'+key+'_Spin')
            if not spin: continue
            sx = -1 if key.endswith('L') else 1
            for j in range(5):
                a = j*math.tau/5
                ob=F.prim_cyl('Lug_'+key+str(j),.011,.017,
                    (sx*(cfg['WT']/2+.012),math.sin(a)*.045,math.cos(a)*.045),'X',segs=8,mat=M['chrome'])
                ob.parent=spin
            for j in range(10):
                a=j*math.tau/10+.07
                radius=cfg['R']*.46
                ob=F.prim_box('SpokeBlade_'+key+str(j),(.027,.020,cfg['R']*.49),
                    (sx*(cfg['WT']/2-.008),-math.sin(a)*radius,math.cos(a)*radius),M['alloy'],bevel=.004,bevel_segs=1)
                ob.rotation_euler.x=a
                ob.parent=spin
    # Merge opaque interior finishes into one slot; the game instantiates by
    # material, so visual details need not each add a draw call.
    collapse = {'VehicleHeadliner':'VehicleInterior','VehicleCarpet':'VehicleInterior',
        'VehicleSeat':'VehicleInterior','VehicleDash':'VehicleInterior',
        'VehicleShutLine':'VehicleTrimBlack','VehicleSteel':'VehicleAlloyDark',
        'VehicleTrimGloss':'VehicleTrimBlack'}
    for ob in F.root().all_objects:
        if ob.type!='MESH': continue
        for slot in ob.material_slots:
            if slot.material and slot.material.name in collapse:
                slot.material=bpy.data.materials[collapse[slot.material.name]]


def ring(name, major, minor, loc, rot, mat, nu, nv):
    """A torus in the XZ plane (axis Y), the frame of the generator's wheels."""
    import bmesh
    bm = bmesh.new(); rows = []
    for i in range(nu):
        a = math.tau * i / nu
        rows.append([bm.verts.new((math.cos(a) * (major + minor * math.cos(b)), minor * math.sin(b),
            math.sin(a) * (major + minor * math.cos(b)))) for b in (math.tau * j / nv for j in range(nv))])
    for i in range(nu):
        p, q = rows[i], rows[(i + 1) % nu]
        for j in range(nv): bm.faces.new((p[j], q[j], q[(j + 1) % nv], p[(j + 1) % nv]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    ob = F.new_obj(name, bm, mat); ob.location = loc; ob.rotation_euler = rot
    return ob


def polybeam(name, points, mat, radius, lod):
    curve = bpy.data.curves.new(name, 'CURVE'); curve.dimensions = '3D'; curve.resolution_u = 1
    curve.bevel_depth = radius; curve.bevel_resolution = 2 if lod == 0 else 0
    sp = curve.splines.new('POLY'); sp.points.add(len(points) - 1)
    for p, co in zip(sp.points, points): p.co = (*co, 1)
    ob = bpy.data.objects.new(name, curve); F.link(ob); ob.data.materials.append(mat)
    bpy.context.view_layer.objects.active = ob; ob.select_set(True)
    bpy.ops.object.convert(target='MESH'); ob.select_set(False)
    return ob


def join_objects(objects, name):
    objects = [ob for ob in objects if ob]
    if not objects: return None
    bpy.ops.object.select_all(action='DESELECT')
    for ob in objects: ob.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    bpy.ops.object.join()
    objects[0].name = name
    objects[0].select_set(False)
    return objects[0]


def cabin_kit(cfg, lod):
    """Readable cabin behind the glass: headrests, every seat row, a lit
    instrument cluster and screen, a spoked wheel, headliner and visors.

    The shared recipe left two seat backs and a dash in 0.012 trim black, so
    through the glass a cabin read as a black void. Every part here is named
    for the interior probe and uses VehicleInterior (one near-tier batch; the
    overlay's mid and far tiers fold it into trim black by name), VehicleDial
    (near tier only) or the existing trim black. Seat bases, headrests and
    the wheel carry the occupant solver's authored node names, so a seated
    figure's head sits against the headrest this kit places.
    """
    if lod >= 2: return
    near = lod == 0
    M = F.materials()
    I, T = M['interior'], M['trim']
    dial = F.mat('VehicleDial', (.02, .02, .025), rough=.2, emit=(.9, .9, 1.0), emit_str=.25)
    W, H, L = cfg['W'], cfg['H'], cfg['L']
    def box(name, size, loc, mat=I, bevel=0.0, segs=1, rot=(0, 0, 0)):
        return F.prim_box(name, size, loc, mat, bevel=bevel if near else 0.0, bevel_segs=segs, rot=rot)
    def cyl(name, r, depth, loc, axis='Y', mat=I, rot=None):
        ob = F.prim_cyl(name, r, depth, loc, axis, segs=16 if near else 8, mat=mat)
        if rot: ob.rotation_euler = rot
        return ob
    for ob in F.root().all_objects:
        # Door cards stay trim black: a lit card would add a batch to every
        # articulated door part for a surface seen edge-on through the glass.
        if ob.type == 'MESH' and ob.name.startswith('DoorCard_'):
            for slot in ob.material_slots: slot.material = T
    if cfg.get('bus'):
        bus_kit(cfg, lod, box, cyl, dial, T)
        return
    onebox = cfg['kind'] == 'onebox'
    if onebox:
        yF = -L / 2
        fr = dict(seat_y=yF + 2.0, seat_z=.95, seat_x=.42, cush=(.52, .55, .24), back_y=yF + 2.26, back_z=1.38,
                  back_h=.66, back_rot=-10, dash_back=yF + 1.525, dash_top=1.33, floor=.30,
                  roof=lambda y: H - .12, screen_top=yF + 1.22, bench=True)
        rows = []
    else:
        cowl, rf, rr, deck, belt, sill = (cfg[k] for k in ('cowl', 'roofFront', 'roofRear', 'deck', 'belt', 'sill'))
        gp = F.Profile(cfg['green'], n_top=cfg['gh_n'], n_bot=10, tumble=cfg['tumble'])
        seatz = sill + .25
        fr = dict(seat_y=cowl + 1.05, seat_z=seatz + .11, seat_x=.38, cush=(.50, .52, .22), back_y=cowl + 1.30,
                  back_z=seatz + .50, back_h=.62 * .78, back_rot=-12, dash_back=cowl + .55, dash_top=belt - .01,
                  floor=sill + .05, roof=lambda y: gp.at('top', y), screen_top=rf, bench=bool(cfg.get('truck')))
        limit = deck - .40
        if cfg.get('truck'): rows = []
        elif cfg.get('rows3'): rows = [y for y in (fr['back_y'] + .70, fr['back_y'] + 1.52) if y <= limit]
        else: rows = [min(max(rr - .10, fr['back_y'] + .62), limit)]
    # Front row: authored names on the recipe's own cushions.
    for tag, key in (('L', 'FL'), ('R', 'FR')):
        ob = bpy.data.objects.get('SeatF' + tag)
        if ob: ob.name = f'Seat_{key}_Base'
    steer = bpy.data.objects.get('Steering')
    wheel_loc, wheel_rot = (steer.location.copy(), steer.rotation_euler.copy()) if steer else (Vector((-.38, fr['dash_back'] + .13, fr['dash_top'] + .13)), Vector((math.radians(-25), 0, 0)))
    if steer and near:
        bpy.data.objects.remove(steer, do_unlink=True)
        ring('SteeringWheel', .185, .016, wheel_loc, wheel_rot, T, 28, 8)
    elif steer:
        steer.name = 'SteeringWheel'
    rot = math.radians(fr['back_rot'])
    top_y = fr['back_y'] + fr['back_h'] / 2 * math.sin(-rot)
    top_z = fr['back_z'] + fr['back_h'] / 2 * math.cos(rot)
    for sx, key in ((-1, 'FL'), (1, 'FR')):
        x = sx * fr['seat_x']
        existing = bpy.data.objects.get('Headrest' + ('L' if sx < 0 else 'R'))
        if existing:
            existing.name = f'Seat_{key}_Headrest0'
            head = existing.location.copy()
        else:
            head = Vector((x, top_y + .03, min(top_z + .10, fr['roof'](top_y + .03) - .10)))
            box(f'Seat_{key}_Headrest0', (.25, .09, .16), head, bevel=.025, segs=2)
        if near:
            for i, px in enumerate((-.06, .06)):
                box(f'SeatHeadrestPost{key}{i}', (.012, .012, .10), (head.x + px, head.y - .01, head.z - .11), mat=T)
            for i, px in enumerate((-.22, .22)):
                box(f'SeatBolster{key}{i}', (.07, fr['cush'][1] * .92, .07), (x + px, fr['seat_y'], fr['seat_z'] + fr['cush'][2] / 2 + .02), bevel=.02)
                box(f'SeatBackBolster{key}{i}', (.07, .10, fr['back_h'] * .86), (x + px, fr['back_y'] - .03, fr['back_z']), bevel=.02, rot=(rot, 0, 0))
            box(f'SeatBelt{key}', (.045, .008, fr['back_h'] * .95), (x - sx * .10, fr['back_y'] - .075, fr['back_z']), mat=T, rot=(rot, math.radians(sx * 32), 0))
    if fr['bench']:
        cw = max(.18, 2 * (fr['seat_x'] - fr['cush'][0] / 2) - .02)
        box('SeatCentre', (cw, fr['cush'][1] * .9, fr['cush'][2] * .9), (0, fr['seat_y'], fr['seat_z']), bevel=.025, segs=2)
        box('SeatCentreBack', (cw, .10, fr['back_h'] * .8), (0, fr['back_y'], fr['back_z'] - .04), bevel=.02, rot=(rot, 0, 0))
    # Instrument panel: hood, lit dials and screen on the driver's side.
    wx, wy, wz = wheel_loc
    db, dt = fr['dash_back'], fr['dash_top']
    box('DashCluster', (.36, .14, .09), (wx, db - .05, dt + .045), bevel=.02)
    if near:
        cab_w = W - .34 if onebox else W * .80
        box('DashTopPad', (cab_w * .9, .20, .04), (0, db - .14, dt + .01), bevel=.015)
        for i, dx in enumerate((-.08, .08)):
            cyl('DashDial' + 'LR'[i], .045, .012, (wx + dx, db + .027, dt + .045), 'Y', mat=dial)
        box('DashScreen', (.24, .012, .14), (0, db + .012, dt + .02), mat=dial, rot=(math.radians(-12), 0, 0))
        for i, dx in enumerate((-.30, .30)):
            box('DashVent' + 'LR'[i], (.12, .02, .05), (dx, db + .008, dt - .07), mat=T)
        box('DashGloveBox', (.40, .02, .15), (-wx, db + .008, dt - .17), bevel=.006)
        tilt = Matrix.Rotation(wheel_rot[0], 3, 'X')
        cyl('SteeringHub', .055, .05, (wx, wy + .01, wz), 'Y', rot=(wheel_rot[0], 0, 0))
        for i, (dx, dz, size) in enumerate(((-.09, 0, (.14, .018, .03)), (.09, 0, (.14, .018, .03)), (0, -.09, (.03, .018, .14)))):
            off = tilt @ Vector((dx, 0, dz))
            box(f'SteeringSpoke{i}', size, (wx + off.x, wy + off.y, wz + off.z), rot=(wheel_rot[0], 0, 0))
        off = tilt @ Vector((0, -.17, -.02))
        cyl('SteeringColumn', .03, .30, (wx + off.x, wy + off.y, wz + off.z), 'Y', mat=T, rot=(wheel_rot[0], 0, 0))
        for name, dx in (('PedalAccel', .14), ('PedalStop', -.02)):
            box(name, (.05, .02, .09), (wx + dx, db + .10, fr['floor'] + .14), mat=T)
        if not onebox:
            cz = sill + .40
            cyl('ConsoleShifter', .018, .12, (0, cowl + .78, cz + .06), 'Z', mat=T)
            box('ConsoleArmrest', (.20, .30, .07), (0, cowl + 1.32, cz + .10), bevel=.02)
            box('ConsoleCupholder', (.16, .10, .012), (0, cowl + .98, cz + .004), mat=T)
        zt = fr['roof'](fr['screen_top'] + .06) - .06
        for sx in (-1, 1):
            box('InteriorSunVisor' + ('L' if sx < 0 else 'R'), (.34, .15, .014), (sx * .33, fr['screen_top'] + .10, zt), rot=(math.radians(-18), 0, 0))
        box('InteriorMirror', (.20, .03, .06), (0, fr['screen_top'] + .05, zt - .05), mat=T)
    if onebox:
        box('InteriorHeadliner', (W - .34, 1.4, .02), (0, -L / 2 + 1.95, H - .14))
    else:
        y0, y1 = rf + .05, (deck - .05 if cfg.get('truck') else rr - .05)
        zh = min(fr['roof'](y) for y in (y0, (y0 + y1) / 2, y1)) - .045
        box('InteriorHeadliner', (W * .62, max(.2, y1 - y0), .02), (0, (y0 + y1) / 2, zh))
    # Rear rows. The first carries the occupant solver's bench names.
    width = min(1.30, W * .72)
    brot = math.radians(-16)
    for r, ry in enumerate(rows):
        name = 'Seat_Rear' if r == 0 else f'SeatRow{r + 2}'
        box(f'{name}_Base', (width, .48, .20), (0, ry, seatz + .11), bevel=.03, segs=2)
        bh = max(.30, min(.56, 2 * (fr['roof'](ry + .30) - .12 - (seatz + .26))))
        bz = seatz + .26 + bh / 2
        box(f'{name}_Back', (width, .12, bh), (0, ry + .25, bz), bevel=.03, segs=2, rot=(brot, 0, 0))
        ty, tz = ry + .25 + bh / 2 * math.sin(-brot), bz + bh / 2 * math.cos(brot)
        heads = [(0, -.40), (1, .40)] + ([(2, 0.0)] if near else [])
        for i, hx in heads:
            hz = min(tz + .09, fr['roof'](ty + .03) - .09)
            box(f'{name}_Headrest{i}', (.24 if hx else .20, .09, .14), (hx, ty + .03, hz), bevel=.025, segs=2)
        if near:
            box(f'{name}Armrest', (.22, .30, .07), (0, ry - .02, seatz + .25), bevel=.02)
            for i, px in enumerate((-1, 1)):
                box(f'{name}Bolster{i}', (.08, .44, .06), (px * (width / 2 - .06), ry, seatz + .24), bevel=.02)
    if cfg.get('truck') and near:
        box('InteriorCabBack', (W * .84, .03, belt + .45 - sill), (0, deck + .02, (belt + .45 + sill) / 2 + .05))


def bus_kit(cfg, lod, box, cyl, dial, T):
    near = lod == 0
    W, H, L = cfg['W'], cfg['H'], cfg['L']
    yF = -L / 2
    alloy = bpy.data.materials['VehicleAlloy']
    for k in range(6):
        y = yF + 2.2 + k * 1.4
        for sx in (-1, 1):
            box(f'SeatRowBack{k}' + ('L' if sx < 0 else 'R'), (W * .36, .08, .42), (sx * W * .21, y + .19, 1.86), bevel=.02, rot=(math.radians(-8), 0, 0))
        if near:
            cyl(f'InteriorPole{k}', .02, 2.0, (W * .41, y + .62, 1.75), 'Z', mat=alloy)
    # Not the occupant solver's names: a bus front row is one driver's seat, and
    # the solver's authored path expects two, so the bus keeps the derived solve.
    box('SeatDriverBase', (.50, .50, .22), (-.75, yF + 1.35, 1.02), bevel=.03, segs=2)
    box('SeatDriverBack', (.50, .10, .62), (-.75, yF + 1.62, 1.40), bevel=.03, segs=2, rot=(math.radians(-10), 0, 0))
    box('SeatDriverHeadrest', (.26, .09, .16), (-.75, yF + 1.68, 1.82), bevel=.025, segs=2)
    box('DashBus', (W * .78, .40, .30), (0, yF + .55, 1.15), bevel=.02)
    box('DashCluster', (.40, .12, .10), (-.75, yF + .78, 1.34), bevel=.02)
    cyl('SteeringBusWheel', .23, .03, (-.75, yF + .98, 1.42), 'Y', mat=T, rot=(math.radians(-55), 0, 0))
    if near:
        for i, dx in enumerate((-.08, .08)):
            cyl('DashDial' + 'LR'[i], .05, .012, (-.75 + dx, yF + .845, 1.34), 'Y', mat=dial)
    box('InteriorHeadliner', (W - .24, L - 1.2, .02), (0, 0, H - .18))


def cabin_finish():
    """Interior tone, set after details(): luminance 0.069, between the 0.012
    trim black (a void) and the 0.10 seat insert of the authored sedan."""
    F.mat('VehicleInterior', (.072, .068, .064), rough=.88)


def trim_details(cfg, lod):
    trim = cfg.get('trim')
    if not trim: return
    mats = bpy.data.materials
    T, P, A = mats['VehicleTrimBlack'], mats['VehiclePaint'], mats['VehicleAlloy']
    W, H, L, R = cfg['W'], cfg['H'], cfg['L'], cfg['R']
    cowl, rf, rr, deck, belt, sill = (cfg[k] for k in ('cowl', 'roofFront', 'roofRear', 'deck', 'belt', 'sill'))
    BP = F.Profile(cfg['body'], n_top=4.2, n_bot=6.0, sigma=0.05, n_top_fn=cfg.get('n_top_fn'))
    gp = F.Profile(cfg['green'], n_top=cfg['gh_n'], n_bot=10, tumble=cfg['tumble'])
    bev = .012 if lod == 0 else 0
    if trim == 'sport':
        # Black floating roof, roof spoiler, skirts, splitter, rear diffuser
        # (the diffuser is why the declared length grows by 3 cm).
        for ob in F.root().all_objects:
            if ob.type == 'MESH' and ob.name.startswith(('RoofPanel', 'Cantrail', 'PillarC', 'Antenna')):
                for slot in ob.material_slots:
                    if slot.material == P: slot.material = T
        F.prim_box('RoofSpoiler', (W * .74, .24, .035), (0, rr + .10, gp.at('top', rr) - .005), P, bevel=bev, rot=(math.radians(8), 0, 0))
        ys = (cfg['frontAxle'] + R + .12, cfg['rearAxle'] - R - .12)
        for sx, tag in ((-1, 'L'), (1, 'R')):
            yc = (ys[0] + ys[1]) / 2
            x = F.body_surface_x(BP, yc, sill + .06)
            F.prim_box('SideSkirt' + tag, (.05, ys[1] - ys[0], .07), (sx * (x + .012), yc, sill + .02), T, bevel=bev * .5)
        F.prim_box('FrontSplitter', (W * .78, .16, .025), (0, -L / 2 + .10, sill + .10), T)
        F.prim_box('RearDiffuser', (W * .62, .12, .06), (0, L / 2 + .01, sill + .14), T, bevel=bev * .5)
    elif trim == 'cross':
        # Raised trim: roof rails (cfg), black arch and sill cladding, skid plates.
        steps = 9 if lod == 0 else 5
        for k, ay in enumerate((cfg['frontAxle'], cfg['rearAxle'])):
            for sx, tag in ((-1, 'L'), (1, 'R')):
                pts = []
                for i in range(steps):
                    a = math.radians(12 + 156 * i / (steps - 1))
                    y = ay - math.cos(a) * (R + .085); z = R + math.sin(a) * (R + .085)
                    pts.append(Vector((sx * (F.body_surface_x(BP, y, z) + .004), y, z)))
                polybeam(f'ArchClad{tag}{k}', pts, T, .03, lod)
            ya, yb = cfg['frontAxle'] + R + .10, cfg['rearAxle'] - R - .10
        for sx, tag in ((-1, 'L'), (1, 'R')):
            pts = [Vector((sx * (F.body_surface_x(BP, y, sill + .08) + .004), y, sill + .08)) for y in (ya + (yb - ya) * i / 6 for i in range(7))]
            polybeam('SillClad' + tag, pts, T, .035, lod)
        F.prim_box('SkidPlateF', (W * .46, .03, .10), (0, -L / 2 + .04, sill + .14), A, bevel=bev * .5)
        F.prim_box('SkidPlateR', (W * .40, .03, .08), (0, L / 2 - .04, sill + .16), A, bevel=bev * .5)
        # 25 mm more ride height: every root part except the wheel pivots rises,
        # which is the height the catalogue declares for this trim (1.50 m).
        for ob in list(F.root().objects):
            if ob.parent is None and not ob.name.startswith('Wheel_'): ob.location.z += .025
    elif trim == 'adboard':
        # A lit two-sided roof advertising board on a taller cab.
        for name in ('TaxiSign', 'TaxiSignBase'):
            ob = bpy.data.objects.get(name)
            if ob: bpy.data.objects.remove(ob, do_unlink=True)
        sign = mats['VehicleDRL'] if lod == 2 else mats['VehicleSign']
        ym = (rf + rr) / 2
        F.prim_box('TaxiAdBoard', (.12, 1.02, .34), (0, ym, H + .25), sign, bevel=bev)
        F.prim_box('TaxiAdBoardFrame', (.14, 1.06, .035), (0, ym, H + .065), T)
        for dy, tag in ((-.36, 'F'), (.36, 'R')):
            F.prim_box('TaxiAdMount' + tag, (.08, .06, .06), (0, ym + dy, H + .02), T)
    elif trim == 'checker':
        # Two-row checker band in place of the plain band, a taller sign.
        for ob in list(F.root().all_objects):
            if ob.name.startswith('LiveryBand'): bpy.data.objects.remove(ob, do_unlink=True)
        sign = bpy.data.objects.get('TaxiSign')
        if sign:
            sign.scale = (1.18, 1.10, 1.45); sign.location.z += .03
        if lod < 2:
            dark = mats['VehicleLiveryDark']
            rows, step = (2, .10) if lod == 0 else (1, .16)
            n = int((deck + .2 - (cowl + .15)) / step)
            for sx, tag in ((1, 'R'), (-1, 'L')):
                squares = []
                for r in range(rows):
                    zf0 = .40 + .05 * r
                    for k in range(n):
                        if (k + r) % 2: continue
                        y0 = cowl + .15 + k * step
                        squares.append(F.patch(f'TaxiChecker{tag}{r}_{k}', BP, dark,
                            (BP.t_for_zf(zf0, sx), BP.t_for_zf(zf0 + .05, sx)),
                            lambda t, y0=y0: (y0, y0 + step), expand=.002, nu=2, nv=2))
                join_objects(squares, 'TaxiChecker' + tag)


def sedan(lod):
    # The established sedan's tiny fuel flap cannot be re-cut at low loft
    # resolution. Derive its distance tiers from the validated near mesh,
    # retaining the named pivots, as the existing sedan pipeline does.
    if lod==0:
        SL.set_quality(0);S.build();S.pose(False)
    else:
        drop_parts(LOD1_DROP if lod==1 else LOD2_DROP,() if lod==1 else LOD2_KEEP)
        for ob in SL.root().all_objects:
            if ob.type!='MESH' or len(ob.data.polygons)<50: continue
            mod=ob.modifiers.new('DistanceReduction','DECIMATE');mod.ratio=.12 if lod==1 else .25
            bpy.context.view_layer.objects.active=ob
            bpy.ops.object.modifier_apply(modifier=mod.name)
    # Retain the proven cabin rig; revise the exterior with a continuous
    # deck accent and machined sill finish at every detail level.
    M=F.materials()
    if lod==0:
        accent=F.prim_box('DeckAccent',(1.12,.012,.022),(0,2.22,.85),M['chrome'])
        SL.root().objects.link(accent);F.root().objects.unlink(accent)
    return ['FL','FR','RL','RR'] if lod<2 else []


def finish_envelope(lib, length):
    # Boolean lens shells in the original libraries retain cutter end faces
    # beyond the fascia. Finish every mesh against the declared bumper planes
    # so a lamp housing cannot silently lengthen the collision silhouette.
    bpy.context.view_layer.update()
    for ob in lib.root().all_objects:
        if ob.type!='MESH': continue
        world=ob.matrix_world.copy();inverse=world.inverted()
        for vertex in ob.data.vertices:
            p=world@vertex.co
            if abs(p.y)>length/2+.02:
                p.y=max(-length/2-.02,min(length/2+.02,p.y))
                vertex.co=inverse@p
        ob.data.update()


def finish_budget(lib, lod):
    if lod==0: return
    # Reduce evaluated geometry, including small bevelled fittings. Counting
    # only their control cages underestimates the exported triangle bill.
    dg=bpy.context.evaluated_depsgraph_get()
    for ob in lib.root().all_objects:
        if ob.type!='MESH': continue
        mesh=bpy.data.meshes.new_from_object(ob.evaluated_get(dg))
        ob.modifiers.clear();ob.data=mesh
    cap=20000 if lod==1 else 5300
    if lib.tri_count()<=cap: return
    reductions=[]
    for ob in lib.root().all_objects:
        if ob.type=='MESH' and len(ob.data.polygons)>24:
            mod=ob.modifiers.new('ExportBudget','DECIMATE')
            reductions.append((ob,mod))
    for ratio in (.95,.90,.85,.80,.75,.70,.65,.60,.55,.50,.45,.40,.35,.30,.25,.20):
        for ob,mod in reductions: mod.ratio=ratio
        bpy.context.view_layer.update()
        if lib.tri_count()<=cap: break
    if lib.tri_count()>cap: raise ValueError('Export cannot meet existing fleet budget')
    for ob,mod in reductions:
        bpy.context.view_layer.objects.active=ob
        bpy.ops.object.modifier_apply(modifier=mod.name)


def patrol_variant(out, report):
    """Equipment/livery variant of one body, preserving the patrol identity."""
    entry={'label':'Patrol sedan equipment variant','lengthM':4.72,'wheelRadiusM':.335,
        'typeIds':['patrolSedan'],'lods':[]}
    for lod in range(3):
        F.clear()
        for material in list(bpy.data.materials):
            if material.users==0: bpy.data.materials.remove(material)
        before=set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=str(out/f'sedan-lod{lod}.glb'))
        for ob in set(bpy.data.objects)-before:
            if ob.name not in F.root().objects: F.root().objects.link(ob)
        trim=bpy.data.materials.get('VehicleTrimBlack') or F.mat('VehicleTrimBlack',(.012,.014,.016),rough=.36)
        for ob in F.root().all_objects:
            if ob.type=='MESH' and ob.name.startswith(('Body','Hood','Trunk','Roof')):
                for slot in ob.material_slots:
                    if slot.material and slot.material.name=='VehiclePaint': slot.material=trim
        red=F.mat('VehicleEmergencyRed',(.45,.005,.006),rough=.2,emit=(1,.005,.006),emit_str=.5)
        blue=F.mat('VehicleEmergencyBlue',(.006,.025,.5),rough=.2,emit=(.006,.04,1),emit_str=.5)
        F.prim_box('EmergencyBarBase',(1.05,.22,.045),(0,.15,1.475),trim,bevel=.015)
        F.prim_box('EmergencyBarRed',(.47,.19,.09),(-.27,.15,1.53),red,bevel=.015)
        F.prim_box('EmergencyBarBlue',(.47,.19,.09),(.27,.15,1.53),blue,bevel=.015)
        for sx in (-1,1):
            F.prim_box('PushBarUpright'+str(sx),(.035,.05,.31),(sx*.38,-2.38,.50),trim,bevel=.012)
        F.prim_box('PushBarCross',(.80,.06,.045),(0,-2.38,.54),trim,bevel=.012)
        file=out/f'patrolSedan-lod{lod}.glb'
        F.export_glb(str(file))
        entry['lods'].append({'file':file.name,'sha256':hashlib.sha256(file.read_bytes()).hexdigest(),
            'bytes':file.stat().st_size,'triangles':F.tri_count(),'doors':['FL','FR','RL','RR'] if lod<2 else []})
    report['bodies']['sedan']['typeIds']=['sedan']
    report['bodies']['sedan'].setdefault('variants',{})['patrolSedan']=entry
    (out/'build-report.json').write_text(json.dumps(report,indent=2)+'\n')


def unify_material_names():
    for ob in F.root().all_objects:
        if ob.type != 'MESH': continue
        for slot in ob.material_slots:
            m = slot.material
            if m and re.search(r'\.\d+$', m.name):
                base = bpy.data.materials.get(re.sub(r'\.\d+$', '', m.name))
                if base: slot.material = base


def hit_on(ob, origin, direction):
    """World hit point of a ray against one mesh, or None."""
    if ob is None or ob.type != 'MESH': return None
    inv = ob.matrix_world.inverted()
    ok, loc, _normal, _index = ob.ray_cast(inv @ Vector(origin), (inv.to_3x3() @ Vector(direction)).normalized())
    return ob.matrix_world @ loc if ok else None


def sedan_trim(out, key):
    """Trim variant of the established sedan, derived from its exported tiers
    like the patrol variant: the validated LOD0 is never rebuilt."""
    _parent, label, length, types, trim = TRIMS[key]
    entry = {'label': label, 'lengthM': length, 'wheelRadiusM': ROSTER['sedan'][2], 'typeIds': types, 'lods': []}
    for lod in range(3):
        F.clear()
        for material in list(bpy.data.materials):
            if material.users == 0: bpy.data.materials.remove(material)
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=str(out / f'sedan-lod{lod}.glb'))
        for ob in set(bpy.data.objects) - before:
            if ob.name not in F.root().objects: F.root().objects.link(ob)
        unify_material_names()
        bpy.context.view_layer.update()
        mats = bpy.data.materials
        T = mats.get('VehicleTrimBlack') or F.mat('VehicleTrimBlack', (.012, .012, .013), rough=.55)
        A = mats.get('VehicleAlloy') or F.mat('VehicleAlloy', (.75, .76, .78), metallic=1.0, rough=.2)
        P, G = mats['VehiclePaint'], mats['VehicleGlass']
        body = bpy.data.objects.get('Body')
        bev = .01 if lod == 0 else 0
        if trim == 'sport':
            # 15 mm lower on its springs: every root part except the wheel pivots.
            for ob in list(F.root().all_objects):
                if ob.parent is None and not ob.name.startswith('Wheel_'): ob.location.z -= .015
            bpy.context.view_layer.update()
            if body:
                # Gloss-black roof: the body's own roof faces change finish.
                if T.name not in [s.material.name for s in body.material_slots if s.material]:
                    body.data.materials.append(T)
                ti = next(i for i, s in enumerate(body.material_slots) if s.material == T)
                mw = body.matrix_world; nm = mw.to_3x3()
                for poly in body.data.polygons:
                    if body.material_slots[poly.material_index].material != P: continue
                    if (mw @ poly.center).z > 1.32 and (nm @ poly.normal).normalized().z > .55:
                        poly.material_index = ti
            top = hit_on(bpy.data.objects.get('Trunk') or body, (0, 2.08, 3), (0, 0, -1)) or Vector((0, 2.08, .94))
            F.prim_box('DeckSpoiler', (1.30, .12, .028), (0, 2.10, top.z + .055), P, bevel=bev, rot=(math.radians(-5), 0, 0))
            for sx in (-1, 1):
                tag = 'L' if sx < 0 else 'R'
                F.prim_box('DeckSpoilerStand' + tag, (.03, .06, .05), (sx * .50, 2.09, top.z + .02), P)
                xs = [p.x for p in (hit_on(body, (sx * 1.5, y, .27), (-sx, 0, 0)) for y in (-.8, 0, .8)) if p]
                x = (max(xs) if sx > 0 else min(xs)) if xs else sx * .90
                F.prim_box('SideSkirt' + tag, (.05, 1.80, .06), (x + sx * .01, 0, .20), T, bevel=bev * .5)
            front = hit_on(body, (0, -2.25, -1), (0, 0, 1))
            F.prim_box('FrontSplitter', (1.40, .16, .02), (0, -2.26, (front.z if front else .20) - .012), T)
        elif trim == 'luxe':
            # Glass sunroof, chrome grille surround, chrome belt and sill strips.
            roof = [p for p in (hit_on(body, (x, .15, 3), (0, 0, -1)) for x in (0, .36, -.36)) if p]
            zr = max(p.z for p in roof) if roof else 1.44
            F.prim_box('SunroofFrame', (.76, .86, .012), (0, .15, zr + .004), T)
            F.prim_box('SunroofGlass', (.68, .78, .01), (0, .15, zr + .010), G)
            for stem, z in (('Top', .655), ('Bottom', .415)):
                F.prim_box('GrilleSurround' + stem, (.86, .024, .03), (0, -2.372, z), A, bevel=bev * .5)
            for sx in (-1, 1):
                F.prim_box('GrilleSurround' + ('L' if sx < 0 else 'R'), (.03, .024, .27), (sx * .43, -2.372, .535), A, bevel=bev * .5)
            for door_key in (('FL', 'FR', 'RL', 'RR') if lod < 2 else ()):
                door = bpy.data.objects.get('Door_' + door_key)
                if not door: continue
                window = bpy.data.objects.get('Window_' + door_key)
                sx = -1 if door_key.endswith('L') else 1
                corners = [door.matrix_world @ Vector(c) for c in door.bound_box]
                y0, y1 = min(c.y for c in corners) + .05, max(c.y for c in corners) - .05
                wz = min((window.matrix_world @ Vector(c)).z for c in window.bound_box) if window else .86
                for stem, z, size in (('Belt', wz - .012, (.012, y1 - y0, .016)), ('Sill', .33, (.012, y1 - y0, .022))):
                    p = hit_on(door, (sx * 1.5, (y0 + y1) / 2, z), (-sx, 0, 0))
                    if not p: continue
                    strip = F.prim_box(f'Chrome{stem}_{door_key}', size, (p.x + sx * .005, (y0 + y1) / 2, z), A)
                    parent_keep(strip, door)
        bpy.context.view_layer.update()
        file = out / f'{key}-lod{lod}.glb'
        F.export_glb(str(file))
        entry['lods'].append({'file': file.name, 'sha256': hashlib.sha256(file.read_bytes()).hexdigest(),
            'bytes': file.stat().st_size, 'triangles': F.tri_count(), 'doors': ['FL', 'FR', 'RL', 'RR'] if lod < 2 else []})
        print('FLEET', key, lod, entry['lods'][-1], flush=True)
    return entry


def build_body(key, lod):
    B.build(key, lod)
    cfg = B.CLASSES[key]
    cabin(cfg, lod)
    doors = articulate(cfg, lod)
    cabin_kit(cfg, lod)
    details(cfg, lod, 0)
    trim_details(cfg, lod)
    cabin_finish()
    author_hatch_lid(cfg)
    return doors


def build_trim(out, key):
    parent, label, length, types, _trim = TRIMS[key]
    entry = {'label': label, 'lengthM': length, 'wheelRadiusM': ROSTER[parent][2], 'typeIds': types, 'lods': []}
    for lod in range(3):
        doors = build_body(key, lod)
        finish_envelope(F, length)
        finish_budget(F, lod)
        bpy.context.view_layer.update()
        file = out / f'{key}-lod{lod}.glb'
        F.export_glb(str(file))
        entry['lods'].append({'file': file.name, 'sha256': hashlib.sha256(file.read_bytes()).hexdigest(),
            'bytes': file.stat().st_size, 'triangles': F.tri_count(), 'doors': doors})
        print('FLEET', key, lod, entry['lods'][-1], flush=True)
    return entry


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--output',required=True)
    parser.add_argument('--bodies',default=','.join([*ROSTER,*TRIMS]))
    parser.add_argument('--render',action='store_true')
    parser.add_argument('--variants-only',action='store_true')
    # Start from an existing (already packed) build: its receipts and exports are
    # copied, and only --bodies are regenerated. Used so the sedan's validated
    # LOD0 is carried byte-for-byte instead of being rebuilt.
    parser.add_argument('--seed-from',default=None)
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:])
    out=Path(args.output).expanduser().resolve()
    if out.is_relative_to(ROOT): raise ValueError('Candidate binaries must be outside the repository')
    out.mkdir(parents=True,exist_ok=True)
    if args.variants_only:
        patrol_variant(out,json.loads((out/'build-report.json').read_text()))
        return
    configure(); B.setup_review_scene()
    report={'schemaVersion':2,'sourcePixels':'none','thirdPartyContent':False,
        'generator':'scripts/assets/build-vehicle-fleet-v2.py', 'blenderVersion':bpy.app.version_string,'bodies':{}}
    if args.seed_from:
        seed=Path(args.seed_from).expanduser().resolve()
        prior=json.loads((seed/'build-report.json').read_text())
        for body in prior['bodies'].values():
            for rec in [body,*body.get('variants',{}).values()]:
                for lod_record in rec['lods']: shutil.copyfile(seed/lod_record['file'],out/lod_record['file'])
        report['bodies']=prior['bodies']
        # A key now declared a variant cannot stay a top-level body recipe.
        for key in TRIMS: report['bodies'].pop(key,None)
    keys=args.bodies.split(',')
    for index,key in enumerate(keys):
        if key in TRIMS:
            parent=TRIMS[key][0]
            if parent not in report['bodies']: raise ValueError(f'{key}: build or seed {parent} first')
            entry=sedan_trim(out,key) if parent=='sedan' else build_trim(out,key)
            report['bodies'][parent].setdefault('variants',{})[key]=entry
            (out/'build-report.json').write_text(json.dumps(report,indent=2)+'\n')
            continue
        label,length,radius,types=ROSTER[key]
        record={'label':label,'lengthM':length,'wheelRadiusM':radius,'typeIds':types,'lods':[]}
        for lod in range(3):
            if key=='sedan': doors=sedan(lod); lib=SL
            else:
                doors=build_body(key,lod)
                lib=F
            finish_envelope(lib,length)
            finish_budget(lib,lod)
            bpy.context.view_layer.update()
            file=out/f'{key}-lod{lod}.glb'
            lib.export_glb(str(file))
            record['lods'].append({'file':file.name,'sha256':hashlib.sha256(file.read_bytes()).hexdigest(),
                'bytes':file.stat().st_size,'triangles':lib.tri_count(),'doors':doors})
            print('FLEET',key,lod,record['lods'][-1],flush=True)
            if lod==0:
                bpy.ops.wm.save_as_mainfile(filepath=str(out/f'{key}.blend'))
                if args.render:
                    B.render(str(out/f'{key}.png'),-35,16,dist=length*1.65,
                        target=(0,0,(B.CLASSES[key]['H'] if key!='sedan' else 1.45)*.45))
                    if key=='sedan': S.pose(True)
                    else:
                        for door in doors: bpy.data.objects['Door_'+door]['open_degrees']=60.0
                        bpy.context.scene.frame_set(2)
                    B.render(str(out/f'{key}-open.png'),35 if key=='cityBus' else -60,18,
                        dist=length*1.7,target=(0,0,(B.CLASSES[key]['H'] if key!='sedan' else 1.45)*.45))
                    if key=='sedan': S.pose(False)
                    else:
                        for door in doors: bpy.data.objects['Door_'+door]['open_degrees']=0.0
                        bpy.context.scene.frame_set(1)
        variants=report['bodies'].get(key,{}).get('variants')
        if variants: record['variants']=variants
        report['bodies'][key]=record
        (out/'build-report.json').write_text(json.dumps(report,indent=2)+'\n')
    if 'sedan' in keys: patrol_variant(out,report)


if __name__=='__main__': main()
