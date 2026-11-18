"""Build the runtime street-prop GLBs from their Tripo retopology masters.

Usage (headless):
  blender -b --factory-startup --python scripts/assets/build-street-props-v1.py

Each kind has an off-repo master under artifacts/tripo/street-furniture-v1/<kind>/lo/
(the server-side `tripo mesh decimate` output: retopologised, textures baked).
Three frontage kinds (cafeSet, sandwichBoard, utilityBox) come from CC0 Poly
Haven models instead (CC0_SOURCES), read from the off-repo Poly Haven store.
This script puts every master into the street-furniture catalogue's frame and
writes the shipped asset:

  * one mesh, origin at the centre of the base, base on y = 0 (glTF Y-up);
  * the long side along +X (along the street) and the FRONT toward -Z, which
    is the side `placeItem` turns to face the carriageway;
  * scaled to the kind's catalogue height, squashed toward the road by at most
    MAX_DEPTH_SQUASH, then shrunk further if a half-extent would still pass the
    kind's `radius` (along the street) or `depth` (toward the road) - placement
    tests the footprint with those numbers, so a prop drawn past them would hang
    over the kerb while the test called it clear;
  * textures downsized to TEXTURE_PX and written as JPEG.

It then writes public/assets/street-props/<kind>-v1.glb, a provenance manifest
beside them, and src/render/passes/street-prop-assets-manifest.js with each
file's sha256 - the runtime refuses a file whose digest does not match. Never
edit a digest by hand; rerun this script.
"""

import bpy
import hashlib
import json
import math
import os
import sys
from mathutils import Matrix, Vector

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
MASTERS = os.path.join(ROOT, 'artifacts', 'tripo', 'street-furniture-v1')
OUT_DIR = os.path.join(ROOT, 'public', 'assets', 'street-props')
MANIFEST_JS = os.path.join(ROOT, 'src', 'render', 'passes', 'street-prop-assets-manifest.js')
PROVENANCE = os.path.join(OUT_DIR, 'street-props-v1.provenance.json')
TEXTURE_PX = 512
MAX_DEPTH_SQUASH = 0.2

# Catalogue envelope per kind, copied from STREET_FURNITURE_KINDS in
# src/render/passes/street-furniture.js: height, radius (half-extent along the
# street), depth (half-extent toward the road).
#
# `front` is the side of the MASTER that carries the door, screen or seat, in
# Blender's frame: '+x', '-x', '+y', '-y', or 'away-from-top' to measure it from
# a backrest. It is not a constant of the generator - the first in-game pass
# showed the news rack, pay station and mailbox with their doors along the
# street, so each kind states what was seen. `face` is where that front ends up:
# 'road' (glTF -Z, the catalogue default) or 'footway' (glTF +Z) for the
# cabinets a person on the pavement uses.
KINDS = {
    'hydrant':      dict(height=0.78, radius=0.34, depth=0.22),
    'wasteBin':     dict(height=0.94, radius=0.36, depth=0.32, front='+x', face='footway'),
    'newsBox':      dict(height=1.18, radius=0.32, depth=0.22, squash=0.3, front='+x', face='footway'),
    'bench':        dict(height=0.86, radius=0.90, depth=0.30, front='away-from-top'),
    'parkingMeter': dict(height=1.25, radius=0.20, depth=0.11),
    'mailbox':      dict(height=1.20, radius=0.42, depth=0.24, squash=0.25, front='+x', face='footway'),
    'planter':      dict(height=0.80, radius=0.55, depth=0.34),
    'bollard':      dict(height=0.92, radius=0.16, depth=0.09),
    'payStation':   dict(height=1.50, radius=0.30, depth=0.16, squash=0.55, front='+x', face='footway'),
    'bikeRack':     dict(height=1.05, radius=0.95, depth=0.42),
    # Signs and transit (folsom r10, user 2026-09-26: "we don't need procedural
    # signs, traffic lights, stop signs, bus stops"). Pole kinds are anchored on
    # the POST, not the bounding box: the semantic street-name blades are drawn
    # on the pole axis, so the post must stand on the item origin. A sign plate
    # is a thin sheet, so depth may be squashed hard without changing the read,
    # and a plate a little wider than the kind's radius is narrowed rather than
    # shrinking the whole 2.6 m post.
    'signPole':       dict(height=2.60, radius=0.16, depth=0.06, squash=0.85, squashX=0.35, anchor='base', front='+x'),
    'busStopFlag':    dict(height=2.90, radius=0.18, depth=0.06, squash=0.85, squashX=0.45, anchor='base', front='+x'),
    'transitShelter': dict(height=2.60, radius=2.10, depth=0.66, squash=0.3, front='+x'),
    # Not a furniture kind: the signal head body, fitted by its lens stack
    # (see fit_signal_head) with its lenses toward glTF +Z.
    'signalHead':     dict(fit='signalHead', front='+x', face='footway'),
    # Not furniture kinds (real-kerbside lane): the timber distribution pole the
    # street-overhead pass strings its wire web from (see fit_utility_pole), and
    # the Canary Island date palm of the surveyed street-tree list.
    'utilityPole':    dict(fit='utilityPole'),
    'palm':           dict(fit='tree', height=10.0),
    'palmFar':        dict(fit='tree', height=10.0, decimateTo=900),
    'cobraArm':       dict(fit='mastArm'),
    # Street-light posts for streets without distribution poles, at the 25 ft
    # shaft of the SF residential standard (no ornamental-post standard is in
    # the table, so the Embarcadero's twin-lantern post takes the same height).
    'lampHistoric':   dict(fit='lamp'),
    'lampBanner':     dict(fit='lamp'),
    'lampCobra':      dict(fit='lamp', cutBanners=True),
    # A Chinatown paper lantern, hung by the street-overhead pass from its top;
    # 0.55 m tall with its tassel (a modelling size: no dataset gives one).
    'lantern':        dict(fit='tree', height=0.55, decimateTo=1300),
    # A bare 2-inch iron sign post (SFMTA's most common sign support), modelled
    # 11 ft tall; the street-signs pass stretches it to each stack's height.
    # Shaft radius: 2-inch nominal pipe is 2.375 in outside diameter (ASME
    # B36.10). The text-to-model master is decimated here (no Tripo retopology).
    'signPost':       dict(fit='tree', height=3.353, decimateTo=400, shaftRadius=0.0302),
    # The property-line utility cabinet, a CC0 Poly Haven model (CC0_MASTERS),
    # at its own measured size.
    'utilityBox':     dict(height=1.12, radius=0.46, depth=0.43, squash=0, front='-y'),
    # Frontage dressing from CC0 scans (see CC0_SOURCES). Heights are the
    # source's own measured heights, so the fit is a 1:1 scale and the props
    # keep their real proportions; the A-board alone is brought down from a
    # 1.5 m easel to a 1.0 m pavement board.
    #   cafeSet        folding table between two chairs, the line of three
    #                  along the shopfront (+x is the long side of the source).
    #   sandwichBoard  faces turned up and down the footway, toward walkers.
    'cafeSet':        dict(height=0.86, radius=0.95, depth=0.42, front='+x'),
    'sandwichBoard':  dict(height=1.00, radius=0.32, depth=0.32, front='+x'),
}
# CC0 sources (Poly Haven, every asset CC0-1.0). The 1k glTF lives in the
# off-repo Poly Haven store that scripts/blender-buildings/furniture/
# fetch-polyhaven.mjs fills; set EO_POLYHAVEN_STORE to that directory.
# Nothing is downloaded or kept in the working tree.
POLYHAVEN_STORE = os.environ.get('EO_POLYHAVEN_STORE')


