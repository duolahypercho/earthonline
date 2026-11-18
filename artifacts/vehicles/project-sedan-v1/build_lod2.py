# Far-tier (LOD2) for the sedan, derived from the shipped LOD1 export:
#   blender -b -P build_lod2.py -- <out_dir> [max_triangles]
#
# The shipped project-sedan-v1-lod2.glb was a byte copy
# of LOD1 (62k triangles), so the far ring spent 22 of 39 far draws on the
# heaviest mesh while every other body had a ~5k proxy. The procedural
# rebuild cannot make a far tier today (the FuelDoor boolean extraction fails
# below the level-0 loft resolution), so the far tier is DERIVED from the
# verified LOD1 file instead: import it, drop the cabin, lamp and trim detail
# the far ring cannot resolve, keep the lamp housings (a tail-light dot), the
# windscreen and rear glass (a glass tone) and the tyre + barrel (a wheel
# disc), add the dark cabin block the far tier uses so the glass reads as a
# volume, then decimate the lofts that are still heavy until the body lands
# in the 4-6k band the other far bodies use. Node names and materials survive
# the round trip, so the runtime bake (src/vehicles/hero-sedan-glb.js) reads
# the file exactly like the other tiers.
import sys, os, time, logging
SP = os.path.dirname(os.path.abspath(__file__))
if SP not in sys.path: sys.path.insert(0, SP)
argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
OUT = argv[0] if argv else SP
TARGET_MAX = float(argv[1]) if len(argv) > 1 else 6200
SRC = f'{SP}/project-sedan-v1-lod1-full.glb'
import bpy
logging.getLogger('glTFExporter').setLevel(logging.WARNING)
logging.getLogger('glTFImporter').setLevel(logging.WARNING)
from headless_lod_lists import LOD2_DROP, LOD2_KEEP

t0 = time.time()
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=SRC)
objects = [ob for ob in bpy.data.objects]
print(f'[lod2] imported {len(objects)} objects from {os.path.basename(SRC)}', flush=True)

def tri_count():
    dg = bpy.context.evaluated_depsgraph_get(); total = 0
    for ob in bpy.data.objects:
        if ob.type != 'MESH': continue
        me = ob.evaluated_get(dg).data
        total += sum(len(p.vertices) - 2 for p in me.polygons)
    return total

def base(name):
    # glTF import keeps the node name; a duplicate suffix (.001) never starts a name.
    return name

# The far ring is seen side-on: every glass pane stays (a glass tone per
# car is the acceptance), and the rim lip and hub go - the barrel face and
# the tyre are the wheel disc at that range.
FAR_DROP = LOD2_DROP + ('Wheel_FL_Lip', 'Wheel_FR_Lip', 'Wheel_RL_Lip', 'Wheel_RR_Lip',
                        'Wheel_FL_Hub', 'Wheel_FR_Hub', 'Wheel_RL_Hub', 'Wheel_RR_Hub')
def is_glass(ob):
    return ob.type == 'MESH' and any(m and m.name == 'VehicleGlass' for m in ob.data.materials)
dropped = 0
for ob in list(bpy.data.objects):
    n = base(ob.name)
    if any(n.startswith(k) for k in LOD2_KEEP) or is_glass(ob): continue
    if any(n.startswith(p) for p in FAR_DROP):
        bpy.data.objects.remove(ob, do_unlink=True); dropped += 1
print(f'[lod2] dropped {dropped} parts, tris={tri_count()}', flush=True)

# Dark cabin block (Blender frame: y negative = nose, z up), so the glass reads
# as a volume and not a hole through to the road.
interior = bpy.data.materials.get('VehicleInterior') or bpy.data.materials.get('VehicleHeadliner') or bpy.data.materials.get('VehicleTrimBlack')
# The previous block (1.40 x 2.20 x 0.36 at y 0.55, z 1.02)
# ran from the cowl to the deck and stood ~0.16 m proud of the raked windscreen
# and rear window and through the side glass, so every far sedan carried a
# black slab where its glass should reflect the sky. It now sits between the
# A and C pillars (glTF z -1.10..+0.10), 0.86-1.22 m up, 0.96 m wide - inside
# the side-glass extent (|x| 0.56-0.86, z -0.97..0.42) and under the roof.
bpy.ops.mesh.primitive_cube_add(size=1.0, location=(0, 0.50, 1.04))
cabin = bpy.context.active_object
cabin.name = 'CabinBlock'
cabin.scale = (0.96, 1.20, 0.36)
if interior: cabin.data.materials.append(interior)
# parent under the body root so the export keeps one hierarchy
roots = [ob for ob in bpy.data.objects if ob.parent is None and ob is not cabin]
if len(roots) == 1: cabin.parent = roots[0]

table = []
dg = bpy.context.evaluated_depsgraph_get()
for ob in bpy.data.objects:
    if ob.type != 'MESH': continue
    me = ob.evaluated_get(dg).data
    table.append((sum(len(p.vertices) - 2 for p in me.polygons), ob.name))
table.sort(reverse=True)
print('[lod2] heaviest', table[:12], flush=True)

added = []
# Wheels keep a fixed, gentler ratio so the tyre stays round; the scan below
# spends the budget on the lofts.
WHEEL_RATIO = 0.22
GLASS_RATIO = 0.25
def decimate(ratio, min_polys=160):
    global added
    for ob, d in added: ob.modifiers.remove(d)
    added = []
    for ob in bpy.data.objects:
        if ob.type != 'MESH' or len(ob.data.polygons) <= min_polys: continue
        r = GLASS_RATIO if is_glass(ob) else (WHEEL_RATIO if ob.name.startswith('Wheel_') else ratio)
        d = ob.modifiers.new('LODDecimate', 'DECIMATE'); d.ratio = r; added.append((ob, d))

chosen = None
for ratio in (0.3, 0.25, 0.2, 0.17, 0.15, 0.13, 0.12, 0.1, 0.09, 0.08, 0.07, 0.06):
    decimate(ratio)
    tris = tri_count()
    print(f'[lod2] ratio={ratio} tris={tris}', flush=True)
    if tris <= TARGET_MAX:
        chosen = (ratio, tris); break
if chosen is None: chosen = (0.06, tri_count())
os.makedirs(OUT, exist_ok=True)
path = f'{OUT}/project-sedan-v1-lod2.glb'
bpy.ops.object.select_all(action='SELECT')
bpy.ops.export_scene.gltf(filepath=path, use_selection=True, export_apply=True, export_yup=True,
                          export_materials='EXPORT', export_format='GLB', export_animations=False, export_skins=False,
                          export_texcoords=True, export_normals=True)
print(f'[lod2] exported ratio={chosen[0]} tris={chosen[1]} bytes={os.path.getsize(path)} in {time.time()-t0:.1f}s', flush=True)
