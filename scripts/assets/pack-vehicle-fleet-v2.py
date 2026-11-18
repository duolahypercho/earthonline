"""Consolidate compatible finishes without merging independent vehicle parts.

Run after build-vehicle-fleet-v2.py and before its verification/promotion gate.
The body, glass, tyres and functional lamp channels retain separate materials.
"""
import bpy
import hashlib
import json
import sys
from pathlib import Path

root=Path(__file__).resolve().parents[2]
args=sys.argv[sys.argv.index('--')+1:]
directory=Path(args[0]).expanduser().resolve()
repack='--repack' in args
if directory.is_relative_to(root): raise ValueError('Pack candidates outside the repository')
report=json.loads((directory/'build-report.json').read_text())
# Policy 2: the cabin keeps ONE lit interior finish (VehicleInterior). Policy 1
# folded it into the 0.012 trim black, so every cabin read as a black void
# through the glass. The mid and far overlay tiers still fold VehicleInterior
# by name.
# Policy 3: the interior may not cost a near-tier batch. Every overlay batch is
# one shared material, and the frame is bound by draw submission, so policy 2's
# two new finishes (VehicleInterior and the dial face VehicleDial) put +2 near
# batches on every recipe body. The dial face folds into the interior finish.
# The reverse-lamp lens (never a lamp channel: nothing in the simulation
# reverses) folds into the plate finish, the other near-white unlit dielectric.
# A recipe body then carries exactly its pre-kit batch count.
PACKING_VERSION=3
collapse={
    'VehicleSeat':'VehicleInterior','VehicleDash':'VehicleInterior',
    'VehicleHeadliner':'VehicleInterior','VehicleCarpet':'VehicleInterior',
    'VehicleDial':'VehicleInterior','VehicleReverse':'VehiclePlate',
    'VehicleAlloyDark':'VehicleTrimBlack',
    'VehicleCaliper':'VehicleTrimBlack','VehicleTrimGloss':'VehicleTrimBlack',
    'VehicleShutLine':'VehicleTrimBlack','VehicleLiveryDark':'VehicleTrimBlack',
    'VehicleChrome':'VehicleAlloy','VehicleSteel':'VehicleAlloy',
    'VehicleHeadlamp':'VehicleGlass',
}
entries=[]
for key,body in report['bodies'].items():
    entries.append((key,body))
    entries.extend(body.get('variants',{}).items())
for key,body in entries:
    for lod,record in enumerate(body['lods']):
        # A record seeded from production is already packed; re-exporting it
        # would change its bytes (the export is not deterministic) for nothing.
        if 'materialSlots' in record and not repack:
            print('PACK skip (already packed)',key,lod,flush=True)
            continue
        bpy.ops.wm.read_factory_settings(use_empty=True)
        file=directory/record['file']
        bpy.ops.import_scene.gltf(filepath=str(file))
        old_materials=set()
        new_materials=set()
        for ob in bpy.data.objects:
            if ob.type!='MESH': continue
            for slot in ob.material_slots:
                if not slot.material: continue
                old_materials.add(slot.material.name)
                target=collapse.get(slot.material.name)
                if target and bpy.data.materials.get(target): slot.material=bpy.data.materials[target]
                new_materials.add(slot.material.name)
        if 'materialSlots' in record and new_materials==old_materials:
            # Already packed and the policy folds nothing here (the sedan
            # lineage has no VehicleInterior or VehicleReverse): keep the
            # receipt and the bytes, record that this policy was applied.
            record.update(packingVersion=PACKING_VERSION)
            print('PACK unchanged',key,lod,len(old_materials),flush=True)
            continue
        bpy.ops.object.select_all(action='SELECT')
        bpy.ops.export_scene.gltf(filepath=str(file),use_selection=True,export_apply=True,
            export_yup=True,export_materials='EXPORT',export_format='GLB',
            export_animations=False,export_skins=False,export_texcoords=False,export_normals=True,
            # Extras carry the vehicle frame stamp (vehicle_frame.py) and lid hinges.
            export_extras=True)
        record.update(sha256=hashlib.sha256(file.read_bytes()).hexdigest(),bytes=file.stat().st_size,
            materialSlotsBefore=len(old_materials),materialSlots=len(new_materials),packingVersion=PACKING_VERSION)
        print('PACK',key,lod,len(old_materials),'->',len(new_materials),flush=True)
report['packing']={'version':PACKING_VERSION,'generator':'scripts/assets/pack-vehicle-fleet-v2.py',
    'policy':'shared opaque trim and alloy finishes; one interior finish that adds no near-tier batch; separate paint, glass, tyre and lamp functions'}
(directory/'build-report.json').write_text(json.dumps(report,indent=2)+'\n')
