# build-robo-cab-v1.py - derive the runtime roboCab body from the off-repo
# autonomous-cab study .blend and export the three LOD GLBs the vehicle
# contract requires.
#
# Usage:
#   blender -b --python scripts/assets/build-robo-cab-v1.py -- <study.blend> <out_dir>
#
# The source is an off-repository study: a user-authored approximation of a
# two-seat autonomous cab (X-longitudinal axes, descriptive node names,
# ~1.95M triangles). This script performs the contract adaptation:
#
#   * reorients the body so the nose faces +Z after the glTF +Y-up conversion,
#   * renames the rig to the semantic names the bake binds (Wheel_*/Wheel_*_Spin,
#     Door_FL/Door_FR, Window_*, Trunk, Seat_*, Plate*),
#   * carries the study's fitted door open-pose onto the door pivots as a
#     hinge-axis descriptor the runtime reads from glTF extras,
#   * folds the study's descriptive materials onto the canonical fleet set,
#   * deletes micro-detail and decimates into the fleet's tier budgets,
#   * writes a build-report.json beside the three GLBs.
#
# It never writes into the source study and never imports Earth Online.

import bpy
import hashlib
import json
import math
import struct
import sys
from mathutils import Matrix, Quaternion, Vector
from pathlib import Path

FRAME_KEY, FRAME = 'vehicleFrame', 'lhd-v1'  # scripts/assets/vehicle_frame.py

argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
SOURCE = Path(argv[0]).resolve()
OUT = Path(argv[1]).resolve()
OUT.mkdir(parents=True, exist_ok=True)

BODY_KEY = 'roboCab'
VEHICLE_ID = 'robo-cab-v1'

# ----------------------------------------------------------------------------
# name contract
# ----------------------------------------------------------------------------

# The study authors -X as the nose, lateral +/-Y, Z up, and its L/R names are
# physically right: its left wheels sit at -Y, which with the nose at -X is the
# car's left. Rotating the asset root +90 degrees about Blender Z carries that
# flank to +X, the runtime frame's left flank (nose +Z, Y up; see
# scripts/assets/vehicle_frame.py), so every study L/R name keeps its side.
WHEEL_RENAME = {
    'Front left | wheel rotation': 'Wheel_FL',
    'Front right | wheel rotation': 'Wheel_FR',
    'Rear left | wheel rotation': 'Wheel_RL',
    'Rear right | wheel rotation': 'Wheel_RR',
}
DOOR_RENAME = {
    'DOOR L | butterfly pivot • inferred': 'Door_FL',
    'DOOR R | butterfly pivot • inferred': 'Door_FR',
}
WINDOW_RENAME = {  # study side -> runtime window node
    'Door optical glazing L': 'Window_FL',
    'Door optical glazing R': 'Window_FR',
}
SEAT_RENAME = {
    'SEAT L | adjustable assembly': 'Seat_FL_Rig',
    'SEAT R | adjustable assembly': 'Seat_FR_Rig',
}
SEAT_PARTS = {  # runtime anchor -> study meshes to measure (still study names)
    'Seat_FL': ('Seat L | sculpted cushion', 'Seat L | continuous lounge back and headrest'),
    'Seat_FR': ('Seat R | sculpted cushion', 'Seat R | continuous lounge back and headrest'),
}
HATCH_NODE = 'HATCH | pivot • inferred hinge'

# Nodes whose names the runtime contract binds; joins must never absorb them.
CONTRACT_NODES = set(WHEEL_RENAME.values()) | {n + '_Spin' for n in WHEEL_RENAME.values()} \
    | set(DOOR_RENAME.values()) | set(WINDOW_RENAME.values()) \
    | {'Trunk', 'PlateFront', 'PlateRear'} \
    | {'Seat_FL_Base', 'Seat_FL_Headrest0', 'Seat_FR_Base', 'Seat_FR_Headrest0'}

# STAGGERED FRONT AXLE. The study authors one wheel and fits it at all four
# corners; the car it studies does not. Its rear is a 225/60R21 - 803.4 mm
# across - which is what the study's 801.5 mm tyre already is, within 2 mm. Its
# front is a 215/60R18: 715.2 mm across and one size narrower. So the front
# wheel here is the study's own wheel taken down by the REAL car's own ratios,
# which keeps the study's styling and changes only its size.
FRONT_WHEELS = ('Wheel_FL', 'Wheel_FR')
FRONT_RADIAL_SCALE = 715.2 / 803.4   # 0.8902, front tyre diameter over rear
FRONT_AXIAL_SCALE = 215.0 / 225.0    # 0.9556, front section width over rear

# Children of a wheel steer pivot that must NOT ride the rolling spin. The
# caliper hangs off the knuckle, not the wheel; everything else (tyre, aero
# disc, barrel, rotor) spins.
WHEEL_STATIC = ('brake caliper',)

