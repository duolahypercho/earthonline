"""Bake diffuse window and fixture transfer into the terrace's two light coordinates."""

import math
from mathutils import Vector
from mathutils.bvhtree import BVHTree
from mathutils.kdtree import KDTree


def subdivide_quad(points, face, spacing):
    """Generate a bilinear patch with regular lighting samples on large flat faces."""
    a, b, c, d = [Vector(points[i]) for i in face]
    nu = max(1, math.ceil(max((b - a).length, (c - d).length) / spacing))
    nv = max(1, math.ceil(max((d - a).length, (c - b).length) / spacing))
    if nu * nv == 1:
        return None
    verts = [tuple(a.lerp(d, j / nv).lerp(b.lerp(c, j / nv), i / nu)) for j in range(nv + 1) for i in range(nu + 1)]
    quads = []
    for j in range(nv):
        for i in range(nu):
            k = j * (nu + 1) + i
            quads.append((k, k + 1, k + nu + 2, k + nu + 1))
    return verts, quads


def filter_cloth_irradiance(mesh, uv, compact=False):
    """Filter Monte Carlo noise along the draped surface, in linear light."""
    # The export mesh has a regular, dense grid and shared smooth vertices.
    # Smoothing only on the tangent sheet keeps the folded hem separate.
    tree = KDTree(len(mesh.vertices))
    for vertex in mesh.vertices:
        tree.insert(vertex.co, vertex.index)
    tree.balance()
    energy = [None] * len(mesh.vertices)
    for loop in mesh.loops:
        daylight, fixture_v = uv.data[loop.index].uv
        energy[loop.vertex_index] = (daylight ** 2, (1.0 - fixture_v) ** 2)

    result = []
    for vertex in mesh.vertices:
        day = lamp = total = 0.0
        nearby = tree.find_n(vertex.co, min(72, len(mesh.vertices))) if compact else tree.find_range(vertex.co, 0.23)
        for point, index, distance in nearby:
            agreement = max(0.0, vertex.normal.dot(mesh.vertices[index].normal))
            separation = abs((point - vertex.co).dot(vertex.normal))
            weight = agreement ** 4 * math.exp(-0.5 * ((distance / 0.085) ** 2 + (separation / 0.025) ** 2))
            day += weight * energy[index][0]
            lamp += weight * energy[index][1]
            total += weight
        if total > 0:
            result.append((math.sqrt(day / total), 1.0 - math.sqrt(lamp / total)))
        else:
            result.append((math.sqrt(energy[vertex.index][0]), 1.0 - math.sqrt(energy[vertex.index][1])))
    for loop in mesh.loops:
        uv.data[loop.index].uv = result[loop.vertex_index]


def contact_transfer(mesh, tree):
    """Cosine-weighted visibility out to 45 cm, baked on the assembled furniture."""
    layer = mesh.uv_layers.new(name="Near-field visibility")
    samples = []
    for i in range(48):
        r = math.sqrt((i + 0.5) / 48)
        a = i * 2.39996323
        samples.append((r * math.cos(a), r * math.sin(a), math.sqrt(1 - r * r)))
    cache = {}
    for face in mesh.polygons:
        for index in face.loop_indices:
            loop = mesh.loops[index]
            n = mesh.corner_normals[index].vector
            key = (loop.vertex_index, *(round(v * 80) for v in n))
            if key not in cache:
                p = mesh.vertices[loop.vertex_index].co + n * 0.0025
                tangent = n.cross(
                    Vector((1, 0, 0)) if abs(n.z) > 0.85 else Vector((0, 0, 1))
                ).normalized()
                bitangent = n.cross(tangent)
                blocked = 0.0
                for u, v, w in samples:
                    hit, _, _, distance = tree.ray_cast(p, tangent * u + bitangent * v + n * w, 0.45)
                    if hit is not None:
                        blocked += (1 - distance / 0.45) ** 2
                cache[key] = 1 - blocked / len(samples)
            layer.data[index].uv = (cache[key], 0.5)


