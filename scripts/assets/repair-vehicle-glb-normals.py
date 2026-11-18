"""Repair the shading normals of a shipped generated-vehicle GLB set, GLB to GLB.

    python3 scripts/assets/repair-vehicle-glb-normals.py \
        --receipt public/assets/vehicles/generated/robo-cab-v1/receipt.json \
        --output <dir outside the repository>

Why: a body decimated from a dense study (scripts/assets/build-robo-cab-v1.py) was
exported with ONE averaged normal per vertex over the whole shell. Every hard crease
(bonnet lip, panel returns, the edge where the bonnet meets the lamp strip, the rim of a
flat wheel cover) was smoothed across, and decimation left the creases as zig-zag
triangle paths, so a clearcoat drew saw-toothed, crumpled highlights along every
crease and star-shaped folds on the wheel covers.

What it does, per primitive, with numpy only (no Blender, so every other byte of the
document -- node names, pivots, extras, materials -- is carried over unchanged):
  * welds coincident vertices to recover the surface topology (UVs are kept per corner),
  * splits the normal at edges whose dihedral angle exceeds --sharp-deg (hard creases),
  * recomputes each smooth fan's normal weighted by face area x corner angle, so the
    thin slivers decimation leaves no longer tilt their neighbours, ignoring faces that
    fold back against the fan (decimation flips),
  * optionally relaxes the normal field inside smooth regions (--relax iterations),
  * leaves positions, indices order, UVs and triangle counts unchanged (vertices are
    only duplicated where a crease is split).
It also flattens KHR_materials_transmission to alpha-blended glass (the runtime rule:
transmission ends a render pass during the environment probe bake) and fails if a
result carries morph targets or transmission. Writes a build report for
scripts/assets/prepare-generated-vehicle.py; promotes nothing.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
PUBLIC = ROOT / 'public' / 'assets' / 'vehicles'
METHOD = 'Normals repair of the shipped runtime GLB (scripts/assets/repair-vehicle-glb-normals.py)'
COMPONENT = {5121: np.uint8, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}
WIDTH = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def read_glb(path):
    raw = Path(path).read_bytes()
    magic, version, _ = struct.unpack_from('<4sII', raw, 0)
    if magic != b'glTF' or version != 2:
        raise ValueError(f'{path} is not a glTF 2 binary')
    json_length = struct.unpack_from('<I', raw, 12)[0]
    document = json.loads(raw[20:20 + json_length])
    binary = raw[28 + json_length:]
    return document, binary


def accessor_array(document, binary, index):
    accessor = document['accessors'][index]
    view = document['bufferViews'][accessor['bufferView']]
    dtype = np.dtype(COMPONENT[accessor['componentType']]).newbyteorder('<')
    width = WIDTH[accessor['type']]
    offset = view.get('byteOffset', 0) + accessor.get('byteOffset', 0)
    stride = view.get('byteStride', width * dtype.itemsize)
    array = np.ndarray((accessor['count'], width), dtype=dtype, buffer=binary,
                       offset=offset, strides=(stride, dtype.itemsize))
    return np.array(array)


class Union:
    def __init__(self, size):
        self.parent = np.arange(size)

    def find_all(self):
        parent = self.parent
        while True:
            grand = parent[parent]
            if np.array_equal(grand, parent):
                return parent
            parent = grand

    def union_pairs(self, a, b):
        # iterate to a fixed point with vectorised pointer jumping
        a = np.asarray(a)
        b = np.asarray(b)
        while len(a):
            ra = self.find_all()[a]
            rb = self.find_all()[b]
            differ = ra != rb
            if not differ.any():
                return
            lo = np.minimum(ra[differ], rb[differ])
            hi = np.maximum(ra[differ], rb[differ])
            # point every high root at its smallest low root
            order = np.lexsort((lo, hi))
            hi_sorted, lo_sorted = hi[order], lo[order]
            first = np.ones(len(hi_sorted), bool)
            first[1:] = hi_sorted[1:] != hi_sorted[:-1]
            self.parent[hi_sorted[first]] = lo_sorted[first]
            a, b = a[differ], b[differ]


def repair_primitive(positions, faces, sharp_deg, relax):
    """Return (vertex source index, normal) arrays and the remapped faces."""
    count = len(faces)
    # weld coincident vertices for topology only
    keys = np.round(positions.astype(np.float64) / 1e-6).astype(np.int64)
    _, weld = np.unique(keys, axis=0, return_inverse=True)
    weld = weld.reshape(-1)
    welded = weld[faces]                                   # (F,3)
    p = positions.astype(np.float64)[faces]                # (F,3,3)
    cross = np.cross(p[:, 1] - p[:, 0], p[:, 2] - p[:, 0])
    double_area = np.linalg.norm(cross, axis=1)
    face_normal = cross / np.maximum(double_area, 1e-20)[:, None]
    # corner angles
    angles = np.zeros((count, 3))
    for k in range(3):
        u = p[:, (k + 1) % 3] - p[:, k]
        v = p[:, (k + 2) % 3] - p[:, k]
        nu = np.linalg.norm(u, axis=1)
        nv = np.linalg.norm(v, axis=1)
        cos = np.einsum('ij,ij->i', u, v) / np.maximum(nu * nv, 1e-20)
        angles[:, k] = np.arccos(np.clip(cos, -1, 1))
    weight = (0.5 * double_area)[:, None] * angles           # (F,3) area x angle
    weight[double_area < 1e-14] = 0.0

    # edges: (face, slot k) edge runs corner k -> corner k+1
    edge_face = np.repeat(np.arange(count), 3)
    edge_slot = np.tile(np.arange(3), count)
    a = welded[edge_face, edge_slot]
    b = welded[edge_face, (edge_slot + 1) % 3]
    lo, hi = np.minimum(a, b), np.maximum(a, b)
    order = np.lexsort((hi, lo))
    lo_s, hi_s = lo[order], hi[order]
    same = (lo_s[1:] == lo_s[:-1]) & (hi_s[1:] == hi_s[:-1])
    # only manifold edges (exactly two faces) can be smooth
    starts = np.flatnonzero(np.r_[True, ~same])
    lengths = np.diff(np.r_[starts, len(order)])
    pair_start = starts[lengths == 2]
    e1 = order[pair_start]
    e2 = order[pair_start + 1]
    f1, f2 = edge_face[e1], edge_face[e2]
    s1, s2 = edge_slot[e1], edge_slot[e2]
    dot = np.einsum('ij,ij->i', face_normal[f1], face_normal[f2])
    smooth = dot > np.cos(np.radians(sharp_deg))
    f1, f2, s1, s2 = f1[smooth], f2[smooth], s1[smooth], s2[smooth]

    # corner id = face*3 + slot; join corners that share a welded vertex across a smooth edge
    union = Union(count * 3)
    va, vb = welded[f1, s1], welded[f1, (s1 + 1) % 3]
    for vertex in (va, vb):
        c1 = f1 * 3 + np.argmax(welded[f1] == vertex[:, None], axis=1)
        c2 = f2 * 3 + np.argmax(welded[f2] == vertex[:, None], axis=1)
        union.union_pairs(c1, c2)
    group = np.unique(union.find_all(), return_inverse=True)[1].reshape(-1)
    groups = group.max() + 1

    corner_normal = np.repeat(face_normal, 3, axis=0)
    corner_weight = weight.reshape(-1)
    fan = np.zeros((groups, 3))
    np.add.at(fan, group, corner_normal * corner_weight[:, None])
    fan /= np.maximum(np.linalg.norm(fan, axis=1), 1e-20)[:, None]
    # faces folded back against their fan (decimation flips) do not vote
    agree = np.einsum('ij,ij->i', corner_normal, fan[group]) > 0.0
    fan = np.zeros((groups, 3))
    np.add.at(fan, group, corner_normal * (corner_weight * agree)[:, None])
    empty = np.linalg.norm(fan, axis=1) < 1e-20
    if empty.any():   # a fan made only of degenerate or folded faces keeps its face normal
        np.add.at(fan, group, corner_normal * empty[group][:, None])
    fan /= np.maximum(np.linalg.norm(fan, axis=1), 1e-20)[:, None]

    if relax:
        # neighbouring fans across smooth edges: (group at va on f1) ~ (group at vb on f1)
        ca = f1 * 3 + np.argmax(welded[f1] == va[:, None], axis=1)
        cb = f1 * 3 + np.argmax(welded[f1] == vb[:, None], axis=1)
        ga, gb = group[ca], group[cb]
        for _ in range(relax):
            acc = fan.copy()
            np.add.at(acc, ga, 0.5 * fan[gb])
            np.add.at(acc, gb, 0.5 * fan[ga])
            fan = acc / np.maximum(np.linalg.norm(acc, axis=1), 1e-20)[:, None]

    # output vertex = (original vertex, fan)
    original = faces.reshape(-1)
    pair = original.astype(np.int64) * groups + group
    unique_pair, new_index = np.unique(pair, return_inverse=True)
    source = (unique_pair // groups).astype(np.int64)
    normals = fan[unique_pair % groups].astype(np.float32)
    return source, normals, new_index.reshape(-1, 3), int(smooth.size - smooth.sum())


def write_glb(document, chunks, path):
    """Rebuild buffer 0 from `chunks` (bytes per bufferView) and write the binary."""
    blob = bytearray()
    for view, data in zip(document['bufferViews'], chunks):
        while len(blob) % 4:
            blob.append(0)
        view['byteOffset'] = len(blob)
        view['byteLength'] = len(data)
        view['buffer'] = 0
        blob.extend(data)
    while len(blob) % 4:
        blob.append(0)
    document['buffers'] = [{'byteLength': len(blob)}]
    text = json.dumps(document, separators=(',', ':')).encode()
    text += b' ' * ((4 - len(text) % 4) % 4)
    total = 12 + 8 + len(text) + 8 + len(blob)
    with open(path, 'wb') as out:
        out.write(struct.pack('<4sII', b'glTF', 2, total))
        out.write(struct.pack('<I4s', len(text), b'JSON'))
        out.write(text)
        out.write(struct.pack('<I4s', len(blob), b'BIN\x00'))
        out.write(blob)


def flatten_transmission(document):
    flattened = []
    for material in document.get('materials', []):
        ext = material.get('extensions', {})
        if 'KHR_materials_transmission' not in ext:
            continue
        factor = ext.pop('KHR_materials_transmission').get('transmissionFactor', 0.0)
        pbr = material.setdefault('pbrMetallicRoughness', {})
        color = pbr.get('baseColorFactor', [1, 1, 1, 1])
        color[3] = min(color[3], 1.0 - 0.75 * factor)
        pbr['baseColorFactor'] = color
        if color[3] < 1.0:
            material['alphaMode'] = 'BLEND'
        if not ext:
            material.pop('extensions', None)
        flattened.append(material['name'])
    used = {name for m in document.get('materials', []) for name in m.get('extensions', {})}
    document['extensionsUsed'] = [e for e in document.get('extensionsUsed', [])
                                  if e in used or not e.startswith('KHR_materials_')]
    if not document['extensionsUsed']:
        document.pop('extensionsUsed')
    return flattened


def assert_runtime_glb(document, name):
    morphs = sum(len(p.get('targets', [])) for m in document.get('meshes', []) for p in m['primitives'])
    if morphs or 'KHR_materials_transmission' in document.get('extensionsUsed', []):
        raise ValueError(f'{name} is not runtime-safe: {morphs} morph targets, '
                         f'extensions {document.get("extensionsUsed", [])}')


def rig_declaration(document):
    """The rig the GLB carries, read from its node names (the repair never changes them)."""
    names = {node.get('name') for node in document.get('nodes', [])}
    corners = ('FL', 'FR', 'RL', 'RR')
    return dict(wheels=[k for k in corners if f'Wheel_{k}' in names],
                doors=[k for k in corners if f'Door_{k}' in names],
                lids=[k for k in ('Hood', 'Trunk') if k in names],
                seats=[k for k in corners if f'Seat_{k}_Rig' in names])


def triangles(document):
    return sum(document['accessors'][p['indices']]['count'] // 3
               for m in document['meshes'] for p in m['primitives'])


def repair_glb(source, target, sharp_deg, relax):
    document, binary = read_glb(source)
    before = triangles(document)
    chunks = [binary[v.get('byteOffset', 0):v.get('byteOffset', 0) + v['byteLength']]
              for v in document['bufferViews']]
    stats = dict(primitives=0, sharpEdges=0, verticesBefore=0, verticesAfter=0)

    def add(array, target_kind, accessor_type, extra=None):
        array = np.ascontiguousarray(array)
        document['bufferViews'].append({'buffer': 0, 'byteLength': array.nbytes, 'target': target_kind})
        chunks.append(array.astype(array.dtype.newbyteorder('<')).tobytes())
        component = {np.dtype(np.float32): 5126, np.dtype(np.uint16): 5123,
                     np.dtype(np.uint32): 5125}[np.dtype(array.dtype)]
        accessor = {'bufferView': len(document['bufferViews']) - 1, 'componentType': component,
                    'count': int(array.shape[0]) if array.ndim > 1 else int(array.size),
                    'type': accessor_type}
        if extra:
            accessor.update(extra)
        document['accessors'].append(accessor)
        return len(document['accessors']) - 1

    for mesh in document['meshes']:
        for primitive in mesh['primitives']:
            attributes = primitive['attributes']
            if primitive.get('mode', 4) != 4 or 'indices' not in primitive or 'NORMAL' not in attributes:
                continue
            if primitive.get('targets'):
                raise ValueError('morph targets are not supported')
            positions = accessor_array(document, binary, attributes['POSITION'])
            faces = accessor_array(document, binary, primitive['indices']).reshape(-1, 3).astype(np.int64)
            src, normals, new_faces, sharp = repair_primitive(positions, faces, sharp_deg, relax)
            stats['primitives'] += 1
            stats['sharpEdges'] += sharp
            stats['verticesBefore'] += len(positions)
            stats['verticesAfter'] += len(src)
            new_attributes = {}
            for key, index in attributes.items():
                values = accessor_array(document, binary, index)[src]
                if key == 'NORMAL':
                    values = normals
                if key == 'TANGENT':
                    raise ValueError('tangents are not supported by this repair; re-derive them')
                accessor = document['accessors'][index]
                extra = {'normalized': True} if accessor.get('normalized') else {}
                if key == 'POSITION':
                    extra.update(min=[float(v) for v in values.min(axis=0)],
                                 max=[float(v) for v in values.max(axis=0)])
                new_attributes[key] = add(values.astype(COMPONENT[accessor['componentType']]),
                                          34962, accessor['type'], extra)
            index_type = np.uint16 if len(src) < 65535 else np.uint32
            primitive['indices'] = add(new_faces.reshape(-1).astype(index_type), 34963, 'SCALAR')
            primitive['attributes'] = new_attributes

    # drop accessors and views nothing references any more, renumbering the rest
    used_accessors = sorted({i for m in document['meshes'] for p in m['primitives']
                             for i in list(p['attributes'].values()) + [p['indices']]})
    remap = {old: new for new, old in enumerate(used_accessors)}
    document['accessors'] = [document['accessors'][i] for i in used_accessors]
    for mesh in document['meshes']:
        for primitive in mesh['primitives']:
            primitive['attributes'] = {k: remap[v] for k, v in primitive['attributes'].items()}
            primitive['indices'] = remap[primitive['indices']]
    used_views = sorted({a['bufferView'] for a in document['accessors']}
                        | {img['bufferView'] for img in document.get('images', []) if 'bufferView' in img})
    view_remap = {old: new for new, old in enumerate(used_views)}
    document['bufferViews'] = [document['bufferViews'][i] for i in used_views]
    chunks = [chunks[i] for i in used_views]
    for accessor in document['accessors']:
        accessor['bufferView'] = view_remap[accessor['bufferView']]
    for image in document.get('images', []):
        if 'bufferView' in image:
            image['bufferView'] = view_remap[image['bufferView']]

    flattened = flatten_transmission(document)
    assert_runtime_glb(document, Path(target).name)
    after = triangles(document)
    if after != before:
        raise ValueError(f'triangle count changed {before} -> {after}')
    document.setdefault('asset', {})['generator'] = 'repair-vehicle-glb-normals.py over ' + \
        document.get('asset', {}).get('generator', 'unknown')
    write_glb(document, chunks, target)
    return dict(stats, triangles=after, transmissionFlattened=flattened, **rig_declaration(document))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--receipt', required=True, help='receipt.json of the shipped generated vehicle')
    parser.add_argument('--output', required=True, help='new directory outside the repository')
    # per tier: a coarser tier spans a curve with fewer faces, so its crease angle is wider
    parser.add_argument('--sharp-deg', default='38,48,60',
                        help='crease angle per LOD tier, comma separated')
    parser.add_argument('--relax', type=int, default=4)
    args = parser.parse_args()

    receipt = json.loads(Path(args.receipt).read_text())
    output = Path(args.output).resolve()
    if output.is_relative_to(ROOT):
        raise ValueError('output must be outside the repository')
    output.mkdir(parents=True, exist_ok=True)
    sharp = [float(v) for v in str(args.sharp_deg).split(',')]
    sharp += [sharp[-1]] * (len(receipt['lods']) - len(sharp))
    lods = []
    previous = []
    for level, lod in enumerate(receipt['lods']):
        source = PUBLIC / lod['file']
        if digest(source) != lod['sha256']:
            raise ValueError(f'{source} does not match its receipt')
        target = output / Path(lod['file']).name
        stats = repair_glb(source, target, sharp[level], args.relax)
        previous.append(dict(file=lod['file'], sha256=lod['sha256'], bytes=lod['bytes']))
        lods.append(dict(file=target.name, sha256=digest(target), bytes=target.stat().st_size,
                         sharpDeg=sharp[level], **stats))
        print(json.dumps(lods[-1]))
    lod0 = PUBLIC / receipt['lods'][0]['file']
    report = dict(
        schemaVersion=1, kind='generated-vehicle', vehicleId=receipt['vehicleId'], body=receipt['body'],
        lengthM=receipt['lengthM'], wheelRadiusM=receipt['wheelRadiusM'], typeIds=receipt['typeIds'],
        generator='scripts/assets/repair-vehicle-glb-normals.py',
        method=METHOD,
        parameters=dict(sharpDeg=sharp, relax=args.relax, weighting='face area x corner angle'),
        source=dict(method=METHOD, file=lod0.name, sha256=digest(lod0), bytes=lod0.stat().st_size),
        sourceSha256=digest(lod0),
        previousLods=previous,
        upstream=dict(generator=receipt.get('assembly', {}).get('generator'),
                      source=receipt.get('source')),
        lods=lods,
        status='candidate; in-game review required')
    (output / 'build-report.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