def polyhaven_dir(name):
    if not POLYHAVEN_STORE:
        raise SystemExit('EO_POLYHAVEN_STORE is unset: point it at the off-repo Poly Haven store '
                         'filled by scripts/blender-buildings/furniture/fetch-polyhaven.mjs')
    return os.path.join(POLYHAVEN_STORE, name)


# CC0 masters (real-kerbside lane): routed ahead of text-to-model. Each names a
# folder in the Poly Haven store holding the downloaded glTF and the
# source.json recording its licence and URL.
CC0_MASTERS = {
    'utilityBox': 'utility_box_02',
}

# `faces` is the decimation target: these are near-ring props whose cost is
# per instance against the ring's triangle cap.
CC0_SOURCES = {
    'cafeSet': dict(polyhaven='outdoor_table_chair_set_01', faces=3400),
    'sandwichBoard': dict(polyhaven='standing_chalkboard_01', faces=1100),
}
ATLAS_CELL_PX = 512
# Kinds drawn by another pass than the furniture catalogue. The runtime keeps
# them out of the furniture installer (which requires a catalogue slot).
CONSUMER = {'signalHead': 'traffic-signal-heads', 'utilityPole': 'street-overhead', 'cobraArm': 'street-overhead',
            'lantern': 'street-overhead', 'signPost': 'street-signs'}
# Masters that were generated and looked at, and do not ship. The kind keeps
# its procedural mesh. Reason recorded so nobody regenerates blind.
REJECTED = {
    'planter': 'retopology flattened the foliage into disconnected shards',
}
# A kind may name a second master directory when its first generation was
# looked at and re-prompted (the first stays on disk as the record).
MASTER_DIR_RETRY = {
    # The first meter came back as a head on a stub with no post.
    'parkingMeter': os.path.join(MASTERS, 'parkingMeter2', 'lo'),
    # The first bollard was squat, 1.3:1, and fitted to its kind at 0.45 m tall.
    'bollard': os.path.join(MASTERS, 'bollard2', 'lo'),
    # Retopologised again at 900 faces (3461 -> fewer triangles) to hold the
    # near ring's population. The hydrant was tried at 800 and lost its bonnet,
    # so it keeps its 2000-face master.
    'bench': os.path.join(MASTERS, 'bench-lo1k'),
}
# The hydrant master predates the chain and lives under its own directory.
MASTER_DIR_OVERRIDE = {'hydrant': os.path.join(MASTERS, 'hydrant-lo2k')}
# The real-kerbside lane's masters. The palm ships its 10k-face retopology
# ('mid'): at 2.6k faces the fronds came apart into shards, as the planter did.
KERBSIDE_MASTERS = os.path.join(ROOT, 'artifacts', 'tripo', 'street-kerbside-v1')
MASTER_DIR_OVERRIDE['utilityPole'] = os.path.join(KERBSIDE_MASTERS, 'utilityPole', 'lo')
MASTER_DIR_OVERRIDE['palm'] = os.path.join(KERBSIDE_MASTERS, 'palmPhoenix', 'near5k')
MASTER_DIR_OVERRIDE['palmFar'] = os.path.join(KERBSIDE_MASTERS, 'palmPhoenix', 'far1k')
MASTER_DIR_OVERRIDE['cobraArm'] = os.path.join(KERBSIDE_MASTERS, 'cobraArm', 'lo')
MASTER_DIR_OVERRIDE['lampHistoric'] = os.path.join(KERBSIDE_MASTERS, 'lampHistoricTwin', 'lo')
MASTER_DIR_OVERRIDE['lampBanner'] = os.path.join(KERBSIDE_MASTERS, 'lampCobraBanner', 'lo')
MASTER_DIR_OVERRIDE['lampCobra'] = os.path.join(KERBSIDE_MASTERS, 'lampCobraBanner', 'lo')
MASTER_DIR_OVERRIDE['lantern'] = os.path.join(KERBSIDE_MASTERS, 'lantern', 'lo')
MASTER_DIR_OVERRIDE['signPost'] = os.path.join(KERBSIDE_MASTERS, 'signPost', 'hi')
KERBSIDE_KINDS = ('utilityPole', 'palm', 'palmFar', 'cobraArm', 'lampHistoric', 'lampBanner', 'lampCobra', 'lantern',
                  'signPost')