def bake(objects, rise, detailed=False, cloth_transfer=False, furniture_contact=False,
         porch_source=(5.97, 2.27, -0.26), entry_setback=0, exterior_bounce=False):
    vertices, faces = [], []
    for obj in objects:
        mesh = obj.data
        if mesh.materials[0].name.startswith("glass"):
            continue
        offset = len(vertices)
        vertices.extend(v.co.copy() for v in mesh.vertices)
        faces.extend([offset + v for v in f.vertices] for f in mesh.polygons)
    tree = BVHTree.FromPolygons(vertices, faces)
    # Stratified samples over each sash. The pendant samples sit inside its lower rim.
    windows = [
        [
            Vector((0.73 + (i + 0.5) * 3.82 / 7, -depth, level * rise + 0.94 + (j + 0.5) * 1.53 / 4))
            for depth in (-0.12, 13.14)
            for j in range(4)
            for i in range(7)
        ]
        for level in range(3)
    ]
    # The installed pendant moves with its dining group. Convert its 70 cm
    # move from physical metres to the surveyed depth used by this bake.
    fixture_z = 7.51 - .70 / 1.295 if exterior_bounce else 7.51
    lamp = [Vector((2.37 + dx, -fixture_z + dz, 1.975)) for dx, dz in [(-0.12, -0.09), (0.13, -0.09), (0, 0.13)]]
    work_lights = [Vector((0.64,-z,1.80)) for z in (9.55,10.10)]
    x, height, depth = porch_source
    porch_light = Vector((x, -depth, height))
    bounce_cache = {}
    direct_cache = {}

    exterior_cache = {}

    def reflected_daylight(point, normal):
        """Diffuse flux reflected by the ground and adjoining sunlit masonry.

        Sky visibility is already evaluated by the runtime. Only directions
        intercepted by an exterior surface contribute to this bounce.
        """
        if not exterior_bounce:
            return 0.
        key = tuple(round(v * 6) for v in point) + tuple(round(v * 10) for v in normal)
        if key in exterior_cache:
            return exterior_cache[key]
        tangent = normal.cross(Vector((0, 0, 1)) if abs(normal.z) < .85 else Vector((1, 0, 0))).normalized()
        bitangent = normal.cross(tangent)
        flux = 0.
        for i in range(32):
            r = math.sqrt((i + .5) / 32)
            angle = i * 2.39996323
            direction = tangent * (r * math.cos(angle)) + bitangent * (r * math.sin(angle)) + normal * math.sqrt(1-r*r)
            hit, hn, _, _ = tree.ray_cast(point,direction,18)
            if hit is None:
                if direction.z < -.04:
                    flux += .36
            else:
                if hn.dot(direction) > 0:
                    hn.negate()
                # Test the sky above the return so the recess retains its shadow.
                up = Vector((0,0,1))
                if tree.ray_cast(hit+hn*.008,up,16)[0] is None:
                    flux += .42 * (.37+.63*max(hn.z,0.))
        # An occluded return receives diffuse light from the adjoining walls.
        # Retain this higher-order component, especially around narrow reveals.
        result = math.sqrt(.022 + .22*flux / 32)
        exterior_cache[key] = result
        return result

    def porch_daylight(p, n):
        if not exterior_bounce:
            return 0.
        # Integrate the portal, rather than treating a moulding as an opaque
        # screen against the entire sky. This stays continuous over the arch.
        rect = 0.
        for ix in range(7):
            x = 5.16+(ix+.5)*1.59/7
            cap = 1.92+.72*math.sqrt(max(0.,1-((x-5.955)/.795)**2))
            for iz in range(7):
                height = (iz+.5)*cap/7
                delta = Vector((x,.88,height))-p
                d2 = max(.04,delta.length_squared)
                direction = delta.normalized()
                rect += cap*max(n.dot(direction),0.)*max(direction.y,0.)/d2
        projected = min(1., rect * 1.59 / (49*math.pi))
        return math.sqrt(.018+.073*projected)

    def visible_energy(p, n, sources, window):
        energy = 0.0
        for q in sources:
            delta = q - p
            d2 = delta.length_squared
            if d2 < 1e-6:
                continue
            direction = delta / math.sqrt(d2)
            cos_light = abs(direction.y) if window else max(direction.z, 0.0)
            projected = max(0.0, n.dot(direction)) * cos_light
            if projected > 0.002 and tree.ray_cast(p, direction, math.sqrt(d2) - 0.001)[0] is None:
                energy += projected / (d2 + (0.067 if window else 0.04))
        return energy

    def direct(p, n, level):
        # Finite areas and Lambert's cosine integral. Lamp intensity is calibrated
        # separately from daylight so its local pool is retained after sunset.
        day = visible_energy(p, n, windows[level], True) * (3.82 * 1.53 / 28) / math.pi
        lum = visible_energy(p, n, lamp, False) * 0.8 / len(lamp) if level == 0 else 0.0
        if detailed and level == 0:
            lum += visible_energy(p,n,work_lights,False) * 0.17 / len(work_lights)
        return day, lum

    def bounce(p, n, level, fine):
        # Subdivide the irradiance cache where the blanket can turn within a
        # single coarse cell. This is an offline cost only.
        spacing = 0.07 if fine else 0.24
        precision = 12 if fine else 4
        key = (level, fine, *(round(v / spacing) for v in p), *(round(v * precision) for v in n))
        if key in bounce_cache:
            return bounce_cache[key]
        tangent = n.cross(Vector((0, 0, 1)) if abs(n.z) < 0.85 else Vector((1, 0, 0))).normalized()
        bitangent = n.cross(tangent)
        day, lum = 0.0, 0.0
        samples = 40 if detailed else 16
        for i in range(samples):
            r = math.sqrt((i + 0.5) / samples)
            phi = i * 2.399963
            direction = tangent * (r * math.cos(phi)) + bitangent * (r * math.sin(phi)) + n * math.sqrt(1 - r * r)
            hit, hn, _, _ = tree.ray_cast(p, direction, 15)
            if hit is not None:
                if direction.dot(hn) > 0:
                    hn.negate()
                a, b = direct(hit + hn * 0.003, hn, level)
                day += a
                lum += b
        bounce_cache[key] = (day * 0.42 / samples, lum * 0.42 / samples)
        return bounce_cache[key]

    for obj in objects:
        mesh = obj.data
        uv = mesh.uv_layers.new(name="Diffuse light transfer")
        component = obj.name.partition(" - ")[0]
        if component == "frontage-broadleaf":
            # Outdoor leaves are lit by the live sun and sky. Keep their
            # geometry in the BVH so it shades the entrance and its glazing.
            uv.data.foreach_set("uv", [0.0, 1.0] * len(mesh.loops))
            continue
        sofa_cloth = furniture_contact and component in ("upholstered-sofa", "woven-lap-throw") and mesh.materials[0].name.partition("@")[0] in ("canvas-plain", "flax-linen")
        fine = sofa_cloth or cloth_transfer and mesh.materials[0].name.partition("@")[0] in ("cotton-coverlet", "cotton-turnback")
        # Compute per corner to preserve the two different normals at a hard edge.
        for face in mesh.polygons:
            for idx in face.loop_indices:
                loop = mesh.loops[idx]
                p = mesh.vertices[loop.vertex_index].co
                n = mesh.corner_normals[idx].vector
                point = p + n * 0.004
                # The entry niche is outside even though it lies inside the footprint.
                in_porch = (entry_setback > 0 and 5.13 < point.x < 6.78 and 0 <= point.z < 2.72
                            and -(entry_setback + .16) < point.y < .89
                            and (point.y > -(entry_setback + .04) or n.y > .35))
                if in_porch:
                    # Exterior daylight already comes from the runtime sky. Bake only
                    # the sconce here so the arch and adjoining facade share that sky.
                    delta = porch_light - point
                    dist = delta.length
                    lum = 0.
                    if dist > .09 and tree.ray_cast(point, delta.normalized(), dist-.09)[0] is None:
                        lum = min(.42, .070 * max(0., n.dot(delta.normalized())) / (dist*dist+.09))
                    uv.data[idx].uv = (porch_daylight(point, n), 1-math.sqrt(lum))
                    continue
                if not (0.06 < point.x < 6.94 and -12.96 < point.y < -0.04 and 0 <= point.z < 3 * rise):
                    if detailed and 4.90 < point.x < 7.0 and 0.025 < point.y < 0.89 and 0 < point.z < 2.94:
                        delta = porch_light - point
                        d2 = delta.length_squared
                        # Diffuse light from the opal globe on the inward-facing
                        # plaster. Stop the visibility ray at the globe's surface.
                        if d2 > 0.012 and tree.ray_cast(point,delta.normalized(),max(0,math.sqrt(d2)-0.094))[0] is None:
                            lamp_transfer = min(0.60,0.115*max(0,n.dot(delta.normalized()))/(d2+0.09))
                            uv.data[idx].uv = (reflected_daylight(point, n),1-math.sqrt(lamp_transfer))
                            continue
                    uv.data[idx].uv = (reflected_daylight(point, n), 1.0)
                    continue
                level = min(2, int(point.z / rise))
                key = (*(round(v * 300) for v in p), *(round(v * 80) for v in n))
                if key not in direct_cache:
                    day, lum = direct(point, n, level)
                    bd, bl = bounce(point, n, level, fine)
                    direct_cache[key] = (math.sqrt(min(1.0, day + bd)), 1.0 - math.sqrt(min(1.0, lum + bl)))
                # Blender's glTF exporter flips V.
                uv.data[idx].uv = direct_cache[key]
        if fine:
            filter_cloth_irradiance(mesh, uv, compact=sofa_cloth)
        outdoor_contact = exterior_bounce and component in (
            "front-door", "arched-entry-and-lantern", "upper-arched-loggia",
            "rear-clay-planter", "rear-herb-pot", "double-bed", "bedroom-curtains",
        )
        if outdoor_contact or furniture_contact and (component.startswith("dining-chair") or component in (
            "upholstered-sofa", "woven-lap-throw", "sitting-room-coffee-cup", "sitting-room-rug",
            "coffee-table", "clothbound-book", "oak-sideboard", "desk-chair", "workroom-chair",
        )):
            contact_transfer(mesh, tree)


