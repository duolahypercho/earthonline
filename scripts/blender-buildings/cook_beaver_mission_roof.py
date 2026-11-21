"""Cook the installed clay into the game's parcel frame using the same tile surfaces."""

import hashlib
import json
import struct
import zlib
import numpy as np


def png(path, pixels):
    def chunk(kind, blob):
        return (
            struct.pack(">I", len(blob))
            + kind
            + blob
            + struct.pack(">I", zlib.crc32(kind + blob))
        )

    h, w, channels = pixels.shape
    color_type = {3: 2, 4: 6}[channels]
    rows = b"".join(b"\0" + row.tobytes() for row in pixels)
    path.write_bytes(
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, color_type, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, 9))
        + chunk(b"IEND", b"")
    )


def manifest(path, public, meta):
    data = path.read_bytes()
    meta.update(
        url="/" + path.relative_to(public).as_posix(),
        bytes=len(data),
        sha256=hashlib.sha256(data).hexdigest(),
        build="scripts/blender-buildings/build_beaver_mission_roof.py --runtime",
    )
    path.with_suffix(".provenance.json").write_text(json.dumps(meta, indent=2) + "\n")


def cook(batches, fittings, site, pixels, public, provenance):
    directory = (
        public / "assets/buildings/beaver-terrace/finishes/patina/furniture/photo"
    )
    # Z-up metres in Blender -> Y-up game coordinates in the house parcel basis.
    transform = np.array([[1.0, 0, 0], [0, 0, 1.0], [0, -1.0, 0]]) @ np.linalg.inv(
        np.asarray(site)[:3, :3]
    )
    inner_rows = [0, 3]
    arc = [0, 2, 4, 5, 6, 8, 10]
    positions = []
    normals = []
    coords = []
    indices = []
    vertex_count = 0

    def strip(p, n, uv):
        nonlocal vertex_count
        tiles, _, k, _ = p.shape
        if n is None:
            along = p[:, 1:2] - p[:, 0:1]
            n = np.cross(along, np.gradient(p, axis=2))
        n = n @ np.linalg.inv(transform)
        lengths = np.linalg.norm(n, axis=-1, keepdims=True)
        assert np.min(lengths) > 1e-8
        positions.append((p.reshape(-1, 3) @ transform.T).astype("<f4"))
        normals.append((n / lengths).reshape(-1, 3).astype("<f4"))
        mapped = uv.reshape(-1, 2).copy()
        mapped[:, 1] = 1 - mapped[:, 1]
        coords.append(np.rint(np.clip(mapped, 0, 1) * 65535).astype("<u2"))
        row = (
            np.arange(tiles)[:, None] * 2 * k + np.arange(k - 1)[None, :] + vertex_count
        ).ravel()
        indices.append(
            np.stack((row, row + k, row + k + 1, row, row + k + 1, row + 1), axis=1)
            .astype("<u4")
            .ravel()
        )
        vertex_count += tiles * 2 * k

    for batch in batches:
        p = np.asarray(batch.verts).reshape(-1, 2, 4, 11, 3)
        uv = np.asarray(batch.uv).reshape(-1, 2, 4, 11, 2)
        # Derive normals on the high-detail mold before reducing axial and angular rings.
        top = p[:, 0]
        n = np.cross(np.gradient(top, axis=1), np.gradient(top, axis=2))
        strip(
            top[:, inner_rows][:, :, arc],
            n[:, inner_rows][:, :, arc],
            uv[:, 0, inner_rows][:, :, arc],
        )
        # The camera can see the underside at a rake or lip. Cover those curved
        # faces more coarsely than the exposed surface and retain the fired edge.
        bottom = p[:, 1]
        coarse = [0, 3, 5, 7, 10]
        strip(bottom[:, [3, 0]][:, :, coarse], None, uv[:, 1, [3, 0]][:, :, coarse])
        strip(p[:, [1, 0], 0][:, :, arc], None, uv[:, [1, 0], 0][:, :, arc])
        for k, order in ((0, [3, 0]), (10, [0, 3])):
            strip(
                p[:, :, order, k].transpose(0, 2, 1, 3),
                None,
                uv[:, :, order, k].transpose(0, 2, 1, 3),
            )
    # The three molded terminals are already small. Keep their exact profiles.
    for obj in fittings:
        mesh = obj.data
        mesh.calc_loop_triangles()
        for tri in mesh.loop_triangles:
            corners = tri.loops
            pos = np.asarray(
                [mesh.vertices[mesh.loops[i].vertex_index].co[:] for i in corners]
            )
            normal = np.asarray(
                [mesh.corner_normals[i].vector[:] for i in corners]
            ) @ np.linalg.inv(transform)
            tex = np.asarray([mesh.uv_layers.active.data[i].uv[:] for i in corners])
            tex[:, 1] = 1 - tex[:, 1]
            positions.append((pos @ transform.T).astype("<f4"))
            normals.append(
                (normal / np.linalg.norm(normal, axis=1, keepdims=True)).astype("<f4")
            )
            coords.append(np.rint(tex * 65535).astype("<u2"))
            indices.append(np.arange(vertex_count, vertex_count + 3, dtype="<u4"))
            vertex_count += 3
    pos = np.concatenate(positions)
    arrays = (
        pos,
        np.concatenate(normals),
        np.concatenate(coords),
        np.concatenate(indices),
    )
    views = []
    accessors = []
    blob = bytearray()
    for i, (array, ctype, kind) in enumerate(
        zip(arrays, (5126, 5126, 5123, 5125), ("VEC3", "VEC3", "VEC2", "SCALAR"))
    ):
        size = array.nbytes
        start = len(blob)
        blob.extend(array.tobytes())
        blob += b"\0" * (-len(blob) % 4)
        views.append(
            dict(
                buffer=0,
                byteOffset=start,
                byteLength=size,
                target=34963 if i == 3 else 34962,
            )
        )
        ac = dict(bufferView=i, componentType=ctype, count=len(array), type=kind)
        if i == 0:
            ac.update(min=pos.min(0).tolist(), max=pos.max(0).tolist())
        if i == 2:
            ac["normalized"] = True
        accessors.append(ac)
    material = dict(
        name="mission-roof-clay",
        pbrMetallicRoughness={
            "baseColorFactor": [1, 1, 1, 1],
            "metallicFactor": 0,
            "roughnessFactor": 0.8,
        },
    )
    doc = dict(
        asset={"version": "2.0", "generator": "Earth Online Mission roof cook"},
        scene=0,
        scenes=[dict(nodes=[0])],
        nodes=[dict(name="Fitted clay roof", mesh=0)],
        materials=[material],
        meshes=[
            {
                "primitives": [
                    {
                        "attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2},
                        "indices": 3,
                        "material": 0,
                    }
                ]
            }
        ],
        accessors=accessors,
        bufferViews=views,
        buffers=[dict(byteLength=len(blob))],
    )
    encoded = json.dumps(doc, separators=(",", ":")).encode()
    encoded += b" " * (-len(encoded) % 4)
    model = directory / "mission-roof.glb"
    model.write_bytes(
        struct.pack(
            "<4sIIII",
            b"glTF",
            2,
            28 + len(encoded) + len(blob),
            len(encoded),
            0x4E4F534A,
        )
        + encoded
        + struct.pack("<II", len(blob), 0x004E4942)
        + blob
    )
    manifest(
        model,
        public,
        dict(
            source="Original modeled roof, independently fitted to the 21 Beaver parcel.",
            license="MIT",
            attribution="Earth Online contributors",
            profile=provenance["profile_source"],
            source_scene=provenance["base_scene"],
            tiles=sum(b.pieces for b in batches) + len(fittings),
            triangles=len(arrays[-1]) // 3,
        ),
    )
    rgb = np.concatenate([pixels[k] for k in ("albedo", "normal", "arm")])
    rgba = np.concatenate(
        [rgb, np.full((*rgb.shape[:2], 1), 255, dtype=np.uint8)], axis=2
    )
    texture = directory / "mission-clay.png"
    png(texture, rgba)
    manifest(
        texture,
        public,
        dict(
            source=["https://polyhaven.com/a/clay_roof_tiles_02"],
            license=provenance["texture_license"],
            attribution=provenance["attribution"],
            inputs=provenance["inputs"],
            families=["mission-roof-clay"],
            metres_per_tile=1.0,
            normal_strength=0.48,
            paint_mix=0.0,
            process="Sixteen individual kiln faces, scan normals high-passed to remove the original tile curvature.",
        ),
    )
