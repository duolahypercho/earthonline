"""Dev review renders for a v3 body.

blender -b -P dev_render.py -- <body id> <out prefix> [--stage shell|full] [--paint r,g,b]
Renders a studio set (front 3/4, side ortho, rear 3/4, front, top) with Eevee.
Review output stays outside the repository.
"""
import importlib
import json
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import v3_body as V  # noqa: E402
importlib.reload(V)

argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
body_id = argv[0] if argv else 'compactHatch'
prefix = argv[1] if len(argv) > 1 else '/tmp/v3'
stage = 'shell'
paint = (0.18, 0.19, 0.20)
for i, a in enumerate(argv):
    if a == '--stage':
        stage = argv[i + 1]
    if a == '--paint':
        paint = tuple(float(v) for v in argv[i + 1].split(','))


def reset():
    for ob in list(bpy.data.objects):
        bpy.data.objects.remove(ob, do_unlink=True)


def load_body(body_id):
    """Resolve the same body-profile inheritance as build_v3.py."""
    cfg = json.loads((HERE / 'bodies' / f'{body_id}.json').read_text())
    if 'extends' in cfg:
        base = load_body(cfg['extends'])
        for key, value in cfg.items():
            if key == 'curvesOverride':
                base['curves'].update(value)
            elif isinstance(value, dict) and isinstance(base.get(key), dict) and key not in ('lamps',):
                base[key] = {**base[key], **value}
            else:
                base[key] = value
        base.pop('extends', None)
        cfg = base
    return cfg


def studio(scn, size, ctr):
    try:
        scn.render.engine = 'BLENDER_EEVEE_NEXT'
    except TypeError:
        scn.render.engine = 'BLENDER_EEVEE'
    scn.render.resolution_x, scn.render.resolution_y = 1200, 700
    world = bpy.data.worlds.get('dev') or bpy.data.worlds.new('dev')
    scn.world = world
    world.use_nodes = True
    bg = world.node_tree.nodes['Background']
    bg.inputs[0].default_value = (0.55, 0.60, 0.68, 1)
    bg.inputs[1].default_value = 0.55
    bpy.ops.mesh.primitive_plane_add(size=80, location=(0, 0, 0))
    fl = bpy.context.active_object
    fm = bpy.data.materials.new('floor')
    fm.use_nodes = True
    b = next(n for n in fm.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
    b.inputs['Base Color'].default_value = (0.30, 0.30, 0.31, 1)
    b.inputs['Roughness'].default_value = 0.7
    fl.data.materials.append(fm)
    for name, loc, e, s in (('key', (4, -5, 6), 450, 3), ('fill', (-5, 3, 4), 150, 7), ('rim', (0, 7, 3), 250, 4)):
        d = bpy.data.lights.new(name, 'AREA')
        d.energy, d.size = e, s
        o = bpy.data.objects.new(name, d)
        scn.collection.objects.link(o)
        o.location = loc
        o.rotation_euler = (ctr - Vector(loc)).to_track_quat('-Z', 'Y').to_euler()
    sun = bpy.data.lights.new('sun', 'SUN')
    sun.energy = 2.0
    so = bpy.data.objects.new('sun', sun)
    scn.collection.objects.link(so)
    so.rotation_euler = (math.radians(35), 0, math.radians(25))


def cam_shot(scn, name, pos, target, ortho=None, lens=50, roll=None):
    cd = bpy.data.cameras.new(name)
    cd.lens = lens
    if ortho:
        cd.type = 'ORTHO'
        cd.ortho_scale = ortho
    c = bpy.data.objects.new(name, cd)
    scn.collection.objects.link(c)
    c.location = pos
    c.rotation_euler = (Vector(target) - Vector(pos)).to_track_quat('-Z', 'Y').to_euler()
    if roll is not None:
        c.rotation_euler = (0, 0, roll)
    scn.camera = c
    scn.render.filepath = f'{prefix}-{name}.png'
    bpy.ops.render.render(write_still=True)


def main():
    reset()
    scn = bpy.context.scene
    cfg = load_body(body_id)
    spec = V.BodySpec(cfg)
    mat = bpy.data.materials.get('VehiclePaint') or bpy.data.materials.new('VehiclePaint')
    mat.use_nodes = True
    b = next(n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED')
    b.inputs['Base Color'].default_value = (*paint, 1)
    b.inputs['Metallic'].default_value = 0.3
    b.inputs['Roughness'].default_value = 0.25
    if 'Coat Weight' in b.inputs:
        b.inputs['Coat Weight'].default_value = 1.0
    if stage == 'shell':
        ob, rings, ys = V.build_shell(spec, 'Body', mat, lod=0, step=0.02)
        scn.collection.objects.link(ob)
        print('SHELL verts', len(ob.data.vertices), 'faces', len(ob.data.polygons), 'rings', rings)
    else:
        import v3_build as BLD
        importlib.reload(BLD)
        BLD.build(cfg, lod=0, paint_mat=mat)
    ctr = Vector((0, 0, 0.72))
    studio(scn, spec.L, ctr)
    L = spec.L
    cam_shot(scn, 'front34', (3.2, -5.2, 1.55), (0, -0.2, 0.6), lens=45)
    cam_shot(scn, 'rear34', (-3.4, 5.0, 1.7), (0, 0.2, 0.6), lens=45)
    cam_shot(scn, 'side', (12, 0, 0.74), (0, 0, 0.74), ortho=L * 1.12)
    cam_shot(scn, 'front', (0, -12, 0.74), (0, 0, 0.74), ortho=2.6)
    cam_shot(scn, 'rear', (0, 12, 0.74), (0, 0, 0.74), ortho=2.6)
    cam_shot(scn, 'top', (0, 0, 12), (0, 0.0001, 0), ortho=L * 1.12, roll=math.pi / 2)
    cam_shot(scn, 'closeF', (1.9, -3.6, 1.05), (0.1, -1.6, 0.62), lens=50)
    cam_shot(scn, 'closeR', (-1.8, 3.6, 1.2), (-0.1, 1.7, 0.75), lens=50)


main()
