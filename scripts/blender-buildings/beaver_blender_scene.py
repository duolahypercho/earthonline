"""Place the editable house on its parcel and light the review cameras in Cycles."""

import gzip
import hashlib
import json
import math
import os
import struct
from itertools import pairwise
from pathlib import Path

import beaver_photo
import bpy
import numpy as np
from mathutils import Matrix, Vector


def terrain_sampler(root):
    with gzip.open(root / "public/data/sf/sf-elevation.bin.gz", "rb") as stream:
        if stream.read(4) != b"EOEL":
            raise ValueError("Expected the city lidar grid")
        meta = json.loads(stream.read(struct.unpack("<I", stream.read(4))[0]))
        count = meta["width"] * meta["height"]
        delta = np.frombuffer(stream.read(count * 2), dtype="<i2").reshape(
            meta["height"], meta["width"]
        )
    missing = delta == meta["nodata"]
    heights = np.cumsum(np.where(missing, 0, delta), axis=1, dtype=np.int32) * 0.01
    heights[missing] = np.nan

    def sample(p):
        x = (p[0] - meta["originX"]) / meta["cellSize"] - 0.5
        y = (p[1] - meta["originZ"]) / meta["cellSize"] - 0.5
        i, j = math.floor(x), math.floor(y)
        u, v = x - i, y - j
        cells = heights[j : j + 2, i : i + 2]
        weights = np.array([[(1 - u) * (1 - v), u * (1 - v)], [(1 - u) * v, u * v]])
        good = np.isfinite(cells)
        if not good.any():
            raise ValueError("Missing ground survey inside the house study area")
        return np.sum(cells[good] * weights[good]) / weights[good].sum()

    return sample


def material(name, rgb, rough=0.75):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (*rgb, 1)
    mat.use_nodes = True
    shader = mat.node_tree.nodes.get("Principled BSDF")
    shader.inputs["Base Color"].default_value = (*rgb, 1)
    shader.inputs["Roughness"].default_value = rough
    return mat


class MeshBatch:
    def __init__(self):
        self.groups = {}

    def face(self, mat, coords):
        verts, faces = self.groups.setdefault(mat, ([], []))
        start = len(verts)
        verts.extend(coords)
        faces.append(tuple(range(start, len(verts))))

    def box(self, mat, low, high):
        a, b, c = low
        x, y, z = high
        q = [
            (a, b, c),
            (x, b, c),
            (x, y, c),
            (a, y, c),
            (a, b, z),
            (x, b, z),
            (x, y, z),
            (a, y, z),
        ]
        for face in (
            (3, 2, 1, 0),
            (4, 5, 6, 7),
            (0, 1, 5, 4),
            (1, 2, 6, 5),
            (2, 3, 7, 6),
            (3, 0, 4, 7),
        ):
            self.face(mat, [q[i] for i in face])

    def finish(self, label, collection):
        for mat, (verts, faces) in self.groups.items():
            name = label + " · " + mat.name
            mesh = bpy.data.meshes.new(name)
            mesh.from_pydata(verts, [], faces)
            mesh.materials.append(mat)
            mesh.update()
            ob = bpy.data.objects.new(name, mesh)
            collection.objects.link(ob)


def place_shell(scene, footprint, components, dimensions, planted, grade):
    # All distances within furniture remain metric. Architecture follows the
    # game's installed footprint, including its small measured skew.
    origin = footprint[1]
    across = (footprint[2] - origin) * 1.25 / 7
    back = (footprint[0] - origin) * 1.25 / 13
    axis = across.normalized()
    side = Vector((-axis.y, axis.x))
    site = Matrix(
        (
            (across.length, -axis.dot(back), 0, 0),
            (0, side.dot(back), 0, 0),
            (0, 0, 1, 0),
            (0, 0, 0, 1),
        )
    )

    def mapped(p):
        q = (Vector(p) - origin) * 1.25
        return Vector((q.dot(axis), -q.dot(side)))

    datum = max(grade(p) for p in footprint) * 1.25 + 0.22

    def ground(q):
        p = origin + (axis * q.x - side * q.y) / 1.25
        return grade(p) * 1.25 - datum

    plants = {item["name"]: item for item in planted}
    scene.view_layers[0].update()
    for name, (handle, meshes) in components.items():
        old = handle.location.copy()
        new = site @ old
        if name in dimensions.GROUP:
            x, h, z = dimensions.ASSEMBLIES[dimensions.GROUP[name]][0]
            anchor = Vector((x, -z, h))
            new = site @ anchor + old - anchor
        elif name in plants:
            plant = plants[name]
            if "survey_xz" in plant:
                new.xy = mapped(plant["survey_xz"])
            new.z = ground(new.xy)
        handle.location = new
        bpy.data.collections[name].instance_offset = new
        if name not in dimensions.GROUP and not (
            name in plants and "survey_xz" in plants[name]
        ):
            for obj in meshes:
                obj.data.transform(site)
        handle["component"] = name
    for obj in scene.objects:
        if obj.type == "MESH" and not obj.parent:
            obj.data.transform(site)
    return site, mapped, ground


