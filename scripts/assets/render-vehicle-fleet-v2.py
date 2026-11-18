"""Offline roster review from exported runtime geometry; no game renderer."""
import bpy
import sys
import json
from pathlib import Path
from mathutils import Vector

directory=Path(sys.argv[sys.argv.index('--')+1]).expanduser().resolve()
root=Path(__file__).resolve().parents[2]
if directory.is_relative_to(root): raise ValueError('Review binaries belong outside the repository')
report=json.loads((directory/'build-report.json').read_text())
bpy.ops.wm.read_factory_settings(use_empty=True)
palette=[(.55,.025,.018),(.065,.11,.20),(.23,.29,.25),(.78,.32,.055),(.72,.74,.76),
    (.12,.14,.16),(.62,.65,.68),(.64,.65,.62),(.075,.22,.29),(.85,.55,.035)]
for i,(key,record) in enumerate(report['bodies'].items()):
    previous=set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=str(directory/record['lods'][0]['file']))
    objects=list(set(bpy.data.objects)-previous)
    group=bpy.data.objects.new(record['label'],None);bpy.context.scene.collection.objects.link(group)
    for ob in objects:
        if ob.parent is None: ob.parent=group
        if ob.type=='MESH':
            for slot in ob.material_slots:
                if slot.material and slot.material.name.startswith('VehiclePaint'):
                    m=slot.material.copy();m.name=key+' review paint'
                    m.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(*palette[i],1)
                    slot.material=m
    group.location=(i%5*4.5, i//5*14, 0)
    curve=bpy.data.curves.new(key+' label','FONT');curve.body=record['label'];curve.align_x='CENTER';curve.size=.23;curve.extrude=.002
    label=bpy.data.objects.new(key+' label',curve);bpy.context.scene.collection.objects.link(label)
    label.location=(group.location.x,group.location.y-record['lengthM']/2-.7,.015)
floor=bpy.data.materials.new('Review floor');floor.diffuse_color=(.11,.14,.17,1)
bpy.ops.mesh.primitive_plane_add(size=200,location=(8,7,-.025));bpy.context.object.data.materials.append(floor)
sc=bpy.context.scene;world=bpy.data.worlds.new('Review environment');sc.world=world;world.use_nodes=True
world.node_tree.nodes['Background'].inputs['Color'].default_value=(.35,.43,.55,1)
world.node_tree.nodes['Background'].inputs['Strength'].default_value=.65
for name,loc,energy,size in [('Key',(-8,-7,22),6500,13),('Fill',(21,-4,15),4200,12),('Rim',(10,24,18),6500,10)]:
    light=bpy.data.lights.new(name,'AREA');light.energy=energy;light.shape='DISK';light.size=size
    ob=bpy.data.objects.new(name,light);sc.collection.objects.link(ob);ob.location=loc
    ob.rotation_euler=(Vector((8,7,0))-ob.location).to_track_quat('-Z','Y').to_euler()
cam=bpy.data.objects.new('Fleet review camera',bpy.data.cameras.new('Fleet review camera'))
sc.collection.objects.link(cam);sc.camera=cam;cam.location=(26,-31,30)
cam.rotation_euler=(Vector((8.5,6,0))-cam.location).to_track_quat('-Z','Y').to_euler();cam.data.type='ORTHO';cam.data.ortho_scale=35
sc.render.engine='CYCLES';sc.cycles.samples=48;sc.cycles.use_denoising=True
sc.render.resolution_x=2400;sc.render.resolution_y=1800;sc.render.resolution_percentage=100
sc.render.image_settings.file_format='PNG';sc.render.filepath=str(directory/'fleet-lineup.png')
bpy.ops.wm.save_as_mainfile(filepath=str(directory/'Fleet-Lineup.blend'))
bpy.ops.render.render(write_still=True)
