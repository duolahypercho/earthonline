"""Turn the downloaded Poly Haven models into small, game-ready furniture pieces.

    blender -b --factory-startup --python scripts/blender-buildings/furniture/prepare_library.py -- \
        --store /path/to/polyhaven-store --out /path/to/furniture-lib \
        --catalog scripts/blender-buildings/furniture/catalog.json \
        --library scripts/blender-buildings/furniture/library.json

For every catalog entry: import the 1k glTF, join it into one mesh, put the
origin at the centre of its base (standing on z = 0), optionally scale it to
`sizeM`, decimate to the entry's `triangles` target, shrink its textures to
512 px, and export `<kind>.glb` into the off-repo library. `library.json`
(tracked) records each piece's size, triangle count, source and licence so the
builder can place it without opening Blender and provenance stays with the repo.

Orientation: Poly Haven models face Blender -Y, which is the building frame's
+Z (toward the viewer), the same convention build_building.py authors in.
"""

import argparse
import json
import os
import sys

import bpy

TEXTURE_PX = 512


def parse_args():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument('--store', required=True)
    parser.add_argument('--out', required=True)
    parser.add_argument('--catalog', required=True)
    parser.add_argument('--library', required=True)
    parser.add_argument('--only', default=None, help='comma-separated kinds')
    return parser.parse_args(argv)


def clear_scene():
    for obj in list(bpy.data.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    for block in (bpy.data.meshes, bpy.data.materials, bpy.data.images):
        for item in list(block):
            if item.users == 0:
                block.remove(item)


def triangles(obj):
    obj.data.calc_loop_triangles()
    return len(obj.data.loop_triangles)


def prepare(entry, store, out):
    ph = entry['polyhaven']
    src_dir = os.path.join(store, ph)
    with open(os.path.join(src_dir, 'source.json')) as fh:
        source = json.load(fh)
    clear_scene()
    bpy.ops.import_scene.gltf(filepath=os.path.join(src_dir, source['gltf']))
    meshes = [o for o in bpy.context.scene.objects if o.type == 'MESH']
    if not meshes:
        raise RuntimeError(f'{ph}: no mesh')
    # bake parent transforms into the meshes, then join
    for obj in bpy.context.scene.objects:
        obj.select_set(False)
    for obj in meshes:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    bpy.ops.object.parent_clear(type='CLEAR_KEEP_TRANSFORM')
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    if len(meshes) > 1:
        bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    for other in [o for o in bpy.context.scene.objects if o != obj]:
        bpy.data.objects.remove(other, do_unlink=True)
    obj.name = entry['kind']
    obj.data.name = entry['kind']
    source_tris = triangles(obj)

    # origin at the centre of the base
    xs = [v.co.x for v in obj.data.vertices]
    ys = [v.co.y for v in obj.data.vertices]
    zs = [v.co.z for v in obj.data.vertices]
    cx, cy, z0 = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2, min(zs)
    for v in obj.data.vertices:
        v.co.x -= cx
        v.co.y -= cy
        v.co.z -= z0
    size = [max(xs) - min(xs), max(zs) - min(zs), max(ys) - min(ys)]  # frame order: x, height, z
    if entry.get('sizeM'):
        target = entry['sizeM']
        if entry.get('fit') == 'stretch':
            # per-axis: footprint to sizeM x/z, height to sizeM y
            sx, sy, sz = (target[0] / size[0], target[2] / size[2], target[1] / size[1])
            for v in obj.data.vertices:
                v.co.x *= sx
                v.co.y *= sy
                v.co.z *= sz
            size = list(target)
        else:
            scale = min(target[0] / size[0], target[2] / size[2]) if size[0] and size[2] else 1.0
            for v in obj.data.vertices:
                v.co *= scale
            size = [s * scale for s in size]
    obj.data.update()

    # decimate to the budget
    want = entry.get('triangles', 400)
    if source_tris > want:
        mod = obj.modifiers.new('decimate', 'DECIMATE')
        mod.ratio = max(0.002, want / source_tris)
        mod.use_collapse_triangulate = True
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.modifier_apply(modifier=mod.name)
    final_tris = triangles(obj)

    # shrink textures
    for image in bpy.data.images:
        if image.size[0] > TEXTURE_PX or image.size[1] > TEXTURE_PX:
            image.scale(min(TEXTURE_PX, image.size[0]), min(TEXTURE_PX, image.size[1]))
    # stable material names that cannot collide with the kit or int-* names
    for index, slot in enumerate(obj.material_slots):
        if slot.material:
            slot.material.name = f'ph-{entry["kind"]}-{index}'

    os.makedirs(out, exist_ok=True)
    path = os.path.join(out, f'{entry["kind"]}.glb')
    for o in bpy.context.scene.objects:
        o.select_set(o == obj)
    bpy.ops.export_scene.gltf(
        filepath=path, export_format='GLB', use_selection=True, export_apply=True,
        export_yup=True, export_image_format='JPEG', export_jpeg_quality=82,
        export_texcoords=True, export_normals=True, export_tangents=False,
        export_cameras=False, export_lights=False, export_extras=False,
    )
    return {
        'kind': entry['kind'],
        'file': f'{entry["kind"]}.glb',
        'sizeM': [round(s, 3) for s in size],
        'triangles': final_tris,
        'sourceTriangles': source_tris,
        'bytes': os.path.getsize(path),
        'source': source['source'],
        'name': source['name'],
        'authors': source['authors'],
        'license': source['license'],
    }


def main():
    args = parse_args()
    with open(args.catalog) as fh:
        catalog = json.load(fh)
    library = {'note': 'GENERATED by prepare_library.py. Game-ready CC0 furniture; binaries live off-repo.', 'pieces': {}}
    if os.path.exists(args.library):
        with open(args.library) as fh:
            library = json.load(fh)
    only = set(args.only.split(',')) if args.only else None
    for entry in catalog['assets']:
        if only and entry['kind'] not in only:
            continue
        if not os.path.exists(os.path.join(args.store, entry['polyhaven'], 'source.json')):
            print(f'MISSING {entry["polyhaven"]}')
            continue
        record = prepare(entry, args.store, args.out)
        library['pieces'][entry['kind']] = record
        print(f'PIECE {record["kind"]}: {record["sourceTriangles"]} -> {record["triangles"]} tris, '
              f'{record["sizeM"]} m, {record["bytes"] // 1024} KB')
    with open(args.library, 'w') as fh:
        json.dump(library, fh, indent=2)
        fh.write('\n')


main()