def photographic_surface(root, material, asset, metres, tint=0.0):
    """Use the pinned scan at its physical size and pack its sources into the scene."""
    library = os.environ.get("EO_CC0_ROOT")
    if not library:
        return None
    manifest = json.loads((root / "native/assets/cc0-manifest.json").read_text())
    record = next(entry for entry in manifest["assets"] if entry["id"] == asset)
    nd = material.node_tree.nodes
    link = material.node_tree.links.new
    uv = nd.new("ShaderNodeUVMap")
    uv.uv_map = "Site survey UV"
    mapping = nd.new("ShaderNodeVectorMath")
    mapping.operation = "SCALE"
    mapping.inputs["Scale"].default_value = 1 / metres
    link(uv.outputs[0], mapping.inputs[0])
    bs = nd.get("Principled BSDF")
    albedo = None
    for f in record["files"]:
        if f["role"] not in ("albedo", "normal", "arm", "roughness"):
            continue
        path = Path(library) / f["path"]
        if hashlib.sha256(path.read_bytes()).hexdigest() != f["sha256"]:
            raise ValueError("Scan checksum failed: " + f["path"])
        image = bpy.data.images.load(str(path), check_existing=True)
        if f["role"] != "albedo":
            image.colorspace_settings.name = "Non-Color"
        if not image.packed_file:
            image.pack()
        tex = nd.new("ShaderNodeTexImage")
        tex.image = image
        link(mapping.outputs[0], tex.inputs[0])
        if f["role"] == "albedo":
            dye = nd.new("ShaderNodeMixRGB")
            dye.blend_type = "SOFT_LIGHT"
            dye.inputs[0].default_value = tint
            dye.inputs[2].default_value = material.diffuse_color
            if material.name.startswith("Context"):
                dye.blend_type = "MIX"
                dye.inputs[0].default_value = 0.66
            link(tex.outputs[0], dye.inputs[1])
            albedo = dye.outputs[0]
            link(albedo, bs.inputs["Base Color"])
        elif f["role"] == "normal":
            normal = nd.new("ShaderNodeNormalMap")
            normal.uv_map = "Site survey UV"
            normal.inputs["Strength"].default_value = 0.72
            link(tex.outputs[0], normal.inputs["Color"])
            link(normal.outputs[0], bs.inputs["Normal"])
        elif f["role"] == "arm":
            channels = nd.new("ShaderNodeSeparateColor")
            link(tex.outputs[0], channels.inputs[0])
            link(channels.outputs["Green"], bs.inputs["Roughness"])
        else:
            link(tex.outputs[0], bs.inputs["Roughness"])
    material["scan_page"] = record["page"]
    material["scan_license"] = record["licence"]
    material["scan_authors"] = ", ".join(record.get("authors", []))
    return albedo