def cc0_source(kind):
    spec = CC0_SOURCES.get(kind)
    if not spec:
        return None
    folder = polyhaven_dir(spec['polyhaven'])
    record = os.path.join(folder, 'source.json')
    if not os.path.exists(record):
        return None
    info = json.load(open(record))
    gltf = os.path.join(folder, info['gltf'])
    return (gltf, info) if os.path.exists(gltf) else None


def find_master(kind):
    if kind in CC0_SOURCES:
        found = cc0_source(kind)
        return found[0] if found else None
    base = (polyhaven_dir(CC0_MASTERS[kind]) if kind in CC0_MASTERS else None) or MASTER_DIR_OVERRIDE.get(kind) or MASTER_DIR_RETRY.get(kind) or os.path.join(MASTERS, kind, 'lo')
    for dirpath, _dirs, files in os.walk(base):
        for name in files:
            if name.endswith('.glb') or (kind in CC0_MASTERS and name.endswith('.gltf')):
                return os.path.join(dirpath, name)
    return None


def task_id(kind, which):
    retry = MASTER_DIR_RETRY.get(kind)
    if kind in KERBSIDE_KINDS:
        retry = MASTER_DIR_OVERRIDE[kind]
    folder = os.path.dirname(retry) if retry else os.path.join(MASTERS, kind)
    path = os.path.join(folder, f'{which}.id')
    # Several retopologies of one master (the palm's near and far rungs) each
    # keep their own task id beside it, named after their directory.
    # A master taken straight from its generation (`hi`) has no retopology task.
    if which == 'lo' and retry and os.path.basename(retry) == 'hi':
        return None
    if which == 'lo' and retry and os.path.exists(os.path.join(folder, f'{os.path.basename(retry)}.id')):
        path = os.path.join(folder, f'{os.path.basename(retry)}.id')
    if os.path.exists(path):
        return open(path).read().strip()
    return None


def source_image(node_input):
    """The image a principled input is ultimately read from, or None."""
    if not node_input.is_linked:
        return None
    node = node_input.links[0].from_node
    for _ in range(6):
        if node.type == 'TEX_IMAGE':
            return node.image
        upstream = next((i for i in node.inputs if i.is_linked), None)
        if upstream is None:
            return None
        node = upstream.links[0].from_node
    return None


def pack_materials(kind, obj):
    """Fold a multi-material source into ONE material over a strip atlas.

    The runtime installs one material per kind (one instanced draw), so a
    source with a table and a chair material, or a frame and a board, is
    packed: each material's base colour, metal-rough and GL normal maps are
    resampled into a cell of a horizontal strip and its faces' UVs are moved
    into that cell. Every CC0 source here keeps its UVs inside [0, 1], which
    is what makes the remap exact (asserted).
    """
    import numpy as np
    me = obj.data
    mats = list(me.materials)
    if len(mats) < 2:
        return
    n, c = len(mats), ATLAS_CELL_PX
    roles = {
        'base': ('Base Color', (1.0, 1.0, 1.0, 1.0), 'sRGB'),
        'mr': ('Roughness', (0.0, 0.8, 0.0, 1.0), 'Non-Color'),
        'normal': ('Normal', (0.5, 0.5, 1.0, 1.0), 'Non-Color'),
    }
    atlases = {}
    for role, (socket, fill, space) in roles.items():
        strip = np.empty((c, c * n, 4), dtype=np.float32)
        strip[:] = fill
        for i, mat in enumerate(mats):
            bsdf = next(nd for nd in mat.node_tree.nodes if nd.type == 'BSDF_PRINCIPLED')
            img = source_image(bsdf.inputs[socket])
            if img is None:
                continue
            cell = img.copy()
            cell.scale(c, c)
            px = np.empty(c * c * 4, dtype=np.float32)
            cell.pixels.foreach_get(px)
            bpy.data.images.remove(cell)
            strip[:, i * c:(i + 1) * c] = px.reshape(c, c, 4)
        atlas = bpy.data.images.new(f'atlas:{kind}:{role}', width=c * n, height=c, alpha=False)
        atlas.colorspace_settings.name = space
        atlas.pixels.foreach_set(strip.ravel())
        path = os.path.join(bpy.app.tempdir, f'{kind}-{role}-atlas.png')
        atlas.filepath_raw = path
        atlas.file_format = 'PNG'
        atlas.save()
        atlases[role] = atlas
    uv = me.uv_layers.active.data
    for poly in me.polygons:
        for li in poly.loop_indices:
            u, v = uv[li].uv
            assert -1e-3 <= u <= 1.001 and -1e-3 <= v <= 1.001, f'{kind}: uv outside [0,1]'
            uv[li].uv = ((min(max(u, 0.0), 1.0) + poly.material_index) / n, v)
        poly.material_index = 0
    mat = bpy.data.materials.new(f'{kind}-atlas')
    mat.use_nodes = True
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    bsdf = next(nd for nd in nodes if nd.type == 'BSDF_PRINCIPLED')
    base = nodes.new('ShaderNodeTexImage'); base.image = atlases['base']
    links.new(base.outputs['Color'], bsdf.inputs['Base Color'])
    mr = nodes.new('ShaderNodeTexImage'); mr.image = atlases['mr']
    split = nodes.new('ShaderNodeSeparateColor')
    links.new(mr.outputs['Color'], split.inputs['Color'])
    links.new(split.outputs['Green'], bsdf.inputs['Roughness'])
    links.new(split.outputs['Blue'], bsdf.inputs['Metallic'])
    nor = nodes.new('ShaderNodeTexImage'); nor.image = atlases['normal']
    nmap = nodes.new('ShaderNodeNormalMap')
    links.new(nor.outputs['Color'], nmap.inputs['Color'])
    links.new(nmap.outputs['Normal'], bsdf.inputs['Normal'])
    me.materials.clear()
    me.materials.append(mat)
    print(f'[street-props] {kind}: packed {n} materials into a {c * n}x{c} atlas')


