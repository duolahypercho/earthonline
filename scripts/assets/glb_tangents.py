"""Tangent repair for exported vehicle GLBs.

Importable from Blender exporters and plain Python alike (numpy only, no bpy).
"""
import json
import struct
from pathlib import Path


def repair_tangents(path):
    """Guarantee unit-length tangents in an exported GLB.

    Blender's tangent export leaves a degenerate vector on an occasional
    collapsed face. The runtime normal map is baked against this basis, so a
    zero tangent silently changes shading. Rewrite only those accessors, keeping
    every other byte and every other accessor untouched.
    """
    import numpy as np
    raw = Path(path).read_bytes()
    json_length = struct.unpack_from('<I', raw, 12)[0]
    document = json.loads(raw[20:20 + json_length])
    binary = bytearray(raw[28 + json_length:])
    formats = {5121: 'u1', 5123: '<u2', 5125: '<u4', 5126: '<f4'}
    dimensions = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}
    repaired = 0
    for mesh in document.get('meshes', []):
        for primitive in mesh.get('primitives', []):
            index = primitive.get('attributes', {}).get('TANGENT')
            if index is None:
                continue
            accessor = document['accessors'][index]
            view = document['bufferViews'][accessor['bufferView']]
            dtype = np.dtype(formats[accessor['componentType']])
            width = dimensions[accessor['type']]
            offset = view.get('byteOffset', 0) + accessor.get('byteOffset', 0)
            stride = view.get('byteStride', width * dtype.itemsize)
            values = np.ndarray((accessor['count'], width), dtype=dtype, buffer=binary,
                                offset=offset, strides=(stride, dtype.itemsize))
            vectors = values[:, :3].astype(np.float64)
            normals = None
            normal_index = primitive.get('attributes', {}).get('NORMAL')
            if normal_index is not None:
                na = document['accessors'][normal_index]
                nv = document['bufferViews'][na['bufferView']]
                normals = np.ndarray((na['count'], dimensions[na['type']]), dtype=np.dtype(formats[na['componentType']]),
                                     buffer=binary, offset=nv.get('byteOffset', 0) + na.get('byteOffset', 0),
                                     strides=(nv.get('byteStride', dimensions[na['type']] * np.dtype(formats[na['componentType']]).itemsize),
                                              np.dtype(formats[na['componentType']]).itemsize))
            unit_normals = None
            if normals is not None:
                unit_normals = normals[:, :3].astype(np.float64)
                lengths_n = np.linalg.norm(unit_normals, axis=1)
                unit_normals = unit_normals / np.maximum(lengths_n[:, None], 1e-12)
            lengths = np.linalg.norm(vectors, axis=1)
            # Blender's exporter leaves both unit-violating and non-orthogonal
            # tangents; the runtime normal map is baked against this basis, so
            # every bad row is repaired, not only the zero ones.
            bad_rows = np.abs(lengths - 1.0) > 1e-5
            if unit_normals is not None:
                dots = np.abs(np.sum(unit_normals * vectors, axis=1))
                bad_rows = bad_rows | (dots > 2e-4)
            bad = np.flatnonzero(bad_rows)
            if not len(bad):
                continue
            for row in bad:
                vector = vectors[row]
                if unit_normals is not None:
                    normal = unit_normals[row]
                    vector = vector - normal * float(vector @ normal)
                    if np.linalg.norm(vector) < 1e-8:
                        axis = np.array([0.0, 1.0, 0.0]) if abs(normal[0]) > 0.9 else np.array([1.0, 0.0, 0.0])
                        vector = np.cross(normal, axis)
                if np.linalg.norm(vector) < 1e-8:
                    vector = np.array([1.0, 0.0, 0.0])
                values[row, :3] = (vector / np.linalg.norm(vector)).astype(dtype)
                repaired += 1
    if repaired:
        Path(path).write_bytes(bytes(raw[:28 + json_length]) + bytes(binary))
    return repaired