def site_model(scene, root, city, house, mapped, ground, site):
    collection = bpy.data.collections.new("Site · parcel and surrounding buildings")
    scene.collection.children.link(collection)
    soil = next(m for m in bpy.data.materials if m.name.startswith("garden-topsoil@"))
    soil = soil.copy()
    soil.name = "Courtyard · dirt and sparse clover"
    nodes = soil.node_tree.nodes
    link = soil.node_tree.links.new
    shader = nodes.get("Principled BSDF")
    base = shader.inputs["Base Color"].links[0].from_socket
    tex = nodes.new("ShaderNodeTexCoord")
    broad = nodes.new("ShaderNodeTexNoise")
    broad.inputs["Scale"].default_value = 0.73
    broad.inputs["Detail"].default_value = 3.3
    link(tex.outputs["Object"], broad.inputs[0])
    cover = nodes.new("ShaderNodeValToRGB")
    cover.color_ramp.elements[0].position = 0.37
    cover.color_ramp.elements[1].position = 0.65
    link(broad.outputs["Fac"], cover.inputs[0])
    mix = nodes.new("ShaderNodeMixRGB")
    link(cover.outputs["Color"], mix.inputs[0])
    link(base, mix.inputs[1])
    lawn = nodes.new("ShaderNodeTexNoise")
    lawn.inputs["Scale"].default_value = 96
    lawn.inputs["Detail"].default_value = 2.2
    link(tex.outputs["Object"], lawn.inputs[0])
    leaf = nodes.new("ShaderNodeValToRGB")
    leaf.color_ramp.elements[0].color = (0.038, 0.050, 0.015, 1)
    leaf.color_ramp.elements[1].color = (0.19, 0.228, 0.068, 1)
    link(lawn.outputs[0], leaf.inputs[0])
    grass_scan = photographic_surface(root, soil, "leafy_grass", 2.0)
    link(grass_scan or leaf.outputs[0], mix.inputs[2])
    link(mix.outputs[0], shader.inputs["Base Color"])
    # Mesh the actual private lot at the sampled grade. UVs use installed metres.
    bounds = sorted(
        [mapped(p) for p in beaver_photo.rear_parcel(root)["survey_xz"]],
        key=lambda p: p.y,
    )
    rear = sorted(bounds[:2], key=lambda p: p.x)
    front = sorted(bounds[2:], key=lambda p: p.x)
    yard = MeshBatch()

    def surface(u, v):
        p = front[0].lerp(rear[0], v).lerp(front[1].lerp(rear[1], v), u)
        return Vector((p.x, p.y, ground(p) + 0.014))

    for i in range(18):
        for j in range(70):
            yard.face(
                soil,
                [
                    surface(i / 18, j / 70),
                    surface(i / 18, (j + 1) / 70),
                    surface((i + 1) / 18, (j + 1) / 70),
                    surface((i + 1) / 18, j / 70),
                ],
            )
    yard.finish("Assessor parcel " + str(house["addr"]), collection)
    # In Cycles a new site mesh needs the same UV set as the packed house soil.
    for obj in collection.objects:
        uv = obj.data.uv_layers.new(name="Photographic finish")
        for loop in obj.data.loops:
            p = obj.data.vertices[loop.vertex_index].co
            uv.data[loop.index].uv = (p.x / 1.85, p.y / 1.85)
    # Narrow physical blades interrupt the flat soil silhouette along the two
    # boundaries. Each root follows the same four-corner survey and lidar.
    handles = MeshBatch()
    rng = np.random.default_rng(2119)
    grasses = [
        material("Courtyard · grass " + str(i), rgb)
        for i, rgb in enumerate(
            (
                (0.073, 0.084, 0.024),
                (0.137, 0.147, 0.044),
                (0.21, 0.16, 0.067),
                (0.048, 0.075, 0.029),
            )
        )
    ]
    for i in range(2200):
        u, v = rng.random(2)
        if 0.08 < u < 0.92 and rng.random() > 0.36:
            continue
        root_p = surface(u, v)
        if root_p.y > -0.9 * 13 * site[1][1]:
            continue
        for j in range(4):
            a = rng.uniform(0, math.tau)
            d = Vector((math.cos(a), math.sin(a), 0))
            span = Vector((-d.y, d.x, 0)) * rng.uniform(0.002, 0.005)
            h = rng.uniform(0.045, 0.13)
            handles.face(
                grasses[(i + j) % 4],
                [
                    root_p - span,
                    root_p + span,
                    root_p + 0.22 * h * d + Vector((0, 0, 0.62 * h)) + span * 0.6,
                    root_p + 0.70 * h * d + Vector((0, 0, h)),
                ],
            )
    handles.finish("Boundary grass", collection)
    # The city model supplies the adjoining footprint positions and heights.
    # Window bays, cladding, coping and glazing are schematic context.
    from mathutils.geometry import tessellate_polygon

    wallm = [
        material("Context · siding " + str(i), rgb)
        for i, rgb in enumerate(
            (
                (0.286, 0.289, 0.26),
                (0.236, 0.262, 0.242),
                (0.411, 0.374, 0.30),
                (0.328, 0.32, 0.298),
            )
        )
    ]
    trim = material("Context · painted timber", (0.49, 0.46, 0.39))
    glass = material("Context · glazing", (0.034, 0.053, 0.058), 0.15)
    bitumen = material("Context · mineral roofing", (0.098, 0.094, 0.082), 0.89)
    for mat, scan, metres in zip(
        wallm,
        (
            "worn_plaster_wall",
            "WoodSiding009",
            "wood_textured_concrete",
            "WoodSiding011",
        ),
        (1.8, 2.5, 1.8, 2.7),
    ):
        photographic_surface(root, mat, scan, metres, 0.52)
    photographic_surface(root, bitumen, "tarred_gravel", 2.2, 0.4)
    stacks = material("Context · chimney masonry", (0.246, 0.122, 0.077))
    blocks = MeshBatch()
    own = np.array(house["centroid"])
    for b in city["detailBuildings"]:
        if b["id"] == house["id"] or np.linalg.norm(np.array(b["centroid"]) - own) > 64:
            continue
        xy = [mapped(p) for p in zip(b["points"][::2], b["points"][1::2])]
        if len(xy) < 3:
            continue
        mat = wallm[b["id"] % len(wallm)]
        base = max(ground(p) for p in xy)
        top = base + b["height"]
        mid = sum(xy, Vector((0.0, 0.0))) / len(xy)
        roof = [Vector((p.x, p.y, top)) for p in xy]
        for tri in tessellate_polygon([roof]):
            blocks.face(bitumen, [roof[i] for i in tri])
        if b["height"] > 8 and len(xy) <= 10:
            vent = xy[0].lerp(mid, 0.33)
            blocks.box(
                stacks,
                (vent.x - 0.25, vent.y - 0.39, top - 0.02),
                (vent.x + 0.25, vent.y + 0.39, top + 0.67),
            )
            blocks.box(
                trim,
                (vent.x - 0.276, vent.y - 0.42, top + 0.64),
                (vent.x + 0.276, vent.y + 0.42, top + 0.73),
            )
        for a, bp in zip(xy, xy[1:] + xy[:1]):
            run = bp - a
            if run.length < 0.20:
                continue
            n = Vector((run.y, -run.x)).normalized()
            if n.dot((a + bp) * 0.5 - mid) < 0:
                n = -n

            def pt(t, h, offset=0, base=a, along=run, outward=n):
                p = base + t * along + outward * offset
                return (p.x, p.y, h)

            def panel(finish, t0, t1, h0, h1, depth=0):
                blocks.face(
                    finish,
                    [
                        pt(t0, h0, depth),
                        pt(t1, h0, depth),
                        pt(t1, h1, depth),
                        pt(t0, h1, depth),
                    ],
                )

            # Each sash occupies an actual opening. Reveals, drip, casing and
            # overlapping sashes contribute real shadows even through another window.
            for low, high, outset in (
                (top - 0.18, top - 0.11, 0.085),
                (top - 0.11, top - 0.065, 0.137),
                (top - 0.065, top + 0.025, 0.19),
            ):
                panel(trim, 0, 1, low, high, outset)
                blocks.face(
                    trim,
                    [
                        pt(0, low, 0),
                        pt(1, low, 0),
                        pt(1, low, outset),
                        pt(0, low, outset),
                    ],
                )
            if run.length < 2.1:
                panel(mat, 0, 1, base - 2, top)
                continue
            bays = max(1, round(run.length / 3.7))
            floors = max(1, round(b["height"] / 3.25))
            dh = b["height"] / floors
            hw = min(0.25 / bays, 0.70 / run.length)
            rail = 0.043 / run.length
            margin = 0.062 / run.length
            for f in range(floors):
                sill = base + 0.83 + f * dh
                head = sill + min(1.65, dh - 0.93)
                panel(mat, 0, 1, base - 2 if not f else base + f * dh, sill)
                panel(mat, 0, 1, head, base + (f + 1) * dh)
                end = 0.0
                for k in range(bays):
                    t = (k + 0.5) / bays
                    left, right = t - hw, t + hw
                    if left > end:
                        panel(mat, end, left, sill, head)
                    end = right
                    for edge, sign in ((left, -1), (right, 1)):
                        blocks.face(
                            trim,
                            [
                                pt(edge, sill, -0.12),
                                pt(edge, sill, 0.018),
                                pt(edge, head, 0.018),
                                pt(edge, head, -0.12),
                            ],
                        )
                        panel(
                            trim,
                            min(edge, edge + sign * margin),
                            max(edge, edge + sign * margin),
                            sill - 0.012,
                            head + 0.054,
                            0.025,
                        )
                    for h in (sill, head):
                        blocks.face(
                            trim,
                            [
                                pt(left, h, -0.13),
                                pt(right, h, -0.13),
                                pt(right, h, 0.045),
                                pt(left, h, 0.045),
                            ],
                        )
                    panel(
                        trim, left - margin, right + margin, sill - 0.047, sill, 0.084
                    )
                    blocks.face(
                        trim,
                        [
                            pt(left - margin, sill, -0.015),
                            pt(right + margin, sill, -0.015),
                            pt(right + margin, sill, 0.084),
                            pt(left - margin, sill, 0.084),
                        ],
                    )
                    panel(
                        trim, left - margin, right + margin, head, head + 0.070, 0.053
                    )
                    panel(glass, left, right, sill, head, -0.121)
                    for q in (left, right - rail):
                        panel(trim, q, q + rail, sill, head, -0.070)
                    for hh in (sill + 0.024, sill + 0.81, head - 0.048):
                        panel(trim, left, right, hh, hh + 0.035, -0.065)
                    if b["id"] % 3 == 1:
                        panel(
                            trim, t - rail * 0.33, t + rail * 0.33, sill, head, -0.076
                        )
                panel(mat, end, 1, sill, head)
    blocks.finish("Neighbouring footprints", collection)
    # The private tiled steps are absent from the modular wall shell until installed.
    apron = MeshBatch()
    mortar = material("Porch · sanded grout", (0.205, 0.172, 0.125))
    clay = next(m for m in bpy.data.materials if m.name.startswith("terracotta-paver@"))
    for i in range(10):
        z = -(i + 1) * 0.33
        h = -i * 0.145
        corners = [
            site @ Vector((x, -q, h)) for x in (5.15, 6.75) for q in (z, z + 0.33)
        ]
        if i and any(p.z <= ground(p.xy) + 0.025 for p in corners):
            break
        lo, hi = corners[0], corners[-1]
        apron.box(mortar, (lo.x, min(lo.y, hi.y), h - 0.50), (hi.x, max(lo.y, hi.y), h))
        for k in range(5):
            a = lo.x + (hi.x - lo.x) * k / 5 + 0.0035
            b = lo.x + (hi.x - lo.x) * (k + 1) / 5 - 0.0035
            near = max(lo.y, hi.y)
            apron.box(
                clay, (a, min(lo.y, hi.y) + 0.003, h), (b, near - 0.003, h + 0.009)
            )
            # A rounded bullnose catches a narrow highlight, above a separate tiled riser.
            profile = [
                (near - 0.003, h + 0.009),
                (near + 0.010, h + 0.008),
                (near + 0.017, h + 0.001),
                (near + 0.018, h - 0.011),
                (near + 0.012, h - 0.019),
                (near, h - 0.022),
            ]
            for (y0, h0), (y1, h1) in pairwise(profile):
                apron.face(clay, [(a, y0, h0), (b, y0, h0), (b, y1, h1), (a, y1, h1)])
            apron.box(clay, (a, near - 0.004, h - 0.142), (b, near + 0.003, h - 0.024))
    apron.finish("Tiled entrance stair", collection)
    for obj in collection.objects:
        if not obj.name.startswith("Tiled entrance stair"):
            continue
        uv = obj.data.uv_layers.new(name="Photographic finish")
        for face in obj.data.polygons:
            axis = max(range(3), key=lambda i: abs(face.normal[i]))
            pair = ((1, 2), (0, 2), (0, 1))[axis]
            for i in face.loop_indices:
                p = obj.data.vertices[obj.data.loops[i].vertex_index].co
                uv.data[i].uv = (p[pair[0]] / 0.36, p[pair[1]] / 0.36)
    # Broad receiving terrain sits underneath the actual parcel mesh.
    earth = material("Ground beyond the parcel", (0.147, 0.139, 0.116))
    photographic_surface(root, earth, "park_dirt", 3.0, 0.25)
    land = MeshBatch()
    for x in range(-54, 58, 4):
        for y in range(-56, 58, 4):
            land.face(
                earth,
                [
                    (a, b, ground(Vector((a, b))) - 0.04)
                    for a, b in ((x, y), (x + 4, y), (x + 4, y + 4), (x, y + 4))
                ],
            )
    land.finish("Surrounding grade", collection)
    for obj in collection.objects:
        uv = obj.data.uv_layers.new(name="Site survey UV")
        for poly in obj.data.polygons:
            nx, ny, nz = poly.normal
            # For vertical faces V measures up. U follows the wall, including
            # oblique wings of the adjoining measured footprints.
            for i in poly.loop_indices:
                p = obj.data.vertices[obj.data.loops[i].vertex_index].co
                uv.data[i].uv = (
                    (p.x, p.y) if abs(nz) > 0.6 else (p.x * ny - p.y * nx, p.z)
                )
    orchard(collection, surface(0.30, 0.93))
    collection["footprint_source"] = "public/data/sf/sf-city.json.gz"
    collection["grade_source"] = "public/data/sf/sf-elevation.bin.gz"
    collection["private_parcel"] = "DataSF 3561049"


