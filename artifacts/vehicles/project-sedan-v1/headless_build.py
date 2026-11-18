# Headless rebuild: blender -b -P headless_build.py -- <out_dir> [render_tag]
import sys, os, time, importlib, logging, io, contextlib
SP = os.path.dirname(os.path.abspath(__file__))
if SP not in sys.path: sys.path.insert(0, SP)
argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
OUT = argv[0] if argv else SP
TAG = argv[1] if len(argv) > 1 else 'int'
EXPORT = (argv[2] if len(argv) > 2 else 'no') == 'export'
import bpy, math
logging.getLogger('glTFExporter').setLevel(logging.WARNING)
# review camera + lights exist only in the interactive scene; recreate them here
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
        import bmesh
        me = bpy.data.meshes.new('Ground'); bm = bmesh.new(); bmesh.ops.create_grid(bm, x_segments=1, y_segments=1, size=30); bm.to_mesh(me); bm.free()
        ob = bpy.data.objects.new('Ground', me); sc.collection.objects.link(ob)
        gm = bpy.data.materials.new('GroundMat'); gm.use_nodes = True
        gm.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value = (0.25, 0.25, 0.26, 1)
        gm.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value = 0.8; me.materials.append(gm)
engines = [e.identifier for e in bpy.types.RenderSettings.bl_rna.properties['engine'].enum_items]
sc.render.engine = 'BLENDER_EEVEE_NEXT' if 'BLENDER_EEVEE_NEXT' in engines else 'BLENDER_EEVEE'
sc.render.resolution_x = 1280; sc.render.resolution_y = 720; sc.render.image_settings.file_format = 'PNG'
import sedan_lib, build_sedan
importlib.reload(sedan_lib); importlib.reload(build_sedan)
t0 = time.time()
body = build_sedan.build()
tris = sedan_lib.tri_count()
print(f'[headless] built in {time.time()-t0:.1f}s objects={len(bpy.data.objects)} tris={tris}', flush=True)
os.makedirs(OUT, exist_ok=True)
build_sedan.pose(False)
build_sedan.render(f'{OUT}/{TAG}_front34.png', -35, 12)
build_sedan.render(f'{OUT}/{TAG}_side_close.png', -75, 10, dist=4.2, target=(0, 0.1, 0.95), lens=45)
build_sedan.render(f'{OUT}/{TAG}_cabin.png', -55, 22, dist=3.0, target=(-0.1, 0.1, 0.85), lens=32)
# inside the cabin: from the rear seat looking at the dashboard
cam = bpy.data.objects['Cam']; cam.data.lens = 18
cam.location = (0.25, 1.05, 1.12)
from mathutils import Vector
cam.rotation_euler = (Vector((-0.25, -0.55, 0.85)) - cam.location).to_track_quat('-Z', 'Y').to_euler()
sc.render.filepath = f'{OUT}/{TAG}_inside.png'; bpy.ops.render.render(write_still=True)
build_sedan.pose(True)
build_sedan.render(f'{OUT}/{TAG}_open_side.png', -80, 18, dist=4.5, target=(-0.3, 0.2, 0.8), lens=40)
build_sedan.render(f'{OUT}/{TAG}_open_front34.png', -35, 14, dist=6.5, target=(0, 0.2, 0.7), lens=45)
build_sedan.render(f'{OUT}/{TAG}_open_rear34.png', 145, 14, dist=6.5, target=(0, 0.2, 0.7), lens=45)
# interior with the glovebox, console lid, visors and seat backs moved: from the open passenger door
cam.data.lens = 18; cam.location = (0.72, 0.55, 1.08)
cam.rotation_euler = (Vector((-0.25, -0.45, 0.80)) - cam.location).to_track_quat('-Z', 'Y').to_euler()
sc.render.filepath = f'{OUT}/{TAG}_open_cabin.png'; bpy.ops.render.render(write_still=True)
# cutaway: body, greenhouse and glass hidden so every interior part is visible from above
cut = [bpy.data.objects[n] for n in ('Body', 'Greenhouse', 'Headliner', 'Windscreen', 'RearWindow', 'Hood', 'Trunk', 'Door_FL', 'Door_FR', 'Door_RL', 'Door_RR',
                                     'Windscreen_GasketA', 'Windscreen_GasketB', 'RearWindow_GasketA', 'RearWindow_GasketB', 'DomeLight') if n in bpy.data.objects]
