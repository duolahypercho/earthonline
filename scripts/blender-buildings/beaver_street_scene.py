"""Bring the game's near street and supporting terrain into the metric Blender study."""

import json
import os
import subprocess

import bpy
from mathutils import Vector


def load_surface(repo, centre):
    command = [
        "cargo",
        "run",
        "--quiet",
        "--release",
        "-p",
        "eo-street",
        "--example",
        "study_surface",
        "--",
        str(centre[0]),
        str(centre[1]),
        "92",
        "2200",
    ]
    # The authoring package can run from any checkout or Blender working directory.
    result = subprocess.run(
        command,
        cwd=repo / "native",
        env={**os.environ, "EO_REPO_ROOT": str(repo)},
        stdout=subprocess.PIPE,
        check=True,
    )
    return json.loads(result.stdout)


def cut_parcel(vertices, faces, ring, ground):
    """Cut the terrain at the cadastral boundary where the private yard supplies the ground."""
    xs, ys = zip(*ring)
    left, right, bottom, top = min(xs), max(xs), min(ys), max(ys)
    edges = list(zip(ring, ring[1:] + ring[:1]))
    assert sum(a.x * b.y - b.x * a.y for a, b in edges) > 0
    result = []

    def add(poly):
        if len(poly) >= 3:
            start = len(vertices)
            vertices.extend(tuple(p) for p in poly)
            result.append(tuple(range(start, start + len(poly))))

    for face in faces:
        pts = [vertices[i] for i in face]
        if (
            max(p[0] for p in pts) <= left
            or min(p[0] for p in pts) >= right
            or max(p[1] for p in pts) <= bottom
            or min(p[1] for p in pts) >= top
        ):
            result.append(face)
            continue
        pending = [Vector(p) for p in pts]
        for a, b in edges:
            if not pending:
                break
            inside, outside = [], []
            delta = b - a

            def distance(p):
                return delta.x * (p.y - a.y) - delta.y * (p.x - a.x)

            previous = pending[-1]
            prior = distance(previous)
            for p in pending:
                current = distance(p)
                if (current < 0) != (prior < 0):
                    cut = previous.lerp(p, prior / (prior - current))
                    inside.append(cut)
                    outside.append(cut)
                (outside if current < 0 else inside).append(p)
                previous, prior = p, current
            add(outside)
            pending = inside
        # Vertical returns stitch the surveyed soil to the public terrain grade.
        for p, v in zip(pending, pending[1:] + pending[:1]):
            if any(
                abs((b - a).cross(p.xy - a)) < 0.0002
                and abs((b - a).cross(v.xy - a)) < 0.0002
                for a, b in edges
            ):
                add(
                    [
                        p,
                        v,
                        Vector((*v.xy, ground(v.xy) + 0.014)),
                        Vector((*p.xy, ground(p.xy) + 0.014)),
                    ]
                )
    return result


