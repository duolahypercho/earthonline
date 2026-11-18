"""Runtime contract checks on one exported skill-vehicle GLB, read back from disk.

    blender -b --factory-startup -P scripts/assets/check-skill-vehicle-glb.py -- \
        <body-lodN.glb> <lod 0|1|2> <body key> <out.json>

The body key selects the same `BODIES` entry export-skill-vehicle-lods.py exported it with, so
the required nodes follow the body (two-door bodies, barn doors with no tailgate or rear glass,
a sliding door, a cab-over body with no bonnet or boot lid, the taxi roof sign). Checked:

* triangles under the tier cap (147,000 / 22,000 / 6,000);
* no morph targets and no KHR_materials_transmission (both break the runtime);
* every material carries a runtime name (`Vehicle*`), with VehiclePaint (or the body's `paintAs` / `paintRename`
  livery name) and VehicleHeadlamp present;
* the nodes the runtime binds by name exist;
* each plate anchor stands 3-9 mm proud of the surface straight behind it;
* every articulated node carries the hinge (or slide) extras the runtime reads.

Exit status is 0 when the GLB passes and 1 when it does not; the JSON records why.
"""
import importlib.util
import json
import sys
from pathlib import Path

import bpy
from mathutils import Vector
from mathutils.bvhtree import BVHTree

args = sys.argv[sys.argv.index('--') + 1:]
glb, lod, body, outp = args[0], int(args[1]), args[2], args[3]
spec = importlib.util.spec_from_file_location('skill_export', Path(__file__).with_name('export-skill-vehicle-lods.py'))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)
cfg = exporter.BODIES[body]
cap = exporter.CAPS[lod]

raw = Path(glb).read_bytes()
doc = json.loads(raw[20:20 + int.from_bytes(raw[12:16], 'little')])
acc = doc['accessors']
mesh_tris = [sum((acc[p['indices']]['count'] if 'indices' in p else acc[p['attributes']['POSITION']]['count']) // 3
                 for p in m['primitives']) for m in doc['meshes']]
tris = sum(mesh_tris[n['mesh']] for n in doc['nodes'] if 'mesh' in n)
names = {n.get('name', '') for n in doc['nodes']}
mats = sorted(m.get('name', '') for m in doc.get('materials', []))
morphs = sum(len(p.get('targets', [])) for m in doc['meshes'] for p in m['primitives'])
ext = doc.get('extensionsUsed', [])

doors = tuple(cfg.get('doors', ('FL', 'FR', 'RL', 'RR')))
window_doors = tuple(cfg.get('windowDoors', doors))
slides = tuple(cfg.get('slideDoors') or ())
no_lids = bool(cfg.get('noLids'))
req = [f'Door_{c}' for c in doors] + ([] if no_lids else ['Hood']) + [f'Wheel_{c}' for c in ('FL', 'FR', 'RL', 'RR')]
req += ['PlateFront', 'PlateRear']
if not (cfg.get('noTrunk') or no_lids):
    req += ['Trunk']
if lod in (0, 1):
    req += [f'SlideDoor_{c}' for c in slides]
if lod == 1:
    req += [f'Panel_{c}' for c in doors] + [f'Window_{c}' for c in window_doors] + ([] if no_lids else ['HoodSkin'])
    req += [f'Panel_Slide{c}' for c in slides]
    if not (cfg.get('noTrunk') or no_lids):
        req += ['TrunkSkin']
if body.startswith('taxi'):
    req += ['TaxiSignBase', 'TaxiSign']
missing = [r for r in req if r not in names]

articulated = ('Door_FL', 'Door_FR', 'Door_RL', 'Door_RR', 'Hood') + tuple(f'SlideDoor_{c}' for c in slides)
extras = {n['name']: n.get('extras', {}) for n in doc['nodes'] if n.get('name') in articulated}


def extras_ok(name, e):
    if name == 'Hood':
        return e.get('hingeAxis') == 'X' and e.get('openSign') in (1, -1) and e.get('openLimitRadians', 0) > 0
    if name.startswith('SlideDoor_'):
        return e.get('doorType') == 'sliding' and e.get('slideTravelM', 0) > 0.5 and 'hingeAxis' not in e
    return (isinstance(e.get('hingeAxis'), list) and len(e['hingeAxis']) == 3 and e.get('openSign') in (1, -1)
            and e.get('openLimitRadians', 0) > 0)


extras_pass = {k: extras_ok(k, v) for k, v in extras.items()}

# plate anchors: node origin to the tier's surface straight behind it
for o in list(bpy.data.objects):
    bpy.data.objects.remove(o, do_unlink=True)
bpy.ops.import_scene.gltf(filepath=glb)
dg = bpy.context.evaluated_depsgraph_get()
others = [o for o in bpy.data.objects if o.type == 'MESH' and o.name.split('.')[0] not in ('PlateFront', 'PlateRear')
          and not o.name.split('.')[0].endswith('_Lining')]
trees = [(o, BVHTree.FromObject(o, dg), o.matrix_world.inverted()) for o in others]
plates = {}
for name, face in (('PlateFront', -1), ('PlateRear', 1)):
    p = bpy.data.objects.get(name)
    if p is None:
        plates[name] = None
        continue
    c = p.matrix_world.translation.copy()
    # Blender import: glTF Y-up back to Blender Z-up, so the car's length is Blender Y again
    best = None
    for o, tree, inv in trees:
        hit = tree.ray_cast(inv @ c, (inv.to_3x3() @ Vector((0, -face, 0))).normalized(), 1.0)
        if hit[0] is not None:
            d = (o.matrix_world @ hit[0] - c).length
            if best is None or d < best[0]:
                best = (d, o.name)
    plates[name] = dict(proudMm=round(best[0] * 1000, 2), onto=best[1]) if best else dict(proudMm=None)
plates_ok = all(v and v.get('proudMm') is not None and 3.0 <= v['proudMm'] <= 9.0 for v in plates.values())

paint_ok = ('VehiclePaint' in mats or (cfg.get('paintAs') or {}).get('name') in mats
            or cfg.get('paintRename') in mats)   # a livery body ships its paint under the livery name
res = dict(file=Path(glb).name, body=body, lod=lod, triangles=tris, cap=cap, underCap=tris <= cap, morphTargets=morphs,
           extensionsUsed=ext, transmission='KHR_materials_transmission' in ext,
           rawMaterials=[m for m in mats if not m.startswith('Vehicle')], paintMaterial=paint_ok,
           headlamp='VehicleHeadlamp' in mats, materials=mats, missingNodes=missing, plates=plates, platesOk=plates_ok,
           runtimeExtras=extras, runtimeExtrasOk=extras_pass)
res['pass'] = bool(res['headlamp'] and res['underCap'] and not morphs and not res['transmission'] and not res['rawMaterials']
                   and res['paintMaterial'] and not missing and plates_ok and all(extras_pass.values()))
Path(outp).write_text(json.dumps(res, indent=1) + '\n')
print('CHECK', json.dumps({k: res[k] for k in ('file', 'triangles', 'pass', 'missingNodes', 'plates', 'runtimeExtrasOk')}))
sys.exit(0 if res['pass'] else 1)