def orchard(collection, root):
    """A young, open citrus in the private garden, with jointed stems and leaf midribs."""
    rng = np.random.default_rng(338)
    bark = material("Citrus · bark", (0.135, 0.095, 0.052))
    tones = [
        material("Citrus leaf " + str(i), c, 0.49)
        for i, c in enumerate(
            (
                (0.019, 0.067, 0.022),
                (0.044, 0.118, 0.027),
                (0.072, 0.144, 0.039),
                (0.069, 0.112, 0.024),
            )
        )
    ]
    batch = MeshBatch()

    def stem(a, b, r0, r1):
        axis = (b - a).normalized()
        side = axis.cross(
            Vector((0, 0, 1)) if abs(axis.z) < 0.88 else Vector((1, 0, 0))
        ).normalized()
        across = axis.cross(side)
        sides = 8 if r0 > 0.016 else 5
        circles = [
            side * math.cos(i * math.tau / sides)
            + across * math.sin(i * math.tau / sides)
            for i in range(sides)
        ]
        for v, w in zip(circles, circles[1:] + circles[:1]):
            batch.face(bark, [a + r0 * v, a + r0 * w, b + r1 * w, b + r1 * v])

    up = Vector((0, 0, 1))
    trunk = [
        root,
        root + Vector((0.08, -0.04, 0.72)),
        root + Vector((0.13, 0.02, 1.55)),
        root + Vector((0.02, -0.06, 2.15)),
    ]
    for i, (a, b) in enumerate(pairwise(trunk)):
        stem(a, b, 0.073 - 0.017 * i, 0.062 - 0.017 * i)
    for limb in range(13):
        angle = limb * 2.40 + rng.uniform(-0.24, 0.24)
        outward = Vector((math.cos(angle), math.sin(angle), 0))
        lat = up.cross(outward)
        a = trunk[1].lerp(trunk[-1], limb / 14)
        b = a + outward * rng.uniform(0.36, 0.67) + up * rng.uniform(0.50, 0.88)
        stem(a, b, 0.032, 0.023)
        for fork in range(4):
            start = a.lerp(b, 0.43 + 0.18 * fork)
            reach = (
                outward * rng.uniform(0.48, 0.88)
                + lat * rng.uniform(-0.64, 0.64)
                + up * rng.uniform(0.36, 0.75)
            )
            tip = start + reach
            stem(start, tip, 0.020, 0.011)
            for j in range(7):
                twig = tip.lerp(start, j * 0.10)
                side = 1 if j % 2 else -1
                shoot = (
                    reach * 0.37
                    + lat * side * rng.uniform(0.18, 0.36)
                    + up * rng.uniform(0.11, 0.27)
                )
                stem(twig, twig + shoot, 0.0065, 0.0017)
                for k in range(11):
                    p = twig + shoot * (k + 0.25) / 11
                    face = 1 if k % 2 else -1
                    d = (
                        outward * 0.57
                        + lat * face * rng.uniform(0.52, 1.15)
                        - up * rng.uniform(-0.24, 0.68)
                    ).normalized()
                    n = up.cross(d).normalized() * rng.uniform(0.018, 0.028)
                    length = rng.uniform(0.078, 0.137)
                    mid = p + d * length * 0.49
                    end = p + d * length - up * 0.009
                    ridge = mid + up * 0.009
                    m = tones[(j + k + limb + fork) % len(tones)]
                    batch.face(m, [p, mid - n, ridge])
                    batch.face(m, [mid - n, end, ridge])
                    batch.face(m, [ridge, end, mid + n])
                    batch.face(m, [ridge, mid + n, p])
    batch.finish("Courtyard citrus", collection)