def decimate_to(kind, obj, faces):
    tris = sum(len(p.vertices) - 2 for p in obj.data.polygons)
    if not faces or tris <= faces:
        return
    bpy.context.view_layer.objects.active = obj
    mod = obj.modifiers.new('decimate', 'DECIMATE')
    mod.ratio = faces / tris
    mod.use_collapse_triangulate = True
    bpy.ops.object.modifier_apply(modifier=mod.name)
    after = sum(len(p.vertices) - 2 for p in obj.data.polygons)
    print(f'[street-props] {kind}: decimated {tris} -> {after} triangles')


def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def fit_signal_head(obj, lens_pitch=0.35, lens_face=0.15):
    """Put a generated three-lens head into the traffic-signal-heads frame.

    That pass (src/render/passes/traffic-signal-heads.js) draws the LENSES
    itself - lit colour, phase and glow are simulation state - at y = +0.35,
    0, -0.35 on local +Z, 0.158 m proud of the head origin. So the model is
    fitted by its lens stack, not its bounding box: the housing (everything in
    front of the backplate) is found, split in three equal sections, scaled so
    the sections are `lens_pitch` apart, centred on the middle section, and
    pushed along +Z until the lens surface sits at `lens_face`, just behind
    the drawn lens discs. Anything behind the backplate or below/above it (the
    master's stub mounting pole) is cut: the pass draws its own poles and arms.
    Blender frame here: glTF +Z (toward traffic) is Blender -Y.
    """
    import bmesh
    vs = obj.data.vertices
    ys = [v.co.y for v in vs]
    # The backplate is the widest sheet; its plane is the modal y of vertices
    # outside the housing's width. The housing is what stands in front of it.
    xs = sorted(abs(v.co.x) for v in vs)
    wide = xs[int(len(xs) * 0.9)]
    plate = [v.co for v in vs if abs(v.co.x) > wide * 0.75]
    plate_back = max(c.y for c in plate)   # the plate's rear face (Blender +Y is behind)
    plate_front = min(c.y for c in plate)
    pz0 = min(c.z for c in plate)
    pz1 = max(c.z for c in plate)
    housing = [v.co for v in vs if v.co.y < plate_front - 0.01 and abs(v.co.x) < wide * 0.75]
    hz0 = min(c.z for c in housing)
    hz1 = max(c.z for c in housing)
    hx = (min(c.x for c in housing) + max(c.x for c in housing)) / 2
    pitch = (hz1 - hz0) / 3
    mid = (hz0 + hz1) / 2
    # Lens surface: the front-most vertex close to the middle lens axis (the
    # visor tunnels are off-axis, the lens dome is on it).
    near_axis = [c for c in housing if abs(c.x - hx) < pitch * 0.2 and abs(c.z - mid) < pitch * 0.2]
    lens_y = min(c.y for c in near_axis) if near_axis else min(c.y for c in housing)
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    cut = [f for f in bm.faces
           if f.calc_center_median().y > plate_back + 0.002
           or min(v.co.z for v in f.verts) < pz0 - 0.002
           or max(v.co.z for v in f.verts) > pz1 + 0.002]
    bmesh.ops.delete(bm, geom=cut, context='FACES')
    bm.to_mesh(obj.data)
    bm.free()
    s = lens_pitch / pitch
    # The generated plate runs far above the housing (0.37 m against 0.06 m
    # below). A mast head hangs 0.70 m under its arm, so that plate would pass
    # through the arm; a US plate is symmetric about its housing. The part of
    # the plate above the housing is compressed to the 0.12 m margin the
    # procedural plate had (or the bottom margin, if larger).
    top_margin = max(hz0 - pz0, 0.12 / s)
    if pz1 - hz1 > top_margin:
        k = top_margin / (pz1 - hz1)
        for v in obj.data.vertices:
            if v.co.z > hz1:
                v.co.z = hz1 + (v.co.z - hz1) * k
        pz1 = hz1 + top_margin
    for v in obj.data.vertices:
        v.co = Vector(((v.co.x - hx) * s, (v.co.y - lens_y) * s - lens_face, (v.co.z - mid) * s))
    obj.data.update()
    print(f'[street-props] signalHead fit: pitch {pitch:.4f} -> {lens_pitch}, scale {s:.4f}, '
          f'plate z {((pz0 - mid) * s):.3f}..{((pz1 - mid) * s):.3f}, cut {len(cut)} faces')
    return s


# A distribution crossarm is 8 ft (2.438 m), the standard length of USDA RUS
# Bulletin 1728H-701 (Specification for Wood Crossarms). The generated master
# has no real scale; its crossarm sets the scale of all its hardware.
CROSSARM_M = 2.438
# A 45 ft wood utility pole stands 38.5 ft (11.73 m) above grade, set 6.5 ft
# deep (supplier listings of 45 ft class poles; no PG&E drawing was reachable,
# recorded as such in the provenance). The shaft BELOW the hardware is
# stretched to this height; the crossarms, insulators and can keep the
# crossarm's scale.
POLE_ABOVE_GRADE_M = 11.73