# Canonical fleet material names; unknown names each grow their own material
# graph, so the study's descriptive set is folded onto this table.
MATERIAL_MAP = {
    'Champagne | satin metallic clearcoat': 'VehiclePaint',
    'Champagne | machined wheel edges': 'VehicleAlloy',
    'Glazing | lightly smoked optical glass': 'VehicleGlass',
    'Glazing | panoramic dark roof': 'VehicleGlass',
    'Rubber | seals and tires': 'VehicleTyre',
    'Obsidian | painted trim': 'VehicleTrimBlack',
    'Satin aluminum': 'VehicleAlloy',
    'Brake rotor steel': 'VehicleSteel',
    'Ivory | fine-grain upholstery': 'VehicleSeat',
    'Ivory | stitched thread': 'VehicleSeatInsert',
    'Warm grey | woven headliner': 'VehicleHeadliner',
    'Graphite | loop-pile carpet': 'VehicleCarpet',
    'Warm graphite | soft dash': 'VehicleDash',
    'LED | warm cabin accent': 'VehicleInterior',
    'LED | cool white': 'VehicleDRL',
    'LED | ruby rear strip': 'VehicleTaillamp',
    'Screen | charcoal luminous panel': 'VehicleScreen',
    'Screen | text and linework': 'VehicleScreen',
    'Screen | map streets': 'VehicleScreen',
    'Display | main map avenues': 'VehicleScreen',
    'Display | warm position marker': 'VehicleScreen',
    'Display | authored vehicle photograph': 'VehicleScreen',
    'Headlamp | optical polycarbonate': 'VehicleGlass',
    'Headlamp | satin reflector chambers': 'VehicleProjector',
    'Safety | buckle release': 'VehicleTrimBlack',
    'Presentation plate | warm ivory ink': 'VehiclePlate',
}
LAMP_MATERIALS = {
    'VehicleHeadlamp': {'channel': 'head', 'color': (0.92, 0.93, 0.96, 1.0), 'emissive': (0.92, 0.93, 1.0), 'strength': 2.0},
    'VehicleTaillamp': {'channel': 'tail', 'color': (0.45, 0.02, 0.02, 1.0), 'emissive': (0.8, 0.02, 0.02), 'strength': 2.0},
}
# Emitter surfaces, assigned a lamp channel material after curve conversion.
LAMP_OBJECTS = {
    'VehicleHeadlamp': (
        'Front lightbar | curved white light guide',
        'Front lamp wing | optical emitter',
        'Headlamp optical cell',
    ),
    'VehicleTaillamp': (
        'Rear lightbar red optical guide',
    ),
}

# ----------------------------------------------------------------------------
# detail tiers
# ----------------------------------------------------------------------------

# Micro-detail that no runtime tier draws: perforations, fasteners, belts,
# printed text, the authored photograph plane. Runs at every LOD.
ALWAYS_DROP = (
    'tactile dots', 'rotor drilling', 'concealed hub bolt', 'shoulder sipes',
    'USB-C', 'Window switch', 'woven strap', 'Seat rail bolt',
    'buckle stalk', 'buckle receiver', 'red buckle release', 'tongue slot',
    'latch tongue', 'guide pivot', 'concealed retractor', 'shoulder guide',
    'retracted woven webbing', 'photograph plane', 'recessed optical lens',
    'rear camera lens', 'fender camera lens', 'cabin camera', 'stop button',
    'dome light', 'release lever recess', 'manual release lever',
    'perforated speaker grille', 'flush exterior door release',
    'tire valve', 'cap seam', 'tie-down', 'floor pull', 'hatch latch receiver',
    'striker loop', 'wiring bellows', 'presentation plate', 'sill step plate',
    'Sill step plate', 'Rear plate inset',
    # Door/hatch actuator hardware: DAMPED_TRACK-constrained to the moving
    # attachments, so they drag when a pivot rotates, and the runtime cannot
    # articulate them. Pocket-mounted detail; invisible at every tier.
    'lift actuator', 'gas spring', 'moving attachment', 'fixed attachment',
    'actuator housing', 'sliding polished rod', 'hinge bracket', 'hinge pin',
)
# Interior fittings the near tier still reads through the glass; dropped at
# mid, where the cabin is one dark mass behind the panes.
MID_DROP = ALWAYS_DROP + (
    'actuator', 'Belt ', 'seat rail', 'lower rail', 'sliding rail',
    'pedestal', 'pocket', 'stitching', 'perimeter seam', 'recessed seam',
    'cushion channel', 'cushion piping', 'double stitch', 'upholstery seam',
    'warm ambient blade', 'dark structural liner', 'ivory inset',
    'dark upper ledge', 'sculpted armrest', 'lower storage pocket',
    'formed inner belt cap', 'textured inner liner', 'formed continuous side liner',
    'upper aperture landing', 'padded front bulkhead', 'lined floor',
    'removable load floor', 'load floor binding', 'rubber aperture seal',
    'rear acoustic bulkhead', 'air channel', 'mounting stem', 'bezel',
    'vehicle visualization card', 'A-pillar interior trim',
    'rear shelf', 'inboard closure',
)
# The far tier is silhouette + glass + wheels + lamps; the whole cabin goes.
FAR_DROP = MID_DROP + (
    'Seat L', 'Seat R', 'Seat_', 'Dashboard', 'Display', 'Screen',
    'Overhead', 'Headliner', 'Cargo', 'Cabin', 'carpet', 'visor',
    'Door L |', 'Door R |', 'rollover', 'closed perimeter returns',
    # NOT 'aero': on this study the wheel's visible FACE is the "solid aero
    # disc", so dropping the whole keyword left the far tier a hollow tyre ring
    # you could see the road through. The disc stays; what goes is the
    # "continuous ten-panel aero cover" over it, which is 91k of the wheel's
    # 107k triangles on its own - detail no 60 m read resolves, and enough to
    # stop the tier's single reduction pass from reaching its target.
    'ten-panel aero cover', 'ventilation slit',
    'wheel barrel', 'center cap', 'rotor',
    'brake', 'wheelwell', 'Wheelwell', 'underbody', 'Battery undertray',
    'reflector chamber', 'optical cell', 'lamp wing', 'tapered sealed housing',
    'wiper',
)