def dress_table(meshes):
    # Each length of oak has its own photographed grain and joint. The short
    # edges use a lateral section through the growth rings of that board.
    source = bpy.data.materials.get("oiled-oak@ba9a79")
    for obj in meshes:
        mat = obj.data.materials[0]
        if not mat.name.startswith("dining-board-"):
            continue
        plank = int(mat.name.split("@")[0][-1])
        stain = mat.diffuse_color[:]
        mat = source.copy()
        mat.name = f"Dining tabletop · plank {plank + 1}"
        obj.data.materials[0] = mat
        tree = mat.node_tree
        bs = tree.nodes.get("Principled BSDF")
        base = bs.inputs["Base Color"].links[0].from_socket
        dye = tree.nodes.new("ShaderNodeMixRGB")
        dye.blend_type = "MULTIPLY"
        dye.inputs[0].default_value = 0.64
        dye.inputs[2].default_value = (0.54, 0.37, 0.24, 1)
        tree.links.new(base, dye.inputs[1])
        tree.links.new(dye.outputs[0], bs.inputs["Base Color"])
        bs.inputs["Coat Weight"].default_value = 0.15
        bs.inputs["Coat Roughness"].default_value = 0.36
        coords = [obj.matrix_world @ v.co for v in obj.data.vertices]
        low = [min(v[k] for v in coords) for k in range(3)]
        high = [max(v[k] for v in coords) for k in range(3)]
        uv = obj.data.uv_layers.get("Board surface") or obj.data.uv_layers.new(
            name="Board surface"
        )
        for poly in obj.data.polygons:
            for loop in poly.loop_indices:
                pt = coords[obj.data.loops[loop].vertex_index]
                u, v, h = [(q - a) / (b - a) for q, a, b in zip(pt, low, high)]
                row = (0, 3, 5, 7)[plank]
                section = (
                    (0.07 + 0.86 * v) if abs(poly.normal.z) > 0.64 else 0.10 + 0.12 * h
                )
                uv.data[loop].uv = (
                    0.025 + 0.95 * (u if plank % 2 else 1 - u),
                    1 - (row + section) / 8,
                )
        # End grain appears on the cut faces at either end of each individual plank.
        end = material(
            f"Dining tabletop · end grain {plank + 1}",
            tuple(0.78 * v for v in stain[:3]),
            0.55,
        )
        nd = end.node_tree.nodes
        links = end.node_tree.links
        coord = nd.new("ShaderNodeTexCoord")
        wave = nd.new("ShaderNodeTexWave")
        wave.wave_type = "RINGS"
        wave.rings_direction = "X"
        wave.inputs["Scale"].default_value = 18.0
        wave.inputs["Distortion"].default_value = 2.35
        wave.inputs["Detail Scale"].default_value = 1.4
        links.new(coord.outputs["Object"], wave.inputs[0])
        ramp = nd.new("ShaderNodeValToRGB")
        for el, amp in zip(ramp.color_ramp.elements, (0.57, 0.92)):
            el.color = tuple(c * amp for c in stain[:3]) + (1.0,)
        links.new(wave.outputs["Fac"], ramp.inputs[0])
        links.new(ramp.outputs[0], nd.get("Principled BSDF").inputs[0])
        obj.data.materials.append(end)
        for poly in obj.data.polygons:
            if abs(poly.normal.x) > 0.86:
                poly.material_index = 1