for ob in cut: ob.hide_render = True
build_sedan.render(f'{OUT}/{TAG}_open_cutaway.png', -50, 58, dist=5.2, target=(0, 0.2, 0.6), lens=40)
build_sedan.render(f'{OUT}/{TAG}_open_cutaway_dash.png', -35, 28, dist=3.0, target=(0.05, -0.35, 0.75), lens=40)
for ob in cut: ob.hide_render = False
build_sedan.pose(False)
# dusk: sun and sky down so the emissive lamp parts read
sun = bpy.data.objects['Sun']; fill = bpy.data.objects['Fill']
sun_e, fill_e = sun.data.energy, fill.data.energy
sun.data.energy = 0.15; fill.data.energy = 20
bg = sc.world.node_tree.nodes.get('Background') if sc.world and sc.world.use_nodes else None
bg_s = bg.inputs['Strength'].default_value if bg else None
if bg: bg.inputs['Strength'].default_value = 0.03
build_sedan.render(f'{OUT}/{TAG}_dusk_front34.png', -35, 10, dist=6.0, target=(0, 0, 0.6), lens=50)
build_sedan.render(f'{OUT}/{TAG}_dusk_rear34.png', 145, 10, dist=6.0, target=(0, 0, 0.6), lens=50)
sun.data.energy, fill.data.energy = sun_e, fill_e
if bg: bg.inputs['Strength'].default_value = bg_s
# Parts dropped from the lower detail levels (built clean at their own resolution, see sedan_lib.set_quality).
LOD1_DROP = ('Headlamp_L_LowBeam', 'Headlamp_R_LowBeam', 'Headlamp_L_HighBeam', 'Headlamp_R_HighBeam', 'Indicator_F', 'BrakeLamp_', 'ReverseLamp_', 'FogLamp_',
             'DomeLight', 'SunVisor', 'Glovebox', 'Console', 'Gear', 'Pedal', 'FuelCap', 'FuelWell', 'Vent', 'Dials', 'CentreScreen', 'ScreenBezel', 'Binnacle',
             'Wheel_FL_Groove', 'Wheel_FR_Groove', 'Wheel_RL_Groove', 'Wheel_RR_Groove', 'Wheel_FL_Disc', 'Wheel_FR_Disc', 'Wheel_RL_Disc', 'Wheel_RR_Disc',
             'Wheel_FL_Caliper', 'Wheel_FR_Caliper', 'Wheel_RL_Caliper', 'Wheel_RR_Caliper', 'Shut', 'BumperSeam', 'DiffuserFin', 'Mirror_L_Glass', 'Mirror_R_Glass',
             'DoorPull', 'DoorInsert', 'RearShelfRib', 'SteeringColumn', 'Tunnel', 'Firewall', 'EngineBlock', 'EngineCover')
LOD2_DROP = LOD1_DROP + ('Headlamp_', 'Taillamp_L_Bar', 'Taillamp_R_Bar', 'Indicator_', 'HighMountStop', 'Seat', 'Dash', 'Floor', 'RearShelf', 'Headliner', 'Steering',
             'RearViewMirror', 'DoorCard', 'DoorArmrest', 'GrilleSlat', 'GrilleFrame', 'Handle_', 'CharLine', 'Antenna', 'Exhaust', 'FuelDoor', 'Wiper', 'Mirror_L_Arm', 'Mirror_R_Arm',
             'Wheel_FL_Spoke', 'Wheel_FR_Spoke', 'Wheel_RL_Spoke', 'Wheel_RR_Spoke', 'Wheel_FL_Cap', 'Wheel_FR_Cap', 'Wheel_RL_Cap', 'Wheel_RR_Cap', 'Window_', 'SideFront', 'SideRear',
             'GasketLo', 'GasketHi', 'GasketA', 'GasketB', 'EngineBay', 'TrunkBay', 'Rocker', 'Underbody', 'Intake')
LOD2_KEEP = ('Headlamp_L_Housing', 'Headlamp_R_Housing', 'Headlamp_L_Cover', 'Headlamp_R_Cover', 'Headlamp_L_DRL0', 'Headlamp_R_DRL0', 'Headlamp_L_DRL3', 'Headlamp_R_DRL3', 'Headlamp_L_DRL6', 'Headlamp_R_DRL6')
def drop_parts(prefixes, keep=()):
    for ob in list(sedan_lib.root().all_objects):
        n = ob.name
        if any(n.startswith(k) for k in keep): continue
        if any(n.startswith(p) for p in prefixes): bpy.data.objects.remove(ob, do_unlink=True)
if EXPORT:
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        sedan_lib.export_glb(f'{OUT}/project-sedan-v1.glb')
    bpy.ops.wm.save_as_mainfile(filepath=f'{OUT}/project-sedan-v1.blend')
    print('[headless] exported lod0', os.path.getsize(f'{OUT}/project-sedan-v1.glb'), flush=True)
    for level, drops, keep, fname in ((1, LOD1_DROP, (), 'project-sedan-v1-lod1.glb'), (2, LOD2_DROP, LOD2_KEEP, 'project-sedan-v1-lod2.glb')):
        t1 = time.time()
        sedan_lib.set_quality(level)
        build_sedan.build()
        drop_parts(drops, keep)
        if level == 2:
            # distant tier: a dark cabin block so the glass reads as a volume, not a hole
            sedan_lib.prim_box('CabinBlock', (1.40, 2.20, 0.36), (0, 0.55, 1.02), bpy.data.materials['VehicleInterior'])
        tris = sedan_lib.tri_count()
        with contextlib.redirect_stdout(buf):
            sedan_lib.export_glb(f'{OUT}/{fname}')
        print(f'[headless] exported lod{level} tris={tris} bytes={os.path.getsize(f"{OUT}/{fname}")} in {time.time()-t1:.1f}s', flush=True)
    sedan_lib.set_quality(0)
print(f'[headless] done in {time.time()-t0:.1f}s', flush=True)