def build_surface(
    collection, root, centre, data, mapped, ground, parcel, earth, material, scan
):
    scale = data["plan_scale"]
    centre_z = ground(mapped(centre))
    reference_height = data["reference_height"]

    def coords(point):
        x, y, z = point
        xy = mapped((x / scale, z / scale))
        return (xy.x, xy.y, centre_z + (y - reference_height))

    # Use the same capped grid and diagonal as the near-level terrain mesh.
    rows = data["ground_rows"]
    cols = len(rows[0])
    vertices = [coords(p) for row in rows for p in row]
    faces = []
    for j in range(len(rows) - 1):
        for i in range(cols - 1):
            a = j * cols + i
            faces.extend(((a, a + cols, a + 1), (a + 1, a + cols, a + cols + 1)))
    terrain = bpy.data.meshes.new("Street terrain · lidar grid")
    faces = cut_parcel(vertices, faces, parcel, ground)
    terrain.from_pydata(vertices, [], faces)
    terrain.materials.append(earth)
    terrain.update()
    collection.objects.link(bpy.data.objects.new(terrain.name, terrain))

    # Colours on the original triangles distinguish the gutter pan, white and
    # yellow markings and curb paint. All edges and heights come from the game.
    layers = (
        ("Carriageway", "worn_asphalt", 3.1),
        ("Curb face", "concrete_floor_worn_001", 1.6),
        ("Curb top", "concrete_floor_worn_001", 1.6),
        ("Sidewalk", "dirty_concrete", 2.0),
        ("Ramp", "dirty_concrete", 2.0),
        ("Verge", "park_dirt", 2.5),
        ("Marking", None, 1),
    )
    by_layer = [[] for _ in layers]
    for tri in data["triangles"]:
        by_layer[tri["layer"]].append(tri)
    for (name, tile, metres), tris in zip(layers, by_layer):
        if not tris:
            continue
        points, colors, faces = [], [], []
        for tri in tris:
            start = len(points)
            verts = [coords(p) for p in tri["positions"]]
            points.extend(verts)
            # Street tessellation accepts either winding and supplies upwards normals.
            forward = (Vector(verts[1]) - Vector(verts[0])).cross(
                Vector(verts[2]) - Vector(verts[0])
            ).z >= 0
            faces.append(
                (start, start + 1, start + 2)
                if forward
                else (start, start + 2, start + 1)
            )
            colors.extend(tri["colors"])
        mesh = bpy.data.meshes.new("Beaver Street · " + name)
        mesh.from_pydata(points, [], faces)
        mesh.update()
        obj = bpy.data.objects.new(mesh.name, mesh)
        collection.objects.link(obj)
        mat = material("Street · " + name, (0.28, 0.27, 0.25))
        tex = scan(root, mat, tile, metres, 0.12) if tile else None
        tree = mat.node_tree
        attr = mesh.color_attributes.new(
            name="Street paint", type="BYTE_COLOR", domain="POINT"
        )
        for entry, packed in zip(attr.data, colors):
            entry.color_srgb = tuple(
                ((packed >> shift) & 255) / 255 for shift in (0, 8, 16)
            ) + (1,)
        color = tree.nodes.new("ShaderNodeVertexColor")
        color.layer_name = attr.name
        surface = color.outputs["Color"]
        if tex:
            # The surveyed material color carries the road markings. Modulate it
            # by the photographed finish to keep both the markings and surface grain.
            mix = tree.nodes.new("ShaderNodeMixRGB")
            mix.blend_type = "OVERLAY"
            mix.inputs[0].default_value = 0.72
            tree.links.new(surface, mix.inputs[1])
            tree.links.new(tex, mix.inputs[2])
            surface = mix.outputs[0]
        tree.links.new(surface, tree.nodes.get("Principled BSDF").inputs["Base Color"])
        mesh.materials.append(mat)
        obj["street_layer"] = name
        obj["exported_triangles"] = len(tris)
    collection["street_source"] = "eo-street near mesh and terrain carpet"
    collection["street_triangles"] = len(data["triangles"])