def fit_utility_pole(obj):
    """Put the generated pole in the street-overhead frame and find its wire seats.

    Origin at the foot of the shaft, crossarms along glTF X (the pass turns X
    across the street), hardware at the crossarm's real scale, the shaft below
    the lowest hardware stretched so the top stands POLE_ABOVE_GRADE_M high.
    Returns (scale, extras): extras.attach lists every crossarm's insulator
    tops in glTF metres - where the pass hangs each conductor of a span.
    """
    vs = obj.data.vertices

    def coords():
        return [v.co.copy() for v in vs]

    cs = coords()
    z0 = min(c.z for c in cs)
    z1 = max(c.z for c in cs)
    span = z1 - z0
    low = [c for c in cs if c.z < z0 + span * 0.25]
    cx = sum(c.x for c in low) / len(low)
    cy = sum(c.y for c in low) / len(low)
    shaft_r = sorted(math.hypot(c.x - cx, c.y - cy) for c in low)[int(len(low) * 0.9)]
    top = [c for c in cs if c.z > z0 + span * 0.75]
    ex = max(c.x for c in top) - min(c.x for c in top)
    ey = max(c.y for c in top) - min(c.y for c in top)
    if ey > ex:
        obj.data.transform(Matrix.Rotation(math.pi / 2, 4, 'Z'))
        obj.data.update()
        cx, cy = -cy, cx
        cs = coords()
    top = [c for c in cs if c.z > z0 + span * 0.75]
    arm = max(c.x for c in top) - min(c.x for c in top)
    s = CROSSARM_M / arm
    hardware = [c for c in cs if math.hypot(c.x - cx, c.y - cy) > shaft_r * 3 and c.z > z0 + span * 0.3]
    zhb = min(c.z for c in hardware)
    k = (POLE_ABOVE_GRADE_M - (z1 - zhb) * s) / ((zhb - z0) * s)
    for v in vs:
        c = v.co
        z = (c.z - z0) * s * k if c.z < zhb else (zhb - z0) * s * k + (c.z - zhb) * s
        v.co = Vector(((c.x - cx) * s, (c.y - cy) * s, z))
    obj.data.update()
    cs = coords()
    half = CROSSARM_M / 2
    # Crossarm levels: only an arm reaches past 0.9 of its half length (the
    # can and its brackets stop well inside that), on BOTH sides of the pole.
    tips = sorted((c for c in cs if abs(c.x) > half * 0.9), key=lambda c: c.z)
    levels = []
    for c in tips:
        if levels and c.z - levels[-1][-1].z < 0.12:
            levels[-1].append(c)
        else:
            levels.append([c])
    levels = [lv for lv in levels if any(c.x > 0 for c in lv) and any(c.x < 0 for c in lv)]
    # A seat is the top of an insulator standing on that arm: the highest
    # vertex out along each side of the arm, no more than an insulator's height
    # above the arm top.
    attach = []
    for lv in levels:
        arm_top = max(c.z for c in lv)
        for sign in (1, -1):
            side = [c for c in cs if half * 0.3 < sign * c.x < half * 1.05 and arm_top - 0.01 < c.z < arm_top + 0.3]
            if not side:
                continue
            ztop = max(c.z for c in side)
            near = [c for c in side if c.z > ztop - 0.04]
            attach.append({
                'x': round(sum(c.x for c in near) / len(near), 4),
                'y': round(ztop, 4),
                'z': round(-sum(c.y for c in near) / len(near), 4),   # Blender Y -> glTF -Z
                'arm': len(levels) - levels.index(lv) - 1,   # 0 = top arm
            })
    attach.sort(key=lambda a: (-a['y'], a['x']))
    print(f'[street-props] utilityPole fit: crossarm scale {s:.3f}, shaft stretch {k:.3f}, '
          f'hardware from {((zhb - z0) * s * k):.2f} m, seats {attach}')
    return s, {'attach': attach, 'heightM': round(POLE_ABOVE_GRADE_M, 3), 'crossarmM': CROSSARM_M,
              'hardwareFromM': round((zhb - z0) * s * k, 3), 'shaftRadiusM': round(shaft_r * s, 3)}


# SF Public Works Standard Plan 87,205: the residential standard's 6 ft Type K
# mast arm (`streetlight-sf-residential-pole` armLength in the California
# street standards table).
MAST_ARM_M = 1.829


