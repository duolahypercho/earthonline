"""Fit solid Mission tiles to the editable 21 Beaver gable and round stair bay.

Run in Blender on the staged photo house. The output directory contains the
furnished scene, the roof assembly, three reusable pieces and their provenance.
"""

import argparse
import gzip
import hashlib
import json
import math
import random
import sys
from pathlib import Path

import bpy
import numpy as np
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
from cook_beaver_mission_roof import png, cook
from beaver_blender_scene import site_basis
from beaver_street_scene import load_surface

# MCA TF02. Length, wide/narrow widths and maximum exposure are physical metres.
PROFILE = "https://www.mca-tile.com/our-product/classic-tapered-mission/"
LENGTH, WIDE, NARROW = 0.4826, 0.1905, 0.127
EXPOSURE, SPACING = 0.4064, 0.254
HEADLAP = LENGTH - EXPOSURE
# Select the faces of individual tiles in the credited scan, inside the joints.
# Coordinates are in a 490-square preview of the original 2048-square image.
PATCHES = (
    (5, 79, 17, 98),
    (25, 67, 36, 94),
    (44, 13, 55, 35),
    (85, 45, 94, 65),
    (107, 63, 118, 84),
    (130, 61, 139, 76),
    (150, 43, 160, 73),
    (192, 63, 200, 80),
    (211, 29, 221, 45),
    (236, 49, 244, 67),
    (253, 12, 263, 28),
    (275, 43, 285, 58),
    (300, 52, 309, 76),
    (343, 75, 352, 90),
    (370, 13, 379, 35),
    (404, 29, 413, 54),
)
STEPS = 10
rng = random.Random(210621)
up = Vector((0.0, 0.0, 1.0))


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def clay_material(inputs, destination):
    pages = {}
    spec = json.loads(
        (
            ROOT
            / "public/assets/buildings/beaver-terrace/finishes/patina/furniture/photo/fired-earth.provenance.json"
        ).read_text()
    )
    mat = bpy.data.materials.new("Mission clay · weathered fired body")
    mat.use_nodes = True
    tree = mat.node_tree
    bs = tree.nodes.get("Principled BSDF")
    bs.inputs["Specular IOR Level"].default_value = 0.28
    for role in ("albedo", "normal", "arm"):
        item = next(i for i in spec["inputs"] if i["role"] == role)
        path = inputs / item["url"].split("/")[-1]
        if sha(path) != item["sha256"]:
            raise ValueError(f"Unexpected source scan {path}")
        im = bpy.data.images.load(str(path), check_existing=False)
        im.colorspace_settings.name = "Non-Color"
        w, h = im.size
        pixels = np.empty(w * h * 4, np.float32)
        im.pixels.foreach_get(pixels)
        pixels = pixels.reshape(h, w, 4)
        if role == "albedo":
            # Eight-bit source pixels are sRGB code values. Filter in linear light.
            swatch = pixels[::4, ::4, :3]
            chroma = np.ptp(swatch, axis=2) / np.maximum(swatch.max(2), 0.01)
            weathered = (chroma < np.quantile(chroma, 0.10)) & (
                swatch.mean(2) > np.quantile(swatch.mean(2), 0.65)
            )
            pale = np.median(swatch[weathered], axis=0)
            pale = np.where(
                pale <= 0.04045, pale / 12.92, ((pale + 0.055) / 1.055) ** 2.4
            )
            c = pixels[:, :, :3]
            pixels[:, :, :3] = np.where(
                c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4
            )
        side = 128
        atlas = np.ones((4 * side, 4 * side, 4), np.float32)
        for i, (x0, y0, x1, y1) in enumerate(PATCHES):
            # Keep a four-texel gutter by repeating the last usable texel.
            f = np.clip((np.arange(side) - 4) / (side - 9), 0, 1)
            x = (x0 + (x1 - x0) * f) / 490 * (w - 1)
            y = (1 - (y1 + (y0 - y1) * f) / 490) * (h - 1)
            ix = x.astype(int)
            iy = y.astype(int)
            fx = (x - ix)[None, :, None]
            fy = (y - iy)[:, None, None]
            rows = lambda yy: (
                pixels[yy[:, None], ix] * (1 - fx) + pixels[yy[:, None], ix + 1] * fx
            )
            tile = rows(iy) * (1 - fy) + rows(iy + 1) * fy
            if role == "normal":
                # The mesh supplies the barrel. Retain only the skin's local relief.
                n = tile[:, :, :3] * 2 - 1
                padded = np.pad(n[:, :, :2], ((7, 7), (7, 7), (0, 0)), mode="edge")
                sums = np.pad(padded, ((1, 0), (1, 0), (0, 0))).cumsum(0).cumsum(1)
                mean = (
                    sums[15:, 15:]
                    - sums[:-15, 15:]
                    - sums[15:, :-15]
                    + sums[:-15, :-15]
                ) / 225
                n[:, :, :2] -= mean
                n[:, :, 2] = np.sqrt(
                    np.maximum(0.01, 1 - (n[:, :, :2] ** 2).sum(axis=2))
                )
                tile[:, :, :3] = 0.5 + 0.5 * n
            if role == "arm":
                tile[:, :, 0] = 1
                tile[:, :, 2] = 0
            v, u = divmod(i, 4)
            atlas[v * side : (v + 1) * side, u * side : (u + 1) * side] = tile
        bpy.data.images.remove(im)
        # Pack standard PNGs that the glTF exporter and Cycles can both read.
        material_dir = destination / "maps"
        material_dir.mkdir(exist_ok=True)
        target = material_dir / (role + ".png")
        colors = atlas[:, :, :3]
        if role == "albedo":
            # One roof was laid from a firing blend. Keep within-tile weathering,
            # while limiting the variation in the mean tone from tile to tile.
            swatches = colors.reshape(4, side, 4, side, 3).transpose(0, 2, 1, 3, 4)
            means = np.median(swatches[:, :, 5:-5, 5:-5, :], axis=(2, 3))
            batch = np.median(means.reshape(-1, 3), axis=0)
            swatches *= (0.50 + 0.50 * batch / means)[:, :, None, None, :]
            # A little of the pale weathered material, sampled from the same
            # scan, settles more heavily near the lips of the exposed courses.
            settle = np.linspace(0.055, 0.15, side)[None, None, :, None, None]
            swatches[:] = swatches * (1 - settle) + pale * settle
            colors = np.where(
                colors <= 0.0031308,
                12.92 * colors,
                1.055 * np.maximum(0, colors) ** (1 / 2.4) - 0.055,
            )
        raw = np.uint8(np.clip(np.rint(colors[::-1] * 255), 0, 255))
        pages[role] = raw
        png(target, raw)
        image = bpy.data.images.load(str(target), check_existing=False)
        image.name = f"Mission clay · {role}"
        image.colorspace_settings.name = "sRGB" if role == "albedo" else "Non-Color"
        image.pack()
        node = tree.nodes.new("ShaderNodeTexImage")
        node.image = image
        if role == "normal":
            normal = tree.nodes.new("ShaderNodeNormalMap")
            normal.inputs["Strength"].default_value = 0.48
            tree.links.new(node.outputs["Color"], normal.inputs["Color"])
            tree.links.new(normal.outputs["Normal"], bs.inputs["Normal"])
        elif role == "arm":
            split = tree.nodes.new("ShaderNodeSeparateColor")
            tree.links.new(node.outputs["Color"], split.inputs[0])
            tree.links.new(split.outputs["Green"], bs.inputs["Roughness"])
            tree.links.new(split.outputs["Blue"], bs.inputs["Metallic"])
        else:
            tree.links.new(node.outputs["Color"], bs.inputs["Base Color"])
    return mat, spec, pages