def dress_floor_for_blender(scene, path, boards):
    """Use the same photographed boards in the editable scene as in the game."""
    import bpy

    if not path.is_file():
        raise FileNotFoundError(f"Build the photographic floor maps first: {path}")
    name = "Board surface"
    paint = {material_name: i for i, material_name in enumerate(boards)}
    for obj in scene.objects:
        if obj.type != "MESH" or not obj.data.materials or obj.data.materials[0].name not in paint:
            continue
        row_paint = paint[obj.data.materials[0].name]
        mesh = obj.data
        uv = mesh.uv_layers.get(name) or mesh.uv_layers.new(name=name)
        coords = []
        for vertex in mesh.vertices:
            p = obj.matrix_world @ vertex.co
            section = 5.1 if p.z > 1 and p.x >= 5.0999 else 0
            board = max(0, (p.x-section)/0.18 + 0.0001)
            row = (int(board)*5 + row_paint*3 + round(p.z/(10/3))) % 8
            v = (row + 0.07 + 0.86*(board % 1)) / 8
            u = (-p.y) * 0.5 * (-1 if row % 2 else 1) + (row*0.618034) % 1
            coords.append((u,1-v))
        for loop in mesh.loops:
            uv.data[loop.index].uv = coords[loop.vertex_index]
    color = bpy.data.images.load(str(path), check_existing=False)
    data = bpy.data.images.load(str(path), check_existing=False)
    data.colorspace_settings.name = "Non-Color"
    for image in (color,data):
        image.pack()
    for mat_name in boards:
        tree = bpy.data.materials[mat_name].node_tree
        shader = next(n for n in tree.nodes if n.type == 'BSDF_PRINCIPLED')
        uv = tree.nodes.new("ShaderNodeUVMap")
        uv.uv_map = name
        for page, slot in enumerate(("Base Color", "Normal", "Roughness")):
            coord = tree.nodes.new("ShaderNodeVectorMath")
            coord.operation = "MULTIPLY_ADD"
            coord.inputs[1].default_value = (1,1/3,1)
            coord.inputs[2].default_value = (0,(2-page)/3,0)
            tree.links.new(uv.outputs["UV"],coord.inputs[0])
            tex = tree.nodes.new("ShaderNodeTexImage")
            tex.image = color if page == 0 else data
            tree.links.new(coord.outputs["Vector"],tex.inputs["Vector"])
            result = tex.outputs["Color"]
            if slot == "Normal":
                normal = tree.nodes.new("ShaderNodeNormalMap")
                normal.uv_map = name
                tree.links.new(result,normal.inputs["Color"])
                result = normal.outputs["Normal"]
            elif slot == "Roughness":
                channels = tree.nodes.new("ShaderNodeSeparateColor")
                tree.links.new(result,channels.inputs["Color"])
                result = channels.outputs["Green"]
            tree.links.new(result,shader.inputs[slot])