def fit_mast_arm(obj):
    """A cobra-head mast arm to bolt onto a distribution pole: arm and head only.

    The generated master came with two post stubs at its bracket end (the
    prompt said "no pole"); everything below the arm, and the post standing
    behind its root, is cut. The root goes to the origin, the arm along glTF
    +X, scaled so root to head tip is MAST_ARM_M.
    """
    import bmesh
    vs = obj.data.vertices
    xs = [v.co.x for v in vs]
    ys = [v.co.y for v in vs]
    if max(ys) - min(ys) > max(xs) - min(xs):
        obj.data.transform(Matrix.Rotation(-math.pi / 2, 4, 'Z'))
        obj.data.update()
    cs = [v.co for v in vs]
    z0 = min(c.z for c in cs)
    z1 = max(c.z for c in cs)
    x0 = min(c.x for c in cs)
    x1 = max(c.x for c in cs)
    # The head end carries the luminaire, the widest part across; the root end
    # the posts. Measure each end's spread in Y over its top fifth.
    top = [c for c in cs if c.z > z0 + (z1 - z0) * 0.8]
    near0 = [c.y for c in top if c.x < x0 + (x1 - x0) * 0.2]
    near1 = [c.y for c in top if c.x > x1 - (x1 - x0) * 0.2]
    spread = lambda a: (max(a) - min(a)) if a else 0
    if spread(near0) > spread(near1):
        obj.data.transform(Matrix.Rotation(math.pi, 4, 'Z'))
        obj.data.update()
        cs = [v.co for v in vs]
        x0, x1 = min(c.x for c in cs), max(c.x for c in cs)
    span = x1 - x0
    # The arm runs in the top band; the posts stand below it at the root end.
    arm_z = z0 + (z1 - z0) * 0.55
    root_x = x0 + span * 0.08
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    cut = [f for f in bm.faces
           if f.calc_center_median().z < arm_z or f.calc_center_median().x < root_x]
    bmesh.ops.delete(bm, geom=cut, context='FACES')
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
    # Shreds of the cut posts stay behind under the root. The arm sweeps UP
    # from its root to the head, so nothing of it lies below the root.
    root = [v.co for v in bm.verts if v.co.x < root_x + span * 0.06]
    if root:
        root_z = sorted(c.z for c in root)[len(root) // 2]
        low = [f for f in bm.faces if f.calc_center_median().z < root_z - span * 0.06]
        bmesh.ops.delete(bm, geom=low, context='FACES')
        bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
    bm.to_mesh(obj.data)
    bm.free()
    cs = [v.co for v in obj.data.vertices]
    rx = min(c.x for c in cs)
    tip = max(c.x for c in cs)
    root = [c for c in cs if c.x < rx + 0.03 * (tip - rx)]
    ry = sum(c.y for c in root) / len(root)
    rz = sum(c.z for c in root) / len(root)
    s = MAST_ARM_M / (tip - rx)
    for v in obj.data.vertices:
        v.co = Vector(((v.co.x - rx) * s, (v.co.y - ry) * s, (v.co.z - rz) * s))
    obj.data.update()
    print(f'[street-props] cobraArm fit: cut {len(cut)} faces, scale {s:.3f}')
    return s, {'armM': MAST_ARM_M}


def standards_metric(entry_id, key):
    """A value from the California street standards table (the source JSON)."""
    table = json.load(open(os.path.join(ROOT, 'scripts', 'world-data', 'sources', 'california-street-standards.json')))
    for e in table['entries']:
        if e['id'] == entry_id:
            return e['metricValue'][key]
    raise KeyError(entry_id)


def fit_lamp(obj, height, cut_banners=False):
    """A street-light post: foot of the shaft at the origin, upright, `height` tall.

    The arm or lanterns keep the master's proportions. `cut_banners` removes
    the merchant banners of the banner master (everything standing out from
    the shaft below the arm), which is the plain cobra-head pole.
    """
    import bmesh
    vs = obj.data.vertices
    z0 = min(v.co.z for v in vs)
    z1 = max(v.co.z for v in vs)
    span = z1 - z0
    low = [v.co for v in vs if v.co.z < z0 + span * 0.12]
    cx = sum(c.x for c in low) / len(low)
    cy = sum(c.y for c in low) / len(low)
    shaft = sorted(math.hypot(c.x - cx, c.y - cy) for c in low)[int(len(low) * 0.5)]
    cut = 0
    if cut_banners:
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        # The banners hang close beside the shaft, on ONE side, between their
        # two brackets in the middle of the pole. The shaft's own radius is
        # read there (its lower decile of vertex distances); a face standing
        # out past it toward the banner side is banner.
        lo_z, hi_z = z0 + span * 0.2, z0 + span * 0.8
        band = [v.co for v in bm.verts if lo_z < v.co.z < hi_z]
        dists = sorted(math.hypot(c.x - cx, c.y - cy) for c in band)
        r_mid = dists[len(dists) // 10]
        bx = sum(c.x - cx for c in band) / len(band)
        by = sum(c.y - cy for c in band) / len(band)
        bl = math.hypot(bx, by) or 1.0
        bx, by = bx / bl, by / bl
        faces = [f for f in bm.faces
                 if lo_z < f.calc_center_median().z < hi_z
                 and (f.calc_center_median().x - cx) * bx + (f.calc_center_median().y - cy) * by > r_mid * 1.25]
        cut = len(faces)
        bmesh.ops.delete(bm, geom=faces, context='FACES')
        bmesh.ops.delete(bm, geom=[v for v in bm.verts if not v.link_faces], context='VERTS')
        bm.to_mesh(obj.data)
        bm.free()
    s = height / span
    for v in obj.data.vertices:
        v.co = Vector(((v.co.x - cx) * s, (v.co.y - cy) * s, (v.co.z - z0) * s))
    obj.data.update()
    # THE ARM FACES THE ROAD. The furniture pass turns glTF -Z (Blender +Y)
    # to the carriageway: a one-sided head (the cobra arm) is turned that way.
    # A symmetric head (the twin lanterns) lays its arms along the street,
    # glTF X.
    top = [v.co for v in obj.data.vertices if v.co.z > height * 0.85]
    ox = sum(c.x for c in top) / len(top)
    oy = sum(c.y for c in top) / len(top)
    if math.hypot(ox, oy) > shaft * s * 2:
        yaw = math.atan2(1.0, 0.0) - math.atan2(oy, ox)
    else:
        ex = max(c.x for c in top) - min(c.x for c in top)
        ey = max(c.y for c in top) - min(c.y for c in top)
        yaw = math.pi / 2 if ey > ex else 0.0
    obj.data.transform(Matrix.Rotation(yaw, 4, 'Z'))
    obj.data.update()
    print(f'[street-props] lamp fit: height {height} m, scale {s:.3f}, banner faces cut {cut}, turned {math.degrees(yaw):.0f} deg')
    return s, {'heightM': height}


def fit_tree(obj, height, decimate_to=None, shaft_radius=None):
    """Trunk foot at the origin (the centroid of the lowest tenth), upright, `height` tall.

    `decimate_to` collapses the mesh toward that many triangles (the window-tier
    palm: past the near ring a frond torn into shards and a trunk with holes are
    a few pixels; its triangle count is what the ring cap pays for).
    """
    if decimate_to:
        tris = sum(len(p.vertices) - 2 for p in obj.data.polygons)
        if tris > decimate_to:
            bpy.context.view_layer.objects.active = obj
            mod = obj.modifiers.new('decimate', 'DECIMATE')
            mod.ratio = decimate_to / tris
            mod.use_collapse_triangulate = True
            bpy.ops.object.modifier_apply(modifier=mod.name)
    vs = obj.data.vertices
    z0 = min(v.co.z for v in vs)
    z1 = max(v.co.z for v in vs)
    low = [v.co for v in vs if v.co.z < z0 + (z1 - z0) * 0.1]
    cx = sum(c.x for c in low) / len(low)
    cy = sum(c.y for c in low) / len(low)
    s = height / (z1 - z0)
    # `shaft_radius` narrows the plan (x, y) so the shaft's median radius over
    # its middle third is that radius: a text-to-model post comes back several
    # times thicker than the pipe it was asked for.
    k = 1.0
    if shaft_radius:
        mid = [v.co for v in vs if z0 + (z1 - z0) / 3 < v.co.z < z0 + 2 * (z1 - z0) / 3]
        r = sorted(math.hypot(c.x - cx, c.y - cy) for c in mid)[len(mid) // 2] * s
        k = shaft_radius / r
    for v in vs:
        v.co = Vector(((v.co.x - cx) * s * k, (v.co.y - cy) * s * k, (v.co.z - z0) * s))
    obj.data.update()
    return s, {'heightM': height, **({'shaftRadiusM': shaft_radius} if shaft_radius else {})}


def build(kind, spec):
    master = find_master(kind)
    if not master:
        print(f'[street-props] {kind}: no master, skipped')
        return None
    reset()
    bpy.ops.import_scene.gltf(filepath=master)
    meshes = [o for o in bpy.context.scene.objects if o.type == 'MESH']
    if not meshes:
        return None
    if kind in CC0_SOURCES:
        # A scanned set arrives as several placed objects. Bake each world
        # transform into its own vertices first, so the join below cannot keep
        # a parent's offset that the headless transform_apply would then skip.
        for o in meshes:
            o.data.transform(o.matrix_world)
            o.parent = None
            o.matrix_world = Matrix.Identity(4)
    bpy.ops.object.select_all(action='DESELECT')
    for o in meshes:
        o.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    if kind in CC0_SOURCES:
        pack_materials(kind, obj)
        decimate_to(kind, obj, CC0_SOURCES[kind].get('faces'))
    # Unparent and bake every transform into the vertices.
    bpy.ops.object.parent_clear(type='CLEAR_KEEP_TRANSFORM')
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    for o in list(bpy.context.scene.objects):
        if o is not obj:
            bpy.data.objects.remove(o, do_unlink=True)

    # Blender frame: Z up, glTF -Z (toward the road) is Blender +Y.
    def bounds():
        xs = [v.co.x for v in obj.data.vertices]
        ys = [v.co.y for v in obj.data.vertices]
        zs = [v.co.z for v in obj.data.vertices]
        return min(xs), max(xs), min(ys), max(ys), min(zs), max(zs)

    if spec.get('fit') == 'utilityPole':
        scale, extra = fit_utility_pole(obj)
        return finish(kind, obj, scale, bounds, extra)
    if spec.get('fit') == 'tree':
        scale, extra = fit_tree(obj, spec['height'], spec.get('decimateTo'), spec.get('shaftRadius'))
        return finish(kind, obj, scale, bounds, extra)
    if spec.get('fit') == 'lamp':
        height = standards_metric('streetlight-sf-residential-pole', 'shaftHeight')
        scale, extra = fit_lamp(obj, height, spec.get('cutBanners', False))
        return finish(kind, obj, scale, bounds, extra)
    if spec.get('fit') == 'mastArm':
        scale, extra = fit_mast_arm(obj)
        return finish(kind, obj, scale, bounds, extra)

    # THE FRONT. Tripo masters face glTF +Z (Blender -Y). A bench is the
    # exception worth measuring rather than assuming: its backrest is the tall
    # part, so the front is the side away from the vertices in its top third.
    x0, x1, y0, y1, z0, z1 = bounds()
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    front = {'+x': (1.0, 0.0), '-x': (-1.0, 0.0), '+y': (0.0, 1.0), '-y': (0.0, -1.0)}.get(
        spec.get('front', '-y'), (0.0, -1.0))
    if spec.get('front') == 'away-from-top':
        top = [v.co for v in obj.data.vertices if v.co.z > z0 + (z1 - z0) * 0.66]
        mx = sum(c.x for c in top) / len(top) - cx
        my = sum(c.y for c in top) / len(top) - cy
        front = (-1.0 if mx > 0 else 1.0, 0.0) if abs(mx) > abs(my) else (0.0, -1.0 if my > 0 else 1.0)
    # Turn the front to its face: the road is glTF -Z (Blender +Y), the footway
    # glTF +Z (Blender -Y). Rotating the vertices
    # directly: an object-level transform_apply is silently skipped in this
    # headless context, which is how the first bench shipped sideways.
    target = (0.0, -1.0) if spec.get('face') == 'footway' else (0.0, 1.0)
    yaw = math.atan2(target[1], target[0]) - math.atan2(front[1], front[0])
    obj.data.transform(Matrix.Rotation(yaw, 4, 'Z'))
    obj.data.update()
    if spec.get('fit') == 'signalHead':
        return finish(kind, obj, fit_signal_head(obj), bounds)
    x0, x1, y0, y1, z0, z1 = bounds()
    height = z1 - z0
    # The origin: the footprint's centre, or for a post kind (`anchor='base'`)
    # the centroid of the bottom quarter, which is the post alone.
    ax, ay = (x0 + x1) / 2, (y0 + y1) / 2
    if spec.get('anchor') == 'base':
        low = [v.co for v in obj.data.vertices if v.co.z < z0 + height * 0.25]
        if low:
            ax = sum(c.x for c in low) / len(low)
            ay = sum(c.y for c in low) / len(low)
    dx = 2 * max(x1 - ax, ax - x0)
    dy = 2 * max(y1 - ay, ay - y0)
    # Height first. The footprint toward the road may be squashed by up to
    # MAX_DEPTH_SQUASH before the whole prop has to shrink - a box 20% shallower
    # reads as the same object, a mailbox 40% shorter does not. A cabinet kind
    # may name its own `squash`: the generator models a pay station or a news
    # rack as a deep box, and the real ones are slim, so squashing it toward
    # the road moves it TOWARD its true proportions rather than away. A sign
    # kind may also name `squashX`, the same allowance along the street.
    s = spec['height'] / height
    sx = s
    if dx * sx > 2 * spec['radius']:
        sx = max(2 * spec['radius'] / dx, s * (1 - spec.get('squashX', 0)))
        if dx * sx > 2 * spec['radius'] + 1e-6:
            s = 2 * spec['radius'] / dx
            sx = s
    sy = min(s, 2 * spec['depth'] / dy) if dy > 0 else s
    sy = max(sy, s * (1 - spec.get('squash', MAX_DEPTH_SQUASH)))
    if dy * sy > 2 * spec['depth'] + 1e-6:
        shrink = 2 * spec['depth'] / (dy * sy)
        s *= shrink
        sx *= shrink
        sy *= shrink
    scale = s
    for v in obj.data.vertices:
        v.co = Vector(((v.co.x - ax) * sx,
                       (v.co.y - ay) * sy,
                       (v.co.z - z0) * s))
    obj.data.update()
    return finish(kind, obj, scale, bounds)


def finish(kind, obj, scale, bounds, extra=None):
    """Downsize textures, export the GLB and return its provenance row."""
    for image in bpy.data.images:
        # A packed strip atlas is already ATLAS_CELL_PX per source material.
        if image.name.startswith('atlas:'):
            continue
        if image.size[0] > TEXTURE_PX or image.size[1] > TEXTURE_PX:
            image.scale(TEXTURE_PX, TEXTURE_PX)

    x0, x1, y0, y1, z0, z1 = bounds()
    dims = [round(x1 - x0, 4), round(z1 - z0, 4), round(y1 - y0, 4)]  # glTF x, y, z
    triangles = sum(len(p.vertices) - 2 for p in obj.data.polygons)

    os.makedirs(OUT_DIR, exist_ok=True)
    out = os.path.join(OUT_DIR, f'{kind}-v1.glb')
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.export_scene.gltf(
        filepath=out, export_format='GLB', use_selection=True,
        export_image_format='JPEG', export_yup=True, export_apply=True,
        export_animations=False, export_extras=False, export_cameras=False, export_lights=False,
    )
    data = open(out, 'rb').read()
    row = {
        'path': f'public/assets/street-props/{kind}-v1.glb',
        'url': f'/assets/street-props/{kind}-v1.glb',
        'sha256': hashlib.sha256(data).hexdigest(),
        'bytes': len(data),
        'triangles': triangles,
        'dimensionsM': dims,
        'scale': round(scale, 5),
        'source': {
            'generator': 'Tripo text-to-model, then Tripo highpoly_to_lowpoly (textures baked)',
            'modelTask': task_id(kind, 'hi'),
            'retopologyTask': task_id(kind, 'lo'),
        },
    }
    found = cc0_source(kind) if kind in CC0_SOURCES else None
    if kind in CC0_MASTERS:
        source = json.load(open(os.path.join(polyhaven_dir(CC0_MASTERS[kind]), 'source.json')))
        row['source'] = {'generator': 'CC0 model, fitted and exported by this script', 'name': source['name'],
                         'url': source['source'], 'license': source['license'], 'authors': source['authors']}
    elif found:
        gltf, info = found
        row['source'] = {
            'generator': 'Poly Haven CC0 model (1k glTF), materials packed into one atlas and decimated by this script',
            'name': info.get('name'),
            'url': info.get('source'),
            'authors': info.get('authors'),
            'license': info.get('license'),
            'gltfSha256': hashlib.sha256(open(gltf, 'rb').read()).hexdigest(),
            'decimatedTo': CC0_SOURCES[kind].get('faces'),
        }
    elif not row['source']['retopologyTask'] and KINDS.get(kind, {}).get('decimateTo'):
        row['source']['generator'] = 'Tripo text-to-model, then Blender decimate (this script)'
    if kind in CONSUMER:
        row['consumer'] = CONSUMER[kind]
    if kind in KERBSIDE_KINDS:
        prompt = os.path.join(os.path.dirname(MASTER_DIR_OVERRIDE[kind]), 'prompt.txt')
        if os.path.exists(prompt):
            row['source']['prompt'] = open(prompt).read().strip()
    if extra:
        row.update(extra)
    return row


def main():
    only = [a for a in sys.argv[sys.argv.index('--') + 1:]] if '--' in sys.argv else []
    rows = {}
    if os.path.exists(PROVENANCE):
        rows = json.load(open(PROVENANCE)).get('assets', {})
    for kind, reason in REJECTED.items():
        rows.pop(kind, None)
        stale = os.path.join(OUT_DIR, f'{kind}-v1.glb')
        if os.path.exists(stale):
            os.remove(stale)
        print(f'[street-props] {kind}: rejected ({reason})')
    for kind, spec in KINDS.items():
        if (only and kind not in only) or kind in REJECTED:
            continue
        row = build(kind, spec)
        if row:
            rows[kind] = row
            print(f'[street-props] {kind}: {row["triangles"]} tri, {row["bytes"]} B, dims {row["dimensionsM"]}')
    json.dump({
        'version': 'street-props-v1',
        'note': 'Generated with Tripo AI from text prompts (retopologised and baked by Tripo), or taken from CC0 Poly Haven models (see each source); normalised by scripts/assets/build-street-props-v1.py.',
        'assets': rows,
        'rejected': REJECTED,
    }, open(PROVENANCE, 'w'), indent=2, sort_keys=True)
    manifest = {k: {key: v[key] for key in ('path', 'url', 'sha256', 'bytes', 'triangles', 'dimensionsM', 'consumer',
                                            'attach', 'heightM', 'crossarmM', 'hardwareFromM', 'shaftRadiusM', 'armM')
                    if key in v}
                for k, v in sorted(rows.items())}
    with open(MANIFEST_JS, 'w') as f:
        f.write('// Generated by scripts/assets/build-street-props-v1.py; do not edit digests.\n')
        f.write('export const STREET_PROP_ASSETS_V1 = Object.freeze(')
        f.write(json.dumps(manifest, indent=2))
        f.write(');\n')


main()