# Curves kept and converted to meshes; every other CURVE/FONT object (seams,
# stitching, lettering, plate ornament) is dropped at every tier.
KEEP_CURVES = (
    'Front lightbar | curved white light guide',
    'Front lightbar | curved black optical housing',
    'Rear lightbar red optical guide',
    'Rear lightbar deep housing',
    'Front lamp wing | optical emitter',
    'Concealed windshield wiper',
    'Door glass encapsulation',
)

# Per-tier triangle budgets (the assembly verifier's ceiling is
# 165160/22000/6000; aim under them with headroom for the bake's merges).
TRI_BUDGET = {0: 90000, 1: 19500, 2: 5700}
# What scripts/verify/verify-generated-vehicle-assembly.py refuses outright. The
# budgets above aim under these with headroom; crossing one of these is a failed
# build, not a tier to ship.
TRI_CEILING = {0: 165160, 1: 22000, 2: 6000}

# Triangles allowed to ONE glazing mesh per tier. Glass is the other part of
# this body that a proportional cut ruins out of proportion to its size: the
# canopy is 6% of the far tier's shell, but it is the greenhouse, and a
# collapsed one reads as a torn, spiky roof from any angle. Holding it at the
# count the previous export happened to leave it (164 for the canopy, 162 a
# pane) costs about 170 triangles and buys the whole read back.
GLAZING_MATERIAL = 'VehicleGlass'
GLAZING_TIER_TRIANGLES = {0: None, 1: None, 2: 170}
# The lamp lenses share the glazing material (the fold above puts the headlamp's
# optical polycarbonate on VehicleGlass), and they are not the greenhouse: held
# at the canopy's budget they cost the far tier 340 triangles for two 0.3 m
# strips, which is shell the silhouette wants back.
GLAZING_EXCLUDE = ('Headlamp', 'lightbar', 'lamp')

# Triangles allowed to ONE wheel subtree per tier, reduced in a single pass and
# then held out of the body's budget passes.
#
# `decimate_to_budget` cuts the biggest meshes by up to 60% and repeats up to
# six times, which is right for a body panel and wrong for a tyre: each pass
# re-runs the collapse on already-collapsed geometry, so the far tier's tread
# came out an oval 0.90-1.02 m across (against 0.80) that sank 114 mm under the
# road, and the four wheels landed at different counts. Reducing a wheel ONCE,
# to a fixed target, keeps the tread round and the four sides identical; the
# body absorbs the rest of the tier's budget.
#
# The far target is measured, not guessed: reduced to 360 the four wheels came
# out at 0.814-0.960 m across with one sunk 128 mm; at 700 all four hold
# 0.802-0.810 m (against 0.8012 in the study) and sit within 5 mm of the road.
WHEEL_TIER_TRIANGLES = {0: 7000, 1: 1200, 2: 700}


def subtree(root):
    return [root] + descendants(root)


def descendants(node):
    out = []
    stack = list(node.children)
    while stack:
        n = stack.pop()
        out.append(n)
        stack.extend(n.children)
    return out


def delete_objects(objects):
    for o in objects:
        try:
            bpy.data.objects.remove(o, do_unlink=True)
        except Exception:
            pass


def world_bounds(objects):
    mn = Vector((float('inf'),) * 3)
    mx = Vector((-float('inf'),) * 3)
    for o in objects:
        if o.type != 'MESH':
            continue
        for corner in o.bound_box:
            p = o.matrix_world @ Vector(corner)
            for i in range(3):
                mn[i] = min(mn[i], p[i])
                mx[i] = max(mx[i], p[i])
    return mn, mx


def tri_count(objects):
    deps = bpy.context.evaluated_depsgraph_get()
    total = 0
    for o in objects:
        if o.type != 'MESH':
            continue
        ev = o.evaluated_get(deps)
        m = ev.to_mesh()
        if m:
            m.calc_loop_triangles()
            total += len(m.loop_triangles)
        ev.to_mesh_clear()
    return total


def apply_drop(objects, patterns):
    doomed = [o for o in objects
              if o.type in ('MESH', 'CURVE', 'FONT', 'SURFACE')
              and o.name not in CONTRACT_NODES
              and not any(p in o.name for p in KEEP_CURVES)
              and any(p.lower() in o.name.lower() for p in patterns)]
    delete_objects(doomed)
    return [root for root in objects if root not in doomed]