class Batch:
    def __init__(self):
        self.verts = []
        self.faces = []
        self.uv = []
        self.smooth = []
        self.pieces = 0

    def tile(self, cell, outside, inside):
        # Outer and inner skins share all four rims to make one watertight piece.
        first = len(self.verts)
        sections = 3
        stride = STEPS + 1
        count = (sections + 1) * stride
        self.pieces += 1
        for skin in (outside, inside):
            for j in range(sections + 1):
                for k in range(stride):
                    t = j / sections
                    u = k / STEPS
                    self.verts.append(tuple(skin(t, u)))
                    self.uv.append(
                        (
                            (cell % 4 + (4.5 + 119 * u) / 128) / 4,
                            (cell // 4 + (4.5 + 119 * t) / 128) / 4,
                        )
                    )

        def face(ids, smooth=False):
            self.faces.append(tuple(first + i for i in ids))
            self.smooth.append(smooth)

        for j in range(sections):
            for k in range(STEPS):
                i = j * stride + k
                front = (i, i + stride, i + stride + 1, i + 1)
                face(front, True)
                face(tuple(a + count for a in reversed(front)), True)
        rim = list(range(stride)) + [j * stride + STEPS for j in range(1, sections + 1)]
        rim += list(range(count - 2, sections * stride - 1, -1)) + [
            j * stride for j in range(sections - 1, 0, -1)
        ]
        for a, b in zip(rim, rim[1:] + rim[:1]):
            face((a, b, b + count, a + count))

    def object(self, name, collection):
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata(self.verts, [], self.faces)
        mesh.update()
        obj = bpy.data.objects.new(name, mesh)
        collection.objects.link(obj)
        mesh.materials.append(clay)
        flags = np.asarray(self.smooth, dtype=bool)
        mesh.polygons.foreach_set("use_smooth", flags)
        uv = mesh.uv_layers.new(name="Fired clay faces")
        idx = np.empty(len(mesh.loops), np.int32)
        mesh.loops.foreach_get("vertex_index", idx)
        uv.data.foreach_set("uv", np.asarray(self.uv, dtype=np.float32)[idx].ravel())
        obj["individual_tiles"] = self.pieces
        return obj


def kiln_face(pan):
    # Dirt and stains collect in the channels between rain-washed covers.
    if pan or rng.random() < 0.13:
        return rng.randrange(16)
    return rng.choice((1, 2, 3, 4, 9, 11, 14, 15))


def shell(batch, kind, cell, point, length=LENGTH, wide=WIDE, narrow=NARROW):
    pan = kind == "Pan"
    ridge = kind == "Ridge"
    thickness = 0.013
    warp = rng.uniform(-0.0018, 0.0018)

    def skin(outer):
        def vertex(t, u):
            # Pans widen uphill to receive the next pan. Covers widen downhill
            # so the broad mouth clears the narrower crown of the lower course.
            mix = t if pan else 1 - t
            radius = (narrow + (wide - narrow) * mix) / 2
            theta = math.pi * u
            r = (
                radius - thickness
                if (pan and outer) or (not pan and not outer)
                else radius
            )
            y = -math.cos(theta) * r
            crest = math.sin(theta) * r
            if pan:
                h = radius - crest + 0.023 * (1 - t)
            elif ridge:
                h = 0.018 + crest + 0.019 * (1 - t)
            else:
                # The underside of the cover rests on the adjacent pan lips.
                pan_r = (narrow + (wide - narrow) * t) / 2
                contact_x = SPACING / 2 - pan_r
                seat = max(
                    0.0,
                    pan_r
                    - math.sqrt(max(0.0, (radius - thickness) ** 2 - contact_x**2)),
                )
                h = seat + 0.006 + crest + 0.023 * (1 - t)
            return point(t * length, y, h + warp * math.sin(theta * 2 + t * 2.5))

        return vertex

    batch.tile(cell, skin(True), skin(False))


def ply(x, z, y):
    return site @ Vector((x, -z, y))


def fit_gable():
    ridge, top, front, rear = 2.87, 10.98, -0.61, 13.22
    dp = (ply(0, rear, 0) - ply(0, front, 0)).length
    lanes = math.ceil(dp / SPACING)
    span = (rear - front) / lanes
    result = {}
    for x0, h0 in ((-0.15, 10.02), (7.15, 10.04)):
        upslope = ply(ridge, 0, top) - ply(x0, 0, h0)
        reach = upslope.length
        upslope.normalize()
        depth = (ply(0, rear, 0) - ply(0, front, 0)).normalized()
        normal = upslope.cross(depth).normalized()
        if normal.z < 0:
            normal.negate()
        cross = normal.cross(upslope)
        courses = math.ceil(reach / EXPOSURE)
        for j in range(courses):
            toward = reach * j / courses
            start_x = x0 + (ridge - x0) * j / courses
            setback = 0.67 * min(1, max(0, (start_x - 4.88) / 0.35))
            for row in range(lanes + 1):
                for kind in ("Pan", "Cover"):
                    if kind == "Pan" and row == lanes:
                        continue
                    at = front + (row + (0.5 if kind == "Pan" else 0)) * span
                    if at < front + setback - 0.065:
                        continue
                    origin = ply(x0, at, h0) + toward * upslope

                    def world(a, b, h):
                        return origin + upslope * a + cross * b + normal * h

                    # Reverse neither the surface normal nor the clay when the
                    # two roof slopes meet. 'cross' follows the local right hand frame.
                    shell(
                        result.setdefault(kind, Batch()),
                        kind,
                        kiln_face(kind == "Pan"),
                        world,
                        length=min(LENGTH, reach - toward + 0.015),
                    )
    direction = (ply(ridge, rear, top) - ply(ridge, front, top)).normalized()
    side = up.cross(direction)
    count = math.ceil(dp / EXPOSURE)
    for k in range(count):
        origin = ply(ridge, front, top) + direction * (dp * k / count) + up * 0.057
        shell(
            result.setdefault("Ridge", Batch()),
            "Ridge",
            kiln_face(False),
            lambda a, b, h: origin + direction * a + side * b + up * h,
            length=min(LENGTH, dp - dp * k / count + 0.02),
            wide=0.278,
            narrow=0.218,
        )
    return result


def fit_turret():
    # The circular bay is slightly elliptical in the surveyed parcel frame.
    # Each ring is a course of purpose-cut radial pans and covers.
    result = {k: Batch() for k in ("Radial pan", "Radial cover")}
    for outer, inner, n in (
        (1.10, 0.744, 32),
        (0.805, 0.478, 32),
        (0.541, 0.23, 32),
        (0.293, 0.188, 32),
    ):
        pitch = 2 * math.pi / n
        for lane in range(n):
            for pan in (True, False):
                mid = (lane + (0.5 if pan else 0)) * pitch

                def layer(top):
                    def at(t, u):
                        r = outer + (inner - outer) * t
                        theta = math.pi * u
                        width = (0.94 if pan else 0.74) * (
                            1 - 0.10 * t if not pan else 1
                        )
                        angle = mid - 0.5 * math.cos(theta) * pitch * width
                        radius = 1.23 * r * pitch * width * 0.5
                        crest = (
                            radius * (1 - math.sin(theta))
                            if pan
                            else 0.019 + radius * math.sin(theta)
                        )
                        h = 10.21 + (1.09 - r) * 0.68 + crest + 0.021 * (1 - t)
                        if not top:
                            h -= 0.012
                        return ply(
                            5.955 + r * math.sin(angle),
                            0.04 - 0.90 * r * math.cos(angle),
                            h,
                        )

                    return at

                kind = "Radial pan" if pan else "Radial cover"
                result[kind].tile(kiln_face(pan), layer(True), layer(False))
    return result


def end_fittings(roof, bay):
    """Close the ridge mouths and the point where all turret cuts terminate."""

    def fitting(name, vertices, faces, smooth, collection):
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata(vertices, [], faces)
        mesh.update()
        mesh.materials.append(clay)
        uv = mesh.uv_layers.new(name="Fired clay faces")
        bounds = np.ptp(np.asarray(vertices), axis=0)
        axes = np.argsort(bounds)[-2:]
        point = np.asarray(vertices)[:, axes]
        tex = (point - point.min(0)) / bounds[axes] * 0.21 + 0.02
        idx = np.empty(len(mesh.loops), np.int32)
        mesh.loops.foreach_get("vertex_index", idx)
        uv.data.foreach_set("uv", tex[idx].astype(np.float32).ravel())
        for polygon in mesh.polygons:
            polygon.use_smooth = smooth
        obj = bpy.data.objects.new(name, mesh)
        collection.objects.link(obj)
        obj.parent = bpy.data.objects[collection.name]
        obj.matrix_parent_inverse = obj.parent.matrix_world.inverted()
        obj["individual_tiles"] = 1
        return obj

    out = []
    front = ply(2.87, -0.61, 10.98)
    rear = ply(2.87, 13.22, 10.98)
    axis = (rear - front).normalized()
    for start, center, radius, name in (
        (True, front, 0.139, "Ridge starter"),
        (False, rear + axis * 0.02, 0.109, "Ridge terminal"),
    ):
        outward = -axis if start else axis
        side = up.cross(outward)
        origin = center + (0.057 + 0.018 + (0.019 if start else 0.0)) * up
        pts = []
        faces = []
        for d, r in (
            (-0.015, radius - 0.003),
            (0.002, radius),
            (0.007, radius - 0.005),
        ):
            for i in range(17):
                theta = math.pi * i / 16
                pts.append(
                    origin
                    + d * outward
                    + r * (math.cos(theta) * side + math.sin(theta) * up)
                )
        for level in range(2):
            for i in range(17):
                k = level * 17 + i
                n = level * 17 + (i + 1) % 17
                faces.append((k, n, n + 17, k + 17))
        faces.extend([tuple(reversed(range(17))), tuple(range(34, 51))])
        out.append(fitting(name, pts, faces, False, roof))
    center = ply(5.955, 0.04, 0)
    # A hollow fired terminal skirts the cut tips. The skirt and inner return
    # give the silhouette a real clay wall instead of converging to an open pinhole.
    profile = [
        (0.001, 11.082),
        (0.014, 11.070),
        (0.031, 11.048),
        (0.065, 11.024),
        (0.123, 10.985),
        (0.225, 10.916),
        (0.313, 10.854),
        (0.320, 10.840),
        (0.312, 10.828),
        (0.296, 10.833),
        (0.213, 10.894),
        (0.105, 10.973),
        (0.031, 11.027),
        (0.002, 11.047),
    ]
    pts = []
    faces = []
    segments = 96
    for j, (radius, h) in enumerate(profile):
        for i in range(segments):
            a = i * 2 * math.pi / segments
            flute = 0.025 * radius * (1 - math.cos(16 * a)) * 0.5
            pts.append(
                center + Vector((radius * math.cos(a), radius * math.sin(a), h + flute))
            )
            if j:
                k = (j - 1) * segments + i
                n = (j - 1) * segments + (i + 1) % segments
                faces.append((n, k, k + segments, n + segments))
    faces.extend(
        [tuple(range(segments)), tuple(reversed(range(len(pts) - segments, len(pts))))]
    )
    out.append(fitting("Turret crown", pts, faces, True, bay))
    return out


def export(parts, path):
    bpy.ops.object.select_all(action="DESELECT")
    for obj in parts:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = parts[0]
    bpy.ops.export_scene.gltf(
        filepath=str(path),
        export_format="GLB",
        use_selection=True,
        export_animations=False,
        export_cameras=False,
        export_lights=False,
        export_extras=True,
        export_tangents=True,
    )


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--runtime",
        action="store_true",
        help="Publish the native roof and scan to the project through the deterministic cook.",
    )
    p.add_argument("output", type=Path)
    p.add_argument("source_scans", type=Path)
    args = p.parse_args(sys.argv[sys.argv.index("--") + 1 :])
    args.output = args.output.resolve()
    if args.output.is_relative_to(ROOT):
        raise ValueError("Keep source assets outside the repository.")
    args.output.mkdir(parents=True, exist_ok=True)
    source = Path(bpy.data.filepath)
    src_sha = sha(source)
    target = args.output / "21-beaver.blend"
    if target == source:
        raise ValueError("Use a new directory for the fitted scene.")
    scene = bpy.context.scene
    scene.frame_set(1)
    roof = bpy.data.collections["terrace-roof"]
    bay = bpy.data.collections["rounded-stair-bay"]
    replace = [
        o
        for c in (roof, bay)
        for o in c.all_objects
        if o.type == "MESH" and o.active_material.name.startswith("fired-clay@")
    ]
    if len(replace) != 12:
        raise ValueError(
            f"Expected twelve kiln batches on the two roofs, got {len(replace)}"
        )
    with gzip.open(ROOT / "public/data/sf/sf-city.json.gz", "rt") as db:
        data = json.load(db)
    house = next(b for b in data["detailBuildings"] if b["addr"] == "21 Beaver Street")
    ring = [Vector(p) for p in zip(house["points"][::2], house["points"][1::2])]
    plan_scale = load_surface(ROOT, house["centroid"])["plan_scale"]
    site = site_basis(ring, plan_scale)[3]
    clay, spec, pages = clay_material(args.source_scans, args.output)
    for obj in replace:
        bpy.data.objects.remove(obj, do_unlink=True)
    fitted = []
    batches = []
    for batchset, collection in ((fit_gable(), roof), (fit_turret(), bay)):
        for key, batch in batchset.items():
            obj = batch.object("Mission roof · " + key, collection)
            # Keep the architectural root editable as one component.
            parent = bpy.data.objects[collection.name]
            obj.parent = parent
            obj.matrix_parent_inverse = parent.matrix_world.inverted()
            fitted.append(obj)
            batches.append(batch)
    fittings = end_fittings(roof, bay)
    fitted.extend(fittings)
    bpy.context.view_layer.update()
    export(fitted, args.output / "fitted-roof.glb")
    masters = bpy.data.collections.new("Mission clay tile library")
    scene.collection.children.link(masters)
    for i, kind in enumerate(("Pan", "Cover", "Ridge")):
        batch = Batch()
        shell(
            batch,
            kind,
            3 + i,
            lambda a, b, h: Vector((a, b, h)),
            wide=0.278 if kind == "Ridge" else WIDE,
            narrow=0.218 if kind == "Ridge" else NARROW,
        )
        part = batch.object(kind, masters)
        part.asset_mark()
        export([part], args.output / (kind.lower() + ".glb"))
    bpy.context.preferences.filepaths.save_version = 0
    bpy.data.libraries.write(
        str(args.output / "Mission tiles.blend"),
        {masters},
        fake_user=True,
        compress=True,
    )
    scene.collection.children.unlink(masters)
    for o in scene.objects:
        if o.select_get():
            o.select_set(False)
    bpy.context.view_layer.objects.active = None
    bpy.ops.wm.save_as_mainfile(filepath=str(target), compress=True)
    meta = {
        "base_scene": {"file": source.name, "sha256": src_sha},
        "profile_source": PROFILE,
        "profile_assumption": "Reference dimensions for the new modeled clay. The property tile manufacturer is unknown.",
        "dimensions_m": {
            "length": LENGTH,
            "wide_end": WIDE,
            "narrow_end": NARROW,
            "max_exposure": EXPOSURE,
            "max_spacing": SPACING,
            "nominal_headlap": HEADLAP,
        },
        "pieces": {o.name: o["individual_tiles"] for o in fitted},
        "inputs": spec["inputs"],
        "geometry_license": "MIT",
        "texture_license": spec["license"],
        "base_scene_notices": "See THIRD_PARTY_NOTICES.md in the Earth Online repository for the furnished scene and its other packed assets.",
        "attribution": spec["attribution"],
        "files": {
            str(f.relative_to(args.output)): {
                "sha256": sha(f),
                "bytes": f.stat().st_size,
            }
            for f in sorted(args.output.rglob("*"))
            if f.suffix in (".glb", ".blend", ".png")
        },
    }
    (args.output / "provenance.json").write_text(json.dumps(meta, indent=2) + "\n")

    if args.runtime:
        cook(batches, fittings, site, pages, ROOT / "public", meta)
