# Mid-tier (runtime "mid", manifest lods[1]) for the sedan, derived from the
# full LOD1 export:
#   blender -b -P build_lod1_mid.py -- <out_dir> [max_triangles]
#
# The shipped lods[1] was the 62k full LOD1 while
# every other body's mid tier is 18-23k, and the sedan is the most common
# body in the parked row - so the mid tier alone put the frame 150% over its
# declared triangle budget. The procedural rebuild still cannot re-cut the
# body below level-0 resolution (the FuelDoor boolean, see build_lod2.py), so
# the mid tier is DERIVED from the verified full LOD1 the same way the far
# tier is: import it, drop the parts a 22-60 m eye cannot resolve (door
# furniture, engine and boot bays, character-line strips, dash vents), keep
# every panel, lamp, glass pane, mirror, handle and the cabin blocks (seats,
# dash, headliner) that show through the glass, then decimate the wheels at
# a fixed gentle ratio and scan the loft ratio down until the body lands in
# the 18-23k band its peers use. Node names and materials survive, so the
# runtime bake reads the file exactly like the other tiers.
import sys, os, time, logging
SP = os.path.dirname(os.path.abspath(__file__))
if SP not in sys.path: sys.path.insert(0, SP)
argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
OUT = argv[0] if argv else SP
TARGET_MAX = float(argv[1]) if len(argv) > 1 else 22000
SRC = f'{SP}/project-sedan-v1-lod1-full.glb'
import bpy
logging.getLogger('glTFExporter').setLevel(logging.WARNING)
logging.getLogger('glTFImporter').setLevel(logging.WARNING)

t0 = time.time()
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=SRC)
print(f'[lod1-mid] imported {len(bpy.data.objects)} objects from {os.path.basename(SRC)}', flush=True)

def tri_count():
    dg = bpy.context.evaluated_depsgraph_get(); total = 0
    for ob in bpy.data.objects:
        if ob.type != 'MESH': continue
        me = ob.evaluated_get(dg).data
        total += sum(len(p.vertices) - 2 for p in me.polygons)
    return total

MID_DROP = ('DoorArmrest', 'DoorCard', 'DoorInsert', 'DoorPull', 'EngineBlock', 'EngineCover', 'EngineBay',
            'TrunkBay', 'Vent', 'CharLine', 'Wheel_FL_Disc', 'Wheel_FR_Disc', 'Wheel_RL_Disc', 'Wheel_RR_Disc',
            'Firewall', 'Tunnel', 'RearShelfRib')
def is_glass(ob):
    return ob.type == 'MESH' and any(m and m.name == 'VehicleGlass' for m in ob.data.materials)
dropped = 0
for ob in list(bpy.data.objects):
    if any(ob.name.startswith(p) for p in MID_DROP):
        bpy.data.objects.remove(ob, do_unlink=True); dropped += 1
print(f'[lod1-mid] dropped {dropped} parts, tris={tri_count()}', flush=True)

table = []
dg = bpy.context.evaluated_depsgraph_get()
for ob in bpy.data.objects:
    if ob.type != 'MESH': continue
    me = ob.evaluated_get(dg).data
    table.append((sum(len(p.vertices) - 2 for p in me.polygons), ob.name))
table.sort(reverse=True)
print('[lod1-mid] heaviest', table[:14], flush=True)

added = []
# Wheels and glass keep fixed ratios (a round tyre, a flat pane); the scan
# spends the budget on the painted lofts and the trim.
WHEEL_RATIO = 0.30
GLASS_RATIO = 0.45
LAMP_RATIO = 0.45
def decimate(ratio, min_polys=120):
    global added
    for ob, d in added: ob.modifiers.remove(d)
    added = []
    for ob in bpy.data.objects:
        if ob.type != 'MESH' or len(ob.data.polygons) <= min_polys: continue
        if is_glass(ob): r = GLASS_RATIO
        elif ob.name.startswith('Wheel_'): r = WHEEL_RATIO
        elif ob.name.startswith(('Headlamp_', 'Taillamp_')): r = LAMP_RATIO
        else: r = ratio
        d = ob.modifiers.new('LODDecimate', 'DECIMATE'); d.ratio = r; added.append((ob, d))

chosen = None
for ratio in (0.6, 0.55, 0.5, 0.45, 0.4, 0.36, 0.33, 0.3, 0.27, 0.25, 0.22, 0.2):
    decimate(ratio)
    tris = tri_count()
    print(f'[lod1-mid] ratio={ratio} tris={tris}', flush=True)
    if tris <= TARGET_MAX:
        chosen = (ratio, tris); break
if chosen is None: chosen = (0.2, tri_count())
os.makedirs(OUT, exist_ok=True)
path = f'{OUT}/project-sedan-v1-lod1.glb'
bpy.ops.object.select_all(action='SELECT')
bpy.ops.export_scene.gltf(filepath=path, use_selection=True, export_apply=True, export_yup=True,
                          export_materials='EXPORT', export_format='GLB', export_animations=False, export_skins=False,
                          export_texcoords=True, export_normals=True)
print(f'[lod1-mid] exported ratio={chosen[0]} tris={chosen[1]} bytes={os.path.getsize(path)} in {time.time()-t0:.1f}s', flush=True)
