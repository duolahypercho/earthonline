"""Cut the split line of a two-tone body along a smooth curve.

A two-tone livery authored by assigning a second paint material to whole faces draws
its split as a staircase that follows the quad grid (one quad per step), which reads
as a pixel staircase on a clearcoat. This cuts the paint faces along a smooth curve
and reassigns the two paints by side, so the colour edge is a real, straight-segmented
edge on the surface instead of the grid.

The curve is the 0.5 level set of a label field smoothed in WORLD space: every paint
face votes 0, every accent face 1, weighted by area and a Gaussian of distance. The
field ignores topology, so a door skin and the body beside it (separate meshes across
a panel gap) are cut on the same curve. New vertices lie on existing edges, so the
surface shape does not change. The cut adds faces; the caller pays for them within its
tier budget (see `dissolve_flat_hidden`).
"""
import math

import bmesh
import bpy
from mathutils.kdtree import KDTree


def _objects_with(paint, accent):
    found = []
    for obj in bpy.context.scene.objects:
        if obj.type != 'MESH':
            continue
        names = [m.name if m else None for m in obj.data.materials]
        if paint in names and accent in names:
            found.append((obj, names.index(paint), names.index(accent)))
    return found


def _triangles(mesh):
    return sum(len(p.vertices) - 2 for p in mesh.polygons)


def cut_two_tone_split(paint='Paint', accent='Paint_Accent', sigma=None, snap=0.2):
    """Cut every mesh carrying both paints along the smoothed split. Returns a report."""
    targets = _objects_with(paint, accent)
    if not targets:
        return {'objects': 0}
    samples = []
    boundary_lengths = []
    for obj, pi, ai in targets:
        mw = obj.matrix_world
        for poly in obj.data.polygons:
            if poly.material_index in (pi, ai):
                samples.append((mw @ poly.center, 1.0 if poly.material_index == ai else 0.0, poly.area))
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        for edge in bm.edges:
            mats = {f.material_index for f in edge.link_faces}
            if pi in mats and ai in mats:
                boundary_lengths.append(edge.calc_length())
        bm.free()
    if not boundary_lengths:
        return {'objects': len(targets), 'boundaryEdges': 0}
    boundary_lengths.sort()
    step = boundary_lengths[len(boundary_lengths) // 2]
    sigma = sigma or 2.0 * step
    tree = KDTree(len(samples))
    for index, (centre, _, _) in enumerate(samples):
        tree.insert(centre, index)
    tree.balance()
    radius = 3.0 * sigma
    inv = 1.0 / (2.0 * sigma * sigma)

    def field(point):
        num = den = 0.0
        for _, index, dist in tree.find_range(point, radius):
            weight = samples[index][2] * math.exp(-dist * dist * inv)
            num += weight * samples[index][1]
            den += weight
        return num / den if den > 0 else None

    report = dict(objects=len(targets), boundaryEdgesBefore=len(boundary_lengths),
                  medianBoundaryEdgeM=round(step, 4), sigmaM=round(sigma, 4),
                  edgesSplit=0, verticesSnapped=0, facesCut=0, facesRecoloured=0,
                  trianglesAdded=0)
    for obj, pi, ai in targets:
        before = _triangles(obj.data)
        mw = obj.matrix_world
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        painted = lambda face: face.material_index in (pi, ai)  # noqa: E731
        phi = {}
        for vert in bm.verts:
            if any(painted(f) for f in vert.link_faces):
                value = field(mw @ vert.co)
                if value is not None:
                    phi[vert] = value
        band = [f for f in bm.faces if painted(f)
                and all(v in phi for v in f.verts)
                and min(phi[v] for v in f.verts) < 0.98 and max(phi[v] for v in f.verts) > 0.02]
        band_set = set(band)
        on_curve = set()
        # vertices within `snap` of an edge crossing are taken as ON the curve (no sliver)
        crossings = []
        for edge in {e for f in band for e in f.edges}:
            a, b = edge.verts
            if a not in phi or b not in phi:
                continue
            if not all(painted(f) for f in edge.link_faces):
                continue
            fa, fb = phi[a] - 0.5, phi[b] - 0.5
            if fa * fb >= 0:
                continue
            t = fa / (fa - fb)
            if t < snap:
                on_curve.add(a)
            elif t > 1 - snap:
                on_curve.add(b)
            else:
                crossings.append((edge, a, t))
        report['verticesSnapped'] += len(on_curve)
        for edge, a, t in crossings:
            if edge.verts[0] in on_curve or edge.verts[1] in on_curve:
                continue
            new_edge, new_vert = bmesh.utils.edge_split(edge, a, t)
            phi[new_vert] = 0.5
            on_curve.add(new_vert)
            report['edgesSplit'] += 1
        for vert in on_curve:
            phi[vert] = 0.5
        # connect the two curve vertices of each crossed face
        cut_pairs = []
        for face in [f for f in bm.faces if painted(f)]:
            ring = [v for v in face.verts if v in on_curve]
            if len(ring) != 2:
                continue
            a, b = ring
            if any(e for e in face.edges if a in e.verts and b in e.verts):
                continue  # the curve already runs along an edge
            # both sides of the cut must carry off-curve vertices on opposite sides
            others = [phi.get(v, 0.5) - 0.5 for v in face.verts if v not in on_curve]
            if not (any(o > 0 for o in others) and any(o < 0 for o in others)):
                continue
            cut_pairs.append((face, a, b))
        for face, a, b in cut_pairs:
            if not face.is_valid:
                continue
            try:
                bmesh.utils.face_split(face, a, b)
                report['facesCut'] += 1
            except ValueError:
                pass
        # recolour every face in the band by the side of the curve it lies on
        for face in bm.faces:
            if not painted(face):
                continue
            values = [phi[v] - 0.5 for v in face.verts if v in phi and v not in on_curve]
            if not values:
                continue
            if not (face in band_set or any(v in on_curve for v in face.verts)):
                continue
            side = ai if sum(values) / len(values) > 0 else pi
            if face.material_index != side:
                face.material_index = side
                report['facesRecoloured'] += 1
        bm.to_mesh(obj.data)
        obj.data.update()
        bm.free()
        report['trianglesAdded'] += _triangles(obj.data) - before
    return report


def dissolve_flat_hidden(materials, budget_triangles, angle_deg=0.5):
    """Merge coplanar faces of materials nobody sees up close, to pay for a cut.

    Only faces whose material is in `materials` (underbody, cabin shell, voids) are
    touched, and only where neighbours are coplanar within `angle_deg`, so no drawn
    shape changes. Stops once `budget_triangles` have been freed. Returns the count.
    """
    freed = 0
    for obj in bpy.context.scene.objects:
        if freed >= budget_triangles or obj.type != 'MESH':
            continue
        names = [m.name if m else None for m in obj.data.materials]
        wanted = {i for i, n in enumerate(names) if n in materials}
        if not wanted:
            continue
        before = _triangles(obj.data)
        bm = bmesh.new()
        bm.from_mesh(obj.data)
        faces = [f for f in bm.faces if f.material_index in wanted]
        if not faces:
            bm.free()
            continue
        bmesh.ops.dissolve_limit(bm, angle_limit=math.radians(angle_deg), use_dissolve_boundaries=False,
                                 verts=list({v for f in faces for v in f.verts}),
                                 edges=[e for e in {e for f in faces for e in f.edges}
                                        if all(lf.material_index in wanted for lf in e.link_faces)],
                                 delimit={'MATERIAL', 'NORMAL'})
        bm.to_mesh(obj.data)
        obj.data.update()
        bm.free()
        freed += before - _triangles(obj.data)
    return freed