def horizon(collection, root, centre, data, mapped, ground, local_ids, material):
    """Surveyed hills, rooflines and inventoried crowns across the upper camera's horizon."""
    import gzip
    import math
    import zlib
    import random

    roof_colors = (
        (0.17, 0.155, 0.13),
        (0.23, 0.19, 0.15),
        (0.27, 0.27, 0.25),
        (0.16, 0.17, 0.17),
    )
    wall_colors = (
        (0.59, 0.52, 0.41),
        (0.69, 0.64, 0.53),
        (0.43, 0.47, 0.43),
        (0.57, 0.48, 0.41),
        (0.38, 0.40, 0.40),
    )
    roofs = [material("Distant roof · " + str(i), c) for i, c in enumerate(roof_colors)]
    walls = [
        material("Distant facade · " + str(i), c) for i, c in enumerate(wall_colors)
    ]
    coords, faces, indices = [], [], []
    scale = data["plan_scale"]
    datum = data["reference_height"] - ground(mapped(centre))
    buildings = 0
    for b in data["skyline"]:
        if b["id"] in local_ids:
            continue
        ring = [mapped((x / scale, z / scale)) for x, z in b["perimeter"]]
        if len(ring) < 3:
            continue
        seed = zlib.crc32(b["id"].encode())
        base = b["ground"] - datum
        top = base + b["height"]
        start = len(coords)
        count = len(ring)
        coords.extend((p.x, p.y, base - 3) for p in ring)
        coords.extend((p.x, p.y, top) for p in ring)
        faces.append(tuple(start + count + i for i in range(count)))
        indices.append(seed % len(roofs))
        for i in range(count):
            j = (i + 1) % count
            faces.append((start + i, start + j, start + count + j, start + count + i))
            indices.append(len(roofs) + (seed // 11) % len(walls))
        buildings += 1
    mesh = bpy.data.meshes.new("Rooflines · measured city footprints")
    mesh.from_pydata(coords, [], faces)
    for mat in roofs + walls:
        mesh.materials.append(mat)
    mesh.polygons.foreach_set("material_index", indices)
    mesh.update()
    collection.objects.link(bpy.data.objects.new(mesh.name, mesh))
    collection["background_footprints"] = buildings

    # Canopy centres and crown sizes come from the street-tree inventory.
    # The low-detail crown has overlapping lobes, with an open underside.
    with gzip.open(root / "native/eo-props/data/props-js.json.gz", "rt") as stream:
        trees = json.load(stream)["trees"]
    foliage = [
        material("Distant canopy · " + str(i), c)
        for i, c in enumerate(
            (
                (0.08, 0.115, 0.044),
                (0.051, 0.09, 0.046),
                (0.14, 0.16, 0.065),
                (0.09, 0.14, 0.079),
            )
        )
    ]
    # Instance a small set of crown meshes on the measured inventory points.
    # Each point retains its actual width, height, grade and a stable orientation.
    groves = [[] for _ in range(12)]
    raw = trees["data"]
    for i in range(0, len(raw), trees["stride"]):
        x, z, _form, width, height = raw[i : i + 5]
        dist = math.hypot(x / 10 - centre[0], z / 10 - centre[1]) * scale
        if dist < 72 or dist > 2200 or width <= 0 or height <= 0:
            continue
        xy = mapped((x / 10, z / 10))
        seed = zlib.crc32(str(i).encode())
        groves[seed % len(groves)].append(
            (
                (xy.x, xy.y, ground(xy)),
                (width / 10, width / 10, height / 10),
                (0, 0, math.tau * (seed % 1601) / 1601),
            )
        )
    for group, grove in enumerate(groves):
        if not grove:
            continue
        rng = random.Random(group)
        coords, faces = [], []
        for k in range(5):
            az = k * 2.4 + rng.uniform(-0.3, 0.3)
            d = 0.20 if k else 0
            cx, cy = d * math.cos(az), d * math.sin(az)
            cz = rng.uniform(0.64, 0.79) if k else 0.81
            radius, high = (0.33, 0.19) if k else (0.36, 0.22)
            start = len(coords)
            for j, (r, v) in enumerate(
                ((0.15, -1), (0.85, -0.58), (1, 0.13), (0.67, 0.78), (0.13, 1))
            ):
                for n in range(10):
                    a = n * math.tau / 10 + az + 0.18 * (j % 2)
                    jitter = rng.uniform(0.85, 1.13)
                    coords.append(
                        (
                            cx + radius * r * jitter * math.cos(a),
                            cy + radius * r * jitter * math.sin(a),
                            cz + high * v * jitter,
                        )
                    )
            for j in range(4):
                for n in range(10):
                    a = start + 10 * j + n
                    b = start + 10 * j + (n + 1) % 10
                    faces.append((a, b, b + 10, a + 10))
            faces.append(tuple(start + 40 + n for n in range(10)))
        template = bpy.data.meshes.new("Canopy branches · " + str(group))
        template.from_pydata(coords, [], faces)
        template.polygons.foreach_set("use_smooth", [True] * len(faces))
        template.materials.append(foliage[group % len(foliage)])
        template.update()
        prototype = bpy.data.objects.new(template.name, template)
        positions, sizes, rotations = zip(*grove)
        points = bpy.data.meshes.new("Inventoried crowns · " + str(group))
        points.from_pydata(positions, [], [])
        for name, values in (
            ("Crown dimensions", sizes),
            ("Crown orientation", rotations),
        ):
            attr = points.attributes.new(name, "FLOAT_VECTOR", "POINT")
            attr.data.foreach_set("vector", [c for value in values for c in value])
        emitter = bpy.data.objects.new(points.name, points)
        collection.objects.link(emitter)
        nodes = bpy.data.node_groups.new(points.name, "GeometryNodeTree")
        nodes.interface.new_socket(
            name="Geometry", in_out="INPUT", socket_type="NodeSocketGeometry"
        )
        nodes.interface.new_socket(
            name="Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry"
        )
        instance = nodes.nodes.new("GeometryNodeInstanceOnPoints")
        source = nodes.nodes.new("GeometryNodeObjectInfo")
        source.inputs["Object"].default_value = prototype
        nodes.links.new(source.outputs["Geometry"], instance.inputs["Instance"])
        for name, slot in (
            ("Crown dimensions", "Scale"),
            ("Crown orientation", "Rotation"),
        ):
            attribute = nodes.nodes.new("GeometryNodeInputNamedAttribute")
            attribute.data_type = "FLOAT_VECTOR"
            attribute.inputs["Name"].default_value = name
            nodes.links.new(attribute.outputs["Attribute"], instance.inputs[slot])
        nodes.links.new(
            nodes.nodes.new("NodeGroupInput").outputs["Geometry"],
            instance.inputs["Points"],
        )
        nodes.links.new(
            instance.outputs["Instances"],
            nodes.nodes.new("NodeGroupOutput").inputs["Geometry"],
        )
        emitter.modifiers.new("Measured crowns", "NODES").node_group = nodes
    collection["background_trees"] = sum(map(len, groves))