def dress_plaster(components, site):
    for key in (
        "ground-storey",
        "bedroom-storey",
        "workroom-storey",
        "rounded-stair-bay",
    ):
        for ob in components[key][1]:
            mat = ob.data.materials[0]
            if not mat.name.startswith("painted-stucco@"):
                continue
            mesh = ob.data
            attr = mesh.attributes.new("Rain exposure", "FLOAT", "POINT")
            for i, vertex in enumerate(mesh.vertices):
                p = site.inverted() @ (ob.matrix_world @ vertex.co)
                x, h, z = p.x, p.z, -p.y
                wear = 0.53 * max(0.0, 1.0 - h / 0.58)
                if z > 12.75 or z < 0.1:
                    for sill in (0.86, 4.193, 7.526):
                        drop = sill - h
                        if 0 < drop < 1.03:
                            run = min(abs(x - 0.70), abs(x - 4.60))
                            wear += (
                                0.74
                                * (1 - drop / 1.03)
                                * max(0.0, 1 - run / (0.13 + drop * 0.13))
                            )
                attr.data[i].value = min(1.0, wear)
    for mat in bpy.data.materials:
        if not mat.name.startswith("painted-stucco@"):
            continue
        nodes = mat.node_tree.nodes
        link = mat.node_tree.links.new
        bs = nodes.get("Principled BSDF")
        base = bs.inputs["Base Color"].links[0].from_socket
        attr = nodes.new("ShaderNodeAttribute")
        attr.attribute_name = "Rain exposure"
        geom = nodes.new("ShaderNodeNewGeometry")
        noise = nodes.new("ShaderNodeTexNoise")
        noise.inputs["Scale"].default_value = 5.5
        link(geom.outputs["Position"], noise.inputs[0])
        mask = nodes.new("ShaderNodeMath")
        mask.operation = "MULTIPLY"
        link(attr.outputs["Fac"], mask.inputs[0])
        link(noise.outputs[0], mask.inputs[1])
        mix = nodes.new("ShaderNodeMixRGB")
        mix.blend_type = "MULTIPLY"
        mix.inputs[2].default_value = (0.45, 0.35, 0.22, 1)
        link(mask.outputs[0], mix.inputs[0])
        link(base, mix.inputs[1])
        link(mix.outputs[0], bs.inputs[0])


