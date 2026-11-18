"""Reload an exact candidate GLB and capture repeatable, off-repo visual evidence.

Run with Blender --background --python-exit-code 1 --python this-file --
--glb candidate.glb --output review-directory. This reports geometry; it does
not turn structural checks or rendered images into visual acceptance.
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector


VIEWS = {
    'quarter': (-5, -6, 3.2),
    'front': (0, -8, 1.4),
    'side': (-8, 0, .85),
    'rear': (4, 6, 2.5),
    'doors': (-5, -4, 3.5),
    'front-detail': (0, -8, 1.25),
    'back-detail': (0, 8, 1.25),
}


def geometry_report():
    parts = []
    for obj in bpy.context.scene.objects:
        if obj.type != 'MESH':
            continue
        mesh = obj.data
        mesh.calc_loop_triangles()
        bm = bmesh.new()
        bm.from_mesh(mesh)
        unseen = set(bm.verts)
        components = []
        while unseen:
            stack = [unseen.pop()]
            count = 0
            while stack:
                vertex = stack.pop()
                count += 1
                for edge in vertex.link_edges:
                    other = edge.other_vert(vertex)
                    if other in unseen:
                        unseen.remove(other)
                        stack.append(other)
            components.append(count)
        points = [obj.matrix_world @ v.co for v in mesh.vertices]
        part = {
            'name': obj.name,
            'triangles': len(mesh.loop_triangles),
            'vertices': len(mesh.vertices),
            # GLB imports split vertices at UV/normal/material seams. Raw
            # boundary counts are diagnostic, not a watertightness verdict.
            'rawBoundaryEdges': sum(e.is_boundary for e in bm.edges),
            'rawNonManifoldEdges': sum(not e.is_manifold for e in bm.edges),
            'rawComponents': sorted(components, reverse=True),
            'degenerateFaces': sum(f.calc_area() < 1e-12 for f in bm.faces),
            'bounds': [[min(p[i] for p in points) for i in range(3)],
                       [max(p[i] for p in points) for i in range(3)]] if points else None,
        }
        # Welding a temporary copy removes export splits without modifying
        # the reviewed artifact. Remaining edges can be intended openings;
        # callers must judge them against the part's actual contract.
        bmesh.ops.remove_doubles(bm, verts=list(bm.verts), dist=1e-6)
        part['weldedBoundaryEdges'] = sum(e.is_boundary for e in bm.edges)
        part['weldedNonManifoldEdges'] = sum(not e.is_manifold for e in bm.edges)
        part['weldedDegenerateFaces'] = sum(f.calc_area() < 1e-12 for f in bm.faces)
        parts.append(part)
        bm.free()
    return {'triangles': sum(p['triangles'] for p in parts),
            'parts': sorted(parts, key=lambda p: -p['triangles'])}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--glb', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--views', nargs='+', choices=VIEWS, default=list(VIEWS))
    parser.add_argument('--samples', type=int, default=24)
    args = parser.parse_args(sys.argv[sys.argv.index('--') + 1:])
    args.output.mkdir(parents=True, exist_ok=True)
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(args.glb.resolve()))
    bpy.context.view_layer.update()
    report = geometry_report()
    report.update({'input': args.glb.name,
                   'sha256': hashlib.sha256(args.glb.read_bytes()).hexdigest(),
                   'status': 'export inspection only; visual acceptance separate',
                   'coordinateSystem': 'Blender import: front -Y, up +Z',
                   'captures': []})
    scene = bpy.context.scene
    scene.render.engine = 'CYCLES'
    scene.cycles.samples = args.samples
    scene.render.resolution_x = 1400
    scene.render.resolution_y = 850
    scene.render.resolution_percentage = 100
    scene.world = bpy.data.worlds.new('Vehicle review world')
    scene.world.color = (.18, .18, .18)
    for name, loc, power, size in [
        ('Key', (-3, -4, 6), 1200, 5),
        ('Fill', (4, 1, 4), 850, 4),
        ('Rim', (0, 4, 5), 1100, 4),
    ]:
        data = bpy.data.lights.new(name, 'AREA')
        data.energy = power
        data.shape = 'DISK'
        data.size = size
        obj = bpy.data.objects.new(name, data)
        scene.collection.objects.link(obj)
        obj.location = loc
        obj.rotation_euler = (Vector((0, 0, .6)) - obj.location).to_track_quat('-Z', 'Y').to_euler()
    data = bpy.data.cameras.new('Vehicle review camera')
    camera = bpy.data.objects.new('Vehicle review camera', data)
    scene.collection.objects.link(camera)
    scene.camera = camera
    data.type = 'ORTHO'
    data.ortho_scale = 5.7
    doors = [bpy.data.objects.get('Door_' + key) for key in ('FL', 'RL')]
    for view in args.views:
        for door in doors:
            if door:
                door.rotation_mode = 'XYZ'
                door.rotation_euler.z = math.radians(58) if view == 'doors' else 0
        camera.location = VIEWS[view]
        data.ortho_scale = 2.5 if view.endswith('-detail') else 5.7
        camera.rotation_euler = (Vector((0, 0, .72)) - camera.location).to_track_quat('-Z', 'Y').to_euler()
        path = args.output / (view + '.png')
        scene.render.filepath = str(path.resolve())
        bpy.ops.render.render(write_still=True)
        report['captures'].append({'view': view, 'file': path.name,
                                   'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                                   'eye': list(VIEWS[view]), 'target': [0, 0, .72],
                                   'orthoScaleM': data.ortho_scale,
                                   'resolution': [1400, 850], 'samples': args.samples})
    (args.output / 'review.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'sha256': report['sha256'], 'triangles': report['triangles'],
                      'captures': len(report['captures'])}))


if __name__ == '__main__':
    main()