def convert_curves(objects):
    for o in list(objects):
        if o.type not in ('CURVE', 'FONT', 'SURFACE'):
            continue
        if not any(k in o.name for k in KEEP_CURVES):
            bpy.data.objects.remove(o, do_unlink=True)
            continue
        o.data.resolution_u = 2
        if hasattr(o.data, 'bevel_resolution'):
            o.data.bevel_resolution = 0
        bpy.ops.object.select_all(action='DESELECT')
        o.select_set(True)
        bpy.context.view_layer.objects.active = o
        try:
            bpy.ops.object.convert(target='MESH')
        except RuntimeError:
            pass
        o.select_set(False)


def fold_materials():
    """Rename/merge the study's materials onto the canonical fleet set."""
    for src_name, dst_name in MATERIAL_MAP.items():
        src = bpy.data.materials.get(src_name)
        if not src:
            continue
        dst = bpy.data.materials.get(dst_name)
        if dst is None:
            src.name = dst_name
            continue
        for o in bpy.data.objects:
            if o.type not in ('MESH', 'CURVE', 'FONT', 'SURFACE'):
                continue
            for i, slot in enumerate(o.material_slots):
                if slot.material == src:
                    slot.material = dst
        bpy.data.materials.remove(src)
    for name, spec in LAMP_MATERIALS.items():
        mat = bpy.data.materials.get(name)
        if mat is None:
            mat = bpy.data.materials.new(name)
        mat.use_backface_culling = True  # lamp backs must not glow
        mat['installedLampChannel'] = spec['channel']
        mat.diffuse_color = spec['color']
        if not mat.use_nodes:
            mat.use_nodes = True
        bsdf = next((n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
        if bsdf:
            bsdf.inputs['Base Color'].default_value = spec['color']
            if 'Emission Color' in bsdf.inputs:
                bsdf.inputs['Emission Color'].default_value = (*spec['emissive'], 1.0)
                bsdf.inputs['Emission Strength'].default_value = spec['strength']


def assign_lamp_objects():
    for name, prefixes in LAMP_OBJECTS.items():
        mat = bpy.data.materials.get(name)
        for o in bpy.data.objects:
            if o.type != 'MESH':
                continue
            if any(p in o.name for p in prefixes):
                o.data.materials.clear()
                o.data.materials.append(mat)


def restructure(root):
    """Rename the rig to the runtime contract and reorient the body.

    Anchors (seats, plates) are authored against the ORIGINAL frame before the
    root is rotated: they are parented under the root and ride the same
    reorientation as everything else.
    """
    # The study drives its actuator hardware with DAMPED_TRACK constraints;
    # any constrained object left in the file follows its target when a pivot
    # rotates here. glTF has no constraints, so bake every object's rest pose.
    for o in descendants(root):
        o.constraints.clear()

    # --- measure the original frame first -----------------------------------
    meshes = [o for o in descendants(root) if o.type == 'MESH']
    body_min, body_max = world_bounds(meshes)
    seat_anchor = {}
    for anchor, (cushion_name, back_name) in SEAT_PARTS.items():
        cushion = bpy.data.objects.get(cushion_name)
        back = bpy.data.objects.get(back_name)
        if cushion:
            mn, mx = world_bounds([cushion])
            seat_anchor[anchor + '_Base'] = Vector(((mn.x + mx.x) / 2, (mn.y + mx.y) / 2, mx.z))
        if back:
            mn, mx = world_bounds([back])
            seat_anchor[anchor + '_Headrest0'] = Vector(((mn.x + mx.x) / 2, (mn.y + mx.y) / 2, mx.z - 0.05))

    # --- anchors, in the unrotated study frame, parented to the root --------
    def add_anchor(name, position):
        empty = bpy.data.objects.new(name, None)
        empty.empty_display_size = 0.08
        bpy.context.scene.collection.objects.link(empty)
        empty.parent = root
        empty.matrix_world = Matrix.Translation(position)
        return empty

    for name, position in seat_anchor.items():
        add_anchor(name, position)
    # Study frame: -X is the nose. The FRONT plate hangs at the min-x face,
    # the REAR plate at max-x, both at bumper height.
    add_anchor('PlateFront', Vector((body_min.x + 0.01, 0.0, 0.55)))
    add_anchor('PlateRear', Vector((body_max.x - 0.01, 0.0, 0.55)))

    # --- wheels: rename steer pivots, insert a spin child -------------------
    for old, new in WHEEL_RENAME.items():
        node = bpy.data.objects.get(old)
        if not node:
            raise RuntimeError(f'missing wheel pivot {old}')
        node.name = new
        wheel_children = list(node.children)  # before spin joins the list
        spin = bpy.data.objects.new(new + '_Spin', None)
        spin.empty_display_size = 0.1
        bpy.context.scene.collection.objects.link(spin)
        spin.parent = node
        spin.matrix_parent_inverse = Matrix.Identity(4)
        spin.matrix_basis = Matrix.Identity(4)
        # Evaluate now: assigning a child's matrix_world below reads the
        # parent's evaluated matrix, which is stale until the depsgraph runs.
        bpy.context.view_layer.update()
        for child in wheel_children:
            if any(k in child.name.lower() for k in WHEEL_STATIC):
                continue
            keep = child.matrix_world.copy()
            child.parent = spin
            child.matrix_world = keep

    # --- doors: rename pivots, carry the fitted open pose into extras -------
    for old, new in DOOR_RENAME.items():
        node = bpy.data.objects.get(old)
        if not node:
            raise RuntimeError(f'missing door pivot {old}')
        rotvec = Vector(node.get('open_rotation_vector_rad', (0, 0, 0)))
        limit = rotvec.length
        if limit < 1e-6:
            raise RuntimeError(f'{old}: no fitted open pose on the door pivot')
        axis = rotvec / limit
        # The study stored the open pose as a rotation in the pivot's own
        # rest frame (identity rest rotation), and the glTF exporter
        # conjugates every node's local transform by the same +Y-up basis
        # change - so the axis the runtime must rotate about, expressed in
        # the exported node's local frame, is the study axis with the same
        # basis change applied: (x, y, z)_blender -> (x, z, -y).
        node.name = new
        node['hingeAxis'] = [round(axis.x, 6), round(axis.z, 6), round(-axis.y, 6)]
        node['openLimitRadians'] = round(limit, 6)
        node['openSign'] = 1
        # A butterfly door's pane is structural: it never drops into the door.
        node['windowDropM'] = 0.0
        node['motion'] = 'authored butterfly open pose, carried as a hinge axis'
    for old, new in WINDOW_RENAME.items():
        node = bpy.data.objects.get(old)
        if node:
            node.name = new

    # --- hatch -> Trunk -------------------------------------------------------
    # The lid contract is hingeAxis 'X': the runtime swings lids about the
    # node's local X. The hatch hinge line is transverse (Blender +/-Y), which
    # the root reorientation carries onto the game's lateral X only if the
    # pivot's local X points down the study's lateral axis - so the empty gets
    # a +90 degree local turn about the study's up axis before the reorient.
    hatch = bpy.data.objects.get(HATCH_NODE)
    if hatch:
        hatch.name = 'Trunk'
        hatch['hingeAxis'] = 'X'
        hatch['openSign'] = -1
        hatch['openLimitRadians'] = 0.95
        saved = {c: c.matrix_world.copy() for c in descendants(hatch)}
        hatch.rotation_mode = 'XYZ'
        hatch.rotation_euler = (0, 0, math.radians(90))
        bpy.context.view_layer.update()
        for c, m in saved.items():
            c.matrix_world = m
        bpy.context.view_layer.update()

    # --- reorient: -X nose -> +Z after the glTF +Y-up conversion ------------
    root.name = BODY_KEY
    root.rotation_mode = 'XYZ'
    root.rotation_euler = (0, 0, math.radians(90))
    bpy.context.view_layer.update()

    for old, new in SEAT_RENAME.items():
        node = bpy.data.objects.get(old)
        if node:
            node.name = new

    # Last, with the body in its shipped orientation: the wheel pivots must
    # rest in the vehicle's own frame for the bake and the hero to roll them,
    # and the front axle takes the smaller tyre the study's subject runs.
    normalize_wheel_pivots()
    stagger_front_axle()
    return body_min, body_max


def normalize_wheel_pivots():
    """Leave every wheel pivot a pure translation, with nothing drawn moved.

    The runtime's wheel contract is not merely "a node with the right name":
    it is a pivot whose REST FRAME IS THE VEHICLE'S OWN. The overlay bakes a
    wheel's geometry into its spin pivot's frame and redraws it at the pivot's
    position under a fresh `rotation.x` (roll) and `rotation.y` (steer), and
    the hero instance rolls `Wheel_*_Spin.rotation.x` in place - so any rest
    rotation or scale left on a pivot is DISCARDED by the bake and MISREAD as
    the roll axis by the hero.

    The study hangs each wheel off an empty that is turned onto its own axle
    (+90 degrees) and scaled ~1.17 to size the tyre, and the root reorient
    above adds another quarter turn. Baked, that shipped a wheel rotated 90
    degrees (tread facing the flank, the rim proud of the wing, the solved
    steer limit collapsed to zero) at 0.342 m instead of 0.400 m - hovering
    58 mm over the road and under-rolling by 15% against its ground speed.

    Pushing the rotation and the scale down into the wheel's own meshes keeps
    every vertex exactly where the study put it and leaves the pivots the pure
    translations both consumers read.
    """
    for name in WHEEL_RENAME.values():
        steer = bpy.data.objects.get(name)
        spin = bpy.data.objects.get(name + '_Spin')
        if not steer or not spin:
            raise RuntimeError(f'missing wheel pivot {name}')
        # Only the DIRECT children need restoring: everything deeper keeps its
        # own local basis and rides its parent unmoved.
        for pivot in (steer, spin):
            keep = {c: c.matrix_world.copy() for c in pivot.children}
            pivot.matrix_world = Matrix.Translation(pivot.matrix_world.translation)
            bpy.context.view_layer.update()
            for child, m in keep.items():
                child.matrix_world = m
            bpy.context.view_layer.update()
        for pivot in (steer, spin):
            _, rotation, scale = pivot.matrix_world.decompose()
            if abs(rotation.angle) > 1e-5 or (scale - Vector((1, 1, 1))).length > 1e-5:
                raise RuntimeError(f'{pivot.name}: pivot rest is not a pure translation '
                                   f'(angle {rotation.angle:.6f}, scale {tuple(scale)})')


def stagger_front_axle():
    """Resize the front wheels about their hubs and settle them on the road.

    Scaling a wheel about its hub lifts its contact patch by
    `hub * (1 - radial)`, so the whole corner drops by the same amount and the
    smaller tyre still meets y = 0. The hub follows the tyre: a wheel that is
    43 mm smaller in diameter rides 22 mm lower, which is what the runtime then
    reads back as this axle's own radius.

    Nothing about the BODY moves. The front arch was authored around the study's
    single wheel size, so the gap over the front tyre opens up by the same
    43 mm; closing it is a change to the study's bodywork, not to its rig.
    """
    for name in FRONT_WHEELS:
        steer = bpy.data.objects.get(name)
        spin = bpy.data.objects.get(name + '_Spin')
        if not steer or not spin:
            raise RuntimeError(f'missing wheel pivot {name}')
        hub = spin.matrix_world.translation.copy()
        # Blender axes at this point: x is the axle, y/z are the wheel's plane.
        resize = (Matrix.Translation(hub)
                  @ Matrix.Diagonal((FRONT_AXIAL_SCALE, FRONT_RADIAL_SCALE, FRONT_RADIAL_SCALE, 1.0))
                  @ Matrix.Translation(-hub))
        drop = Vector((0, 0, -hub.z * (1 - FRONT_RADIAL_SCALE)))
        settle = Matrix.Translation(drop)
        # The pivots move first - the wheel's meshes ride them, so their
        # absolute placement has to be written afterwards or the drop lands
        # twice. `spin` sits at `steer` with an identity basis, so moving the
        # steer pivot moves both.
        keep = {child: child.matrix_world.copy()
                for pivot in (steer, spin) for child in pivot.children if child is not spin}
        steer.matrix_world = Matrix.Translation(steer.matrix_world.translation + drop)
        bpy.context.view_layer.update()
        for child, world in keep.items():
            child.matrix_world = settle @ resize @ world
        bpy.context.view_layer.update()
        for pivot in (steer, spin):
            _, rotation, scale = pivot.matrix_world.decompose()
            if abs(rotation.angle) > 1e-5 or (scale - Vector((1, 1, 1))).length > 1e-5:
                raise RuntimeError(f'{pivot.name}: staggering disturbed the pivot rest')


def apply_all_modifiers(objects):
    """Bake every modifier stack into base mesh data before any join.

    `object.join` keeps only the active object's modifiers, and the exporter
    runs with export_apply=False - so a bevel or solidify that is not applied
    here silently vanishes from the shipped mesh while still being counted in
    the budget. Converting each mesh to itself bakes the stack in place.
    """
    for o in objects:
        if o.type != 'MESH' or not o.modifiers:
            continue
        bpy.ops.object.select_all(action='DESELECT')
        o.select_set(True)
        bpy.context.view_layer.objects.active = o
        try:
            bpy.ops.object.convert(target='MESH')
        except RuntimeError:
            pass
        o.select_set(False)


def join_by_material(subtree_root, exclude=None, keep_separate=()):
    """Join every mesh inside one rig subtree into a single object.

    Multi-material objects export as one mesh with one primitive per slot;
    the GLTFLoader splits those back into per-material meshes, which is
    exactly what the bake merges on. Contract nodes (Window_*) keep their
    names and are never absorbed. `exclude` holds subtree roots whose
    contents belong to a different join (the articulated rig).
    """
    excluded = set()
    for other in (exclude or []):
        excluded.update(descendants(other))
        excluded.add(other)
    held = set(keep_separate)
    meshes = [o for o in descendants(subtree_root)
              if o.type == 'MESH' and o.name not in CONTRACT_NODES and o not in excluded and o not in held]
    if len(meshes) < 2:
        return
    bpy.ops.object.select_all(action='DESELECT')
    for o in meshes:
        o.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    bpy.ops.object.join()
    meshes[0].name = f'{subtree_root.name} | joined'


def glazing_meshes(root):
    """Every mesh that is ENTIRELY glazing, wheels excluded.

    Every slot, not any slot: the study's shell object carries a glass slot
    among a dozen others, and treating it as glazing held the whole body out of
    the join and out of the tier reduction - a 1.23M-triangle near tier that
    the assembly ceiling check refused.
    """
    wheel_meshes = set()
    for name in WHEEL_RENAME.values():
        node = bpy.data.objects.get(name)
        if node:
            wheel_meshes.update(descendants(node))
    return [o for o in descendants(root)
            if o.type == 'MESH' and o not in wheel_meshes and o.material_slots
            and not any(key.lower() in o.name.lower() for key in GLAZING_EXCLUDE)
            and all(slot.material and slot.material.name == GLAZING_MATERIAL for slot in o.material_slots)]


def reduce_glazing(level, root):
    """Reduce each glazing mesh once and return the meshes to protect."""
    target = GLAZING_TIER_TRIANGLES.get(level)
    protected = glazing_meshes(root)
    if not target:
        # No target, no protection: a tier that does not hold its glass back
        # must let the budget pass reach it, or the tier cannot be met at all.
        return []
    for o in protected:
        total = tri_count([o])
        if total <= target:
            continue
        mod = o.modifiers.new('glass-lod', 'DECIMATE')
        mod.decimate_type = 'COLLAPSE'
        mod.ratio = max(0.002, target / total)
        bpy.context.view_layer.objects.active = o
        try:
            bpy.ops.object.modifier_apply(modifier=mod.name)
        except RuntimeError:
            if mod.name in o.modifiers:
                o.modifiers.remove(mod)
    return protected


def reduce_wheels(level):
    """Reduce each wheel subtree once and return the meshes to protect."""
    target = WHEEL_TIER_TRIANGLES.get(level)
    protected = []
    for name in WHEEL_RENAME.values():
        node = bpy.data.objects.get(name)
        if not node:
            continue
        meshes = [o for o in descendants(node) if o.type == 'MESH']
        protected.extend(meshes)
        if not target:
            continue
        total = tri_count(meshes)
        if total <= target:
            continue
        for o in meshes:
            mod = o.modifiers.new('wheel-lod', 'DECIMATE')
            mod.decimate_type = 'COLLAPSE'
            # No generous lower bound here: a clamp that silently misses the
            # target is how the far tier shipped four 1078-triangle wheels
            # against a 700 budget. If a subtree is so heavy that the ratio
            # goes this low, the tier's DROP LIST is what needs fixing.
            mod.ratio = max(0.002, target / total)
            bpy.context.view_layer.objects.active = o
            try:
                bpy.ops.object.modifier_apply(modifier=mod.name)
            except RuntimeError:
                if mod.name in o.modifiers:
                    o.modifiers.remove(mod)
    return protected


def decimate_to_budget(objects, budget, protected=(), floor=120, passes=3):
    """Reduce a tier to its budget PROPORTIONALLY, in as few passes as possible.

    The loop this replaces took up to 60% off the largest mesh and repeated, so
    the biggest thing in the tier paid for the whole overage and each pass ran
    the collapse over already-collapsed geometry. Both halves of that hurt: the
    compounding is what left the far tier's tyres 0.91-1.04 m across with one
    sunk 114 mm, and the biggest-first order is what left the body shell - the
    silhouette the far tier exists to draw - at 176 triangles measuring 4.84 m
    long against an authored 4.53 m, while its doors kept theirs.

    Cutting every unprotected mesh by the SAME FRACTION keeps the authored
    balance between shell, doors and glass, and reaches the budget in one pass.
    A protected mesh (`reduce_wheels` has already reduced those once, to a
    target their shape survives) still counts against the budget. `floor` is
    the fewest triangles any mesh is cut to; a tier that cannot reach its
    budget without breaking that floor is reported by the caller's ceiling
    check rather than paid for with shape.
    """
    held = set(protected)
    total = 0
    for _pass in range(passes):
        counts = []
        total = 0
        deps = bpy.context.evaluated_depsgraph_get()
        for o in objects:
            if o.type != 'MESH':
                continue
            ev = o.evaluated_get(deps)
            m = ev.to_mesh()
            if m:
                m.calc_loop_triangles()
            n = len(m.loop_triangles) if m else 0
            ev.to_mesh_clear()
            counts.append((n, o))
            total += n
        if total <= budget:
            return total
        pool = sum(n for n, o in counts if o not in held)
        room = budget - (total - pool)
        if pool <= 0 or room <= 0:
            return total
        scale = room / pool
        cut = False
        for n, o in counts:
            if o in held or n <= floor:
                continue
            target = max(floor, n * scale)
            if target >= n:
                continue
            mod = o.modifiers.new('lod', 'DECIMATE')
            mod.decimate_type = 'COLLAPSE'
            mod.ratio = target / n
            bpy.context.view_layer.objects.active = o
            try:
                bpy.ops.object.modifier_apply(modifier=mod.name)
                cut = True
            except RuntimeError:
                if mod.name in o.modifiers:
                    o.modifiers.remove(mod)
        if not cut:
            break
    return total


def exported_triangles(path):
    """Triangles the shipped GLB actually carries - what the assembly verifier
    counts - rather than the pre-export evaluated count, which drifts on ngon
    tessellation and curve resolution."""
    raw = Path(path).read_bytes()
    (chunk_size,) = struct.unpack_from('<I', raw, 12)
    gltf = json.loads(raw[20:20 + chunk_size])
    total = 0
    for mesh in gltf.get('meshes', []):
        for primitive in mesh['primitives']:
            accessor = primitive.get('indices', primitive['attributes']['POSITION'])
            total += gltf['accessors'][accessor]['count'] // 3
    return total


def flatten_transmission():
    """Export transmissive materials as alpha-blended glass, never KHR_materials_transmission.

    three renders a transmissive material with a ViewportTextureNode that copies the framebuffer
    mid-pass; built during the environment probe bake it ends a render pass the bake ends again,
    and the boot GPU gate rejects the boot ("RenderPassEncoder was already ended"). The same rule
    as scripts/assets/export-skill-vehicle-lods.py: zero the transmission, carry the see-through as
    alpha (alpha = 1 - 0.75 * transmission), blended, both faces kept. Nothing else changes.
    """
    flattened = []
    for material in bpy.data.materials:
        if not material.use_nodes or not material.node_tree:
            continue
        for node in material.node_tree.nodes:
            if node.type != 'BSDF_PRINCIPLED':
                continue
            weight = node.inputs.get('Transmission Weight')
            if weight is None or weight.is_linked or weight.default_value <= 0.0:
                continue
            alpha = node.inputs.get('Alpha')
            if alpha is not None and not alpha.is_linked:
                alpha.default_value = min(alpha.default_value, 1.0 - 0.75 * weight.default_value)
                material.surface_render_method = 'BLENDED'
            weight.default_value = 0.0
            material.use_backface_culling = False
            flattened.append(material.name)
    return sorted(set(flattened))


def assert_runtime_glb(path):
    """Fail the export if the GLB still carries transmission (the boot GPU gate rejects it)."""
    raw = Path(path).read_bytes()
    (chunk_size,) = struct.unpack_from('<I', raw, 12)
    gltf = json.loads(raw[20:20 + chunk_size])
    if 'KHR_materials_transmission' in gltf.get('extensionsUsed', []):
        raise RuntimeError(f'{path}: KHR_materials_transmission survived the export')
    for m in gltf.get('materials', []):
        if 'KHR_materials_transmission' in (m.get('extensions') or {}):
            raise RuntimeError(f'{path}: material {m.get("name")} still transmits')


def export_glb(path, objects):
    bpy.ops.object.select_all(action='DESELECT')
    for o in objects:
        o.select_set(True)
    # Already in the runtime frame (rotation only, correct side names): stamp it
    # the way scripts/assets/vehicle_frame.py stamps a converted fleet body.
    bpy.context.scene[FRAME_KEY] = FRAME
    bpy.ops.export_scene.gltf(
        filepath=str(path),
        use_selection=True,
        export_extras=True,
        export_yup=True,
        export_apply=False,
        export_animations=False,
        export_cameras=False,
        export_lights=False,
        export_skins=False,
        export_morph=False,
    )


def build_lod(level, drops, budget):
    bpy.ops.wm.open_mainfile(filepath=str(SOURCE))
    scene = next(s for s in bpy.data.scenes if s.name.startswith('CYBERCAB'))
    bpy.context.window.scene = scene
    root = next(o for o in scene.objects if o.name.startswith('CYBERCAB |'))
    delete_objects([o for o in scene.objects if o not in subtree(root)])

    body_min, body_max = restructure(root)
    fold_materials()
    convert_curves(subtree(root))
    assign_lamp_objects()
    apply_drop(descendants(root), drops)
    apply_all_modifiers(subtree(root))

    rig_roots = [bpy.data.objects.get(n) for n in
                 list(WHEEL_RENAME.values()) + list(DOOR_RENAME.values()) + ['Trunk']]
    rig_roots = [r for r in rig_roots if r]
    for r in rig_roots:
        join_by_material(r)
    # Root statics join LAST and never absorb a rig subtree: a door panel
    # merged into the statics would bake shut and never articulate.
    # Glazing stays OUT of the shell join: merged into the shell it is cut with
    # it, and the canopy cannot survive the far tier's share. It costs nothing
    # at runtime - the bake merges every static by material anyway.
    glazing = glazing_meshes(root)
    join_by_material(root, exclude=rig_roots, keep_separate=glazing)
    objects = subtree(root)

    protected = reduce_wheels(level) + reduce_glazing(level, root)
    decimate_to_budget(objects, budget, protected)
    flatten_transmission()
    out = OUT / f'{BODY_KEY}-lod{level}.glb'
    export_glb(out, objects)
    assert_runtime_glb(out)
    triangles = exported_triangles(out)
    if triangles > TRI_CEILING[level]:
        raise RuntimeError(f'lod{level}: {triangles} triangles exceeds the assembly '
                           f'ceiling of {TRI_CEILING[level]}')
    return {'file': out.name, 'triangles': triangles,
            'lengthM': round(body_max.x - body_min.x, 4)}


if __name__ == '__main__':
    results = []
    for level, drops in ((0, ALWAYS_DROP), (1, MID_DROP), (2, FAR_DROP)):
        results.append(build_lod(level, drops, TRI_BUDGET[level]))
    report = {'body': BODY_KEY, 'vehicleId': VEHICLE_ID,
              'lengthM': results[0]['lengthM'], 'wheelRadiusM': 0.4,
              # The body's reference wheel is the rear; the front axle is
              # staggered smaller and the runtime reads its radius per pivot.
              'wheelRadiusFrontM': round(0.4 * FRONT_RADIAL_SCALE, 4),
              'typeIds': ['roboCab'], 'generator': 'scripts/assets/build-robo-cab-v1.py',
              'sourceSha256': hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
              'lods': []}
    for r in results:
        raw = (OUT / r['file']).read_bytes()
        report['lods'].append({'file': r['file'], 'triangles': r['triangles'],
                               'bytes': len(raw), 'sha256': hashlib.sha256(raw).hexdigest(),
                               'doors': ['FL', 'FR'], 'lids': ['Trunk'],
                               'wheels': ['FL', 'FR', 'RL', 'RR'],
                               'seats': ['FL', 'FR']})
    (OUT / 'build-report.json').write_text(json.dumps(report, indent=2) + '\n')
    print('BUILT', json.dumps(report, indent=1))