def stage(scene, root, components, dimensions, planted):
    with gzip.open(root / "public/data/sf/sf-city.json.gz", "rt") as f:
        city = json.load(f)
    house = next(b for b in city["detailBuildings"] if b["addr"] == "21 Beaver Street")
    ring = [Vector(p) for p in zip(house["points"][::2], house["points"][1::2])]
    site, mapped, ground = place_shell(
        scene, ring, components, dimensions, planted, terrain_sampler(root)
    )
    site_model(scene, root, city, house, mapped, ground, site)
    scene.view_layers[0].update()
    dress_table(components["oak-dining-table"][1])
    dress_plaster(components, site)
    door = components["front-door"][0]
    door["Open angle"] = 0.0
    door.id_properties_ui("Open angle").update(
        min=0.0,
        max=95.0,
        soft_min=0.0,
        soft_max=95.0,
        description="Inward swing in degrees, 0 is fully closed",
    )
    driver = door.driver_add("rotation_euler", 2).driver
    var = driver.variables.new()
    var.name = "degrees_open"
    var.type = "SINGLE_PROP"
    var.targets[0].id = door
    var.targets[0].data_path = '["Open angle"]'
    driver.expression = "degrees_open * 0.0174532925199433"
    scene.unit_settings.system = "METRIC"
    scene.unit_settings.length_unit = "METERS"
    scene.unit_settings.scale_length = 1.0
    # Continue the rear stucco over its stem wall down to a low, exposed
    # concrete footing. The surveyed grade and all three sill heights stay fixed.
    base_material = next(
        m for m in bpy.data.materials if m.name.startswith("painted-stucco@")
    )
    coating = MeshBatch()
    for i in range(35):
        a, b = 7 * i / 35, 7 * (i + 1) / 35
        left, right = [site @ Vector((x, -13.052, 0)) for x in (a, b)]
        low_a, low_b = ground(left.xy) + 0.19, ground(right.xy) + 0.19
        coating.face(
            base_material,
            [
                (left.x, left.y, low_a),
                (right.x, right.y, low_b),
                (right.x, right.y, 0.015),
                (left.x, left.y, 0.015),
            ],
        )
    exterior = bpy.data.collections["Site · parcel and surrounding buildings"]
    coating.finish("Rear foundation stucco", exterior)
    weather = next(
        o for o in exterior.objects if o.name.startswith("Rear foundation stucco")
    )
    uv = weather.data.uv_layers.new(name="Photographic finish")
    attr = weather.data.attributes.new("Rain exposure", "FLOAT", "POINT")
    for v in weather.data.vertices:
        attr.data[v.index].value = min(
            0.94, max(0.12, 1.0 - (v.co.z - ground(v.co.xy)) / 0.77)
        )
    for l in weather.data.loops:
        p = weather.data.vertices[l.vertex_index].co
        uv.data[l.index].uv = (p.x / 1.8, p.z / 1.8)
    for mat in bpy.data.materials:
        if mat.name.startswith(
            ("cotton-coverlet@", "cotton-turnback@", "cotton-binding@")
        ):
            tree = mat.node_tree
            bs = tree.nodes.get("Principled BSDF")
            bs.inputs["Sheen Weight"].default_value = 0.20
            bs.inputs["Specular IOR Level"].default_value = 0.22
            rough = bs.inputs["Roughness"]
            if rough.is_linked:
                channel = rough.links[0].from_socket
                adjust = tree.nodes.new("ShaderNodeMath")
                adjust.operation = "MULTIPLY_ADD"
                adjust.inputs[1].default_value = 0.20
                adjust.inputs[2].default_value = 0.72
                tree.links.new(channel, adjust.inputs[0])
                tree.links.new(adjust.outputs[0], rough)
    for obj in list(scene.objects):
        if obj.type in ("LIGHT", "CAMERA"):
            bpy.data.objects.remove(obj, do_unlink=True)
    rig = bpy.data.collections.new("Photography · cameras and practical lights")
    scene.collection.children.link(rig)

    def position(x, h, z):
        return site @ Vector((x, -z, h))

    def camera(name, eye, target, lens):
        data = bpy.data.cameras.new(name)
        data.lens = lens
        data.clip_start = 0.025
        data.clip_end = 300.0
        ob = bpy.data.objects.new(name, data)
        rig.objects.link(ob)
        ob.location = position(*eye)
        ob.rotation_euler = (
            (position(*target) - ob.location).to_track_quat("-Z", "Y").to_euler()
        )
        return ob

    scene.camera = camera(
        "01 · Upper story and roof", (10.25, 14.0, -8.8), (3.48, 5.55, 2.1), 29
    )
    camera("02 · Rear courtyard", (1.10, 1.88, 24.6), (3.37, 4.00, 13.2), 28)
    dining = camera(
        "03 · Dining room", (5.014, 1.71, 2.50), (3.075, 0.247, 9.113), 32.63
    )
    dining.data.shift_y = 0.059
    camera("04 · Upstairs bedroom", (4.32, 4.93, 8.35), (2.30, 4.08, 11.77), 24)
    camera("05 · Front entry", (5.65, 1.62, -5.70), (5.70, 1.14, 0.13), 39)
    camera("06 · Kitchen at table height", (3.44, 1.36, 5.56), (1.37, 1.12, 9.56), 35)
    world = scene.world
    world.use_nodes = True
    sky = world.node_tree.nodes.new("ShaderNodeTexSky")
    sky.sun_elevation = math.radians(36)
    sky.sun_rotation = math.radians(241)
    sky.sun_size = math.radians(0.72)
    sky.sun_disc = False
    bg = world.node_tree.nodes.get("Background")
    world.node_tree.links.new(sky.outputs[0], bg.inputs[0])
    bg.inputs[1].default_value = 0.34
    sun = bpy.data.lights.new("Morning sun", "SUN")
    sun.energy = 3.0
    sun.angle = math.radians(1.25)
    sun.color = (1.0, 0.84, 0.68)
    ob = bpy.data.objects.new(sun.name, sun)
    rig.objects.link(ob)
    sunlight = ob

    # A thin, clear sash admits exterior daylight in shadow rays as in real single glazing.
    for mat in bpy.data.materials:
        if not mat.name.startswith("glass-door@"):
            continue
        tree = mat.node_tree
        bs = tree.nodes.get("Principled BSDF")
        bs.inputs["Roughness"].default_value = 0.022
        bs.inputs["IOR"].default_value = 1.49
        path = tree.nodes.new("ShaderNodeLightPath")
        transparent = tree.nodes.new("ShaderNodeBsdfTransparent")
        mix = tree.nodes.new("ShaderNodeMixShader")
        tree.links.new(path.outputs["Is Shadow Ray"], mix.inputs[0])
        tree.links.new(bs.outputs[0], mix.inputs[1])
        tree.links.new(transparent.outputs[0], mix.inputs[2])
        tree.links.new(
            mix.outputs[0], tree.nodes.get("Material Output").inputs["Surface"]
        )

    def bulb(name, xyz, watts, radius, rgb):
        light = bpy.data.lights.new(name, "POINT")
        light.energy = watts
        light.shadow_soft_size = radius
        light.color = rgb
        ob = bpy.data.objects.new(name, light)
        rig.objects.link(ob)
        ob.location = xyz

    bulb(
        "Dining pendant · opal globe",
        Vector((*components["dining-pendant"][0].location.xy, 1.96)),
        19,
        0.078,
        (1, 0.78, 0.56),
    )
    bulb("Entry sconce", position(5.97, 2.23, -0.26), 6, 0.053, (1, 0.75, 0.51))
    bulb(
        "Bedside reading light",
        components["bedside-lamp"][0].location + Vector((0, 0, 0.37)),
        5,
        0.065,
        (1, 0.77, 0.53),
    )
    scene.render.engine = "CYCLES"
    scene.cycles.samples = 144
    scene.cycles.use_denoising = True
    scene.cycles.preview_samples = 24
    scene.cycles.use_preview_denoising = True
    scene.cycles.adaptive_threshold = 0.022
    scene.cycles.max_bounces = 10
    scene.cycles.transmission_bounces = 8
    scene.view_settings.view_transform = "AgX"
    cameras = sorted(
        (o for o in rig.objects if o.type == "CAMERA"), key=lambda o: o.name
    )
    # Bookmarks also select the exposure used for each interior or exterior.
    scene.frame_start = 1
    scene.frame_end = len(cameras)
    for frame, (cam, ev) in enumerate(
        zip(cameras, (-0.30, -0.25, 2.80, 2.35, 0.30, 2.70)), 1
    ):
        incoming = (
            (-0.70, -0.65, -0.85),
            (0.065, 0.99, -0.68),
            (0.31, 0.90, -0.49),
            (0.31, 0.90, -0.49),
            (-0.55, -0.77, -0.56),
            (0.31, 0.90, -0.49),
        )[frame - 1]
        sunlight.rotation_euler = Vector(incoming).to_track_quat("-Z", "Y").to_euler()
        sunlight.keyframe_insert("rotation_euler", frame=frame)
        scene.timeline_markers.new(cam.name, frame=frame).camera = cam
        scene.view_settings.exposure = ev
        scene.view_settings.keyframe_insert("exposure", frame=frame)
    scene.frame_set(1)
    scene.render.resolution_x = 1536
    scene.render.resolution_y = 1024
    scene.render.resolution_percentage = 100
    scene["source_footprint"] = str(house["id"])
    scene["address"] = house["addr"]
    for screen in bpy.data.screens:
        for area in screen.areas:
            if area.type == "VIEW_3D":
                area.spaces.active.region_3d.view_perspective = "CAMERA"
                area.spaces.active.overlay.show_overlays = False
                area.spaces.active.shading.type = "MATERIAL"
                area.spaces.active.shading.use_scene_lights = True
                area.spaces.active.shading.use_scene_world = True
