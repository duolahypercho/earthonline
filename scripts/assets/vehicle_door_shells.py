"""Close prepared door skins and construct their exposed inner surfaces."""
import bpy
import bmesh
from mathutils import Vector


def _box(name, location, dimensions, mat, parent):
    bpy.ops.mesh.primitive_cube_add(size=1, location=location)
    ob=bpy.context.object;ob.name=name;ob.dimensions=dimensions
    bpy.ops.object.transform_apply(location=False,rotation=False,scale=True)
    ob.data.materials.append(mat)
    bevel=ob.modifiers.new('Rounded upholstered edge','BEVEL')
    bevel.width=.018;bevel.segments=3
    world=ob.matrix_world.copy();ob.parent=parent;ob.matrix_world=world
    return ob


def _topology(bm):
    return {'vertices':len(bm.verts),'faces':len(bm.faces),
        'boundaryEdges':sum(edge.is_boundary for edge in bm.edges),
        'nonManifoldEdges':sum(not edge.is_manifold for edge in bm.edges),
        'inconsistentWindingEdges':sum(edge.is_manifold and not edge.is_contiguous for edge in bm.edges),
        'zeroAreaFaces':sum(face.calc_area()<1e-10 for face in bm.faces)}


def _directed_boundary(bm):
    # BMEdge.verts has no face-winding guarantee. The adjacent face loop does.
    # A rim must traverse that outer edge in the opposite direction or the
    # closed shell has two faces pointing through the same edge the same way.
    return [(edge,edge.link_loops[0].vert,edge.link_loops[0].link_loop_next.vert)
            for edge in bm.edges if edge.is_boundary]


def _jamb_clearance(point, yf, yr):
    # The fixed reveal must not be coplanar with a closed door's moving rim.
    # Move it 3 mm toward the adjacent chassis, leaving source door vertices
    # untouched. At corners both planes contribute to the same shared vertex.
    result=point.copy()
    for axis,plane,direction in [(1,yf,-1),(1,yr,1),(2,.30,-1),(2,1.0,1)]:
        if abs(point[axis]-plane)<.003:result[axis]+=direction*.003
    return result


def finish_door_shells():
    interior=bpy.data.materials['VehicleInterior']
    report={}
    for key in ['FL','FR','RL','RR']:
        door=bpy.data.objects['Door_'+key];skin=bpy.data.objects['Panel_'+key]
        sx=-1 if key.endswith('L') else 1
        yf,yr=(-.98,.20) if key.startswith('F') else (.20,1.22)
        bm=bmesh.new();bm.from_mesh(skin.data)
        before=_topology(bm)
        # A cut can leave two surface fans touching at one vertex. Extruding
        # that shared point creates a four-face connector edge. Separate the
        # fans in place, retaining every source coordinate and UV loop.
        separated_vertices=0
        for vertex in list(bm.verts):
            edges=[edge for edge in vertex.link_edges if edge.is_boundary]
            if len(edges)>2:
                separated_vertices+=len(bmesh.utils.vert_separate(vertex,edges))-1
        original_faces=list(bm.faces)
        bmesh.ops.recalc_face_normals(bm,faces=original_faces)
        # These are open exterior side sheets. Establish the outward side
        # explicitly before deriving their inward copy; do not infer it from
        # the arbitrary orientation chosen for an open surface by BMesh.
        world_normals=skin.matrix_world.to_3x3().inverted().transposed()
        facing=sum((world_normals@face.normal).x*face.calc_area()*sx for face in original_faces)
        if facing<0:bmesh.ops.reverse_faces(bm,faces=original_faces)
        boundary=_directed_boundary(bm)
        bm.verts.index_update()
        # Use exact source boundary vertices for both rings. Sharing each
        # vertex across adjacent quads removes disconnected per-edge slivers.
        verts=[];faces=[];jamb_indices={}
        def jamb_vertex(vertex,inner):
            key=(vertex.index,inner)
            if key not in jamb_indices:
                point=_jamb_clearance(skin.matrix_world@vertex.co,yf,yr)
                if inner:point-=Vector((sx*.065,0,0))
                jamb_indices[key]=len(verts);verts.append(point)
            return jamb_indices[key]
        for edge,a,b in boundary:
            points=[skin.matrix_world@vertex.co for vertex in (a,b)]
            if max(abs(p.x) for p in points)<.70:continue
            # Interior remnant boundaries are not door aperture edges.
            if not any(all(abs(p[axis]-plane)<.003 for p in points)
                       for axis,plane in [(1,yf),(1,yr),(2,.30),(2,1.0)]):continue
            faces.append((jamb_vertex(b,False),jamb_vertex(a,False),
                          jamb_vertex(a,True),jamb_vertex(b,True)))
        jamb_data=bpy.data.meshes.new('DoorJamb_'+key)
        jamb_data.from_pydata(verts,[],faces);jamb_data.materials.append(interior)
        jamb=bpy.data.objects.new('DoorJamb_'+key,jamb_data)
        bpy.context.scene.collection.objects.link(jamb)
        skin.data.materials.append(interior)
        offset=skin.matrix_world.to_3x3().inverted()@Vector((-sx*.022,0,0))
        original_vertices=list(bm.verts)
        inner={v:bm.verts.new(v.co+offset) for v in original_vertices}
        inner_index=len(skin.data.materials)-1
        for face in original_faces:
            copy=bm.faces.new([inner[v] for v in reversed(list(face.verts))])
            copy.material_index=inner_index;copy.smooth=True
        for edge,a,b in boundary:
            rim=bm.faces.new([b,a,inner[a],inner[b]])
            rim.material_index=inner_index;rim.smooth=False
            edge.smooth=False
            inner_edge=bm.edges.get((inner[a],inner[b]))
            if inner_edge:inner_edge.smooth=False
        bm.normal_update()
        after=_topology(bm)
        volume=bm.calc_volume(signed=True)
        bm.to_mesh(skin.data);bm.free()
        old=bpy.data.objects.get('DoorCard_'+key)
        if old:bpy.data.objects.remove(old,do_unlink=True)
        middle=(yf+yr)/2
        _box('DoorCard_'+key,(sx*.815,middle,.665),(.045,yr-yf-.12,.51),interior,door)
        _box('DoorArmrest_'+key,(sx*.765,middle,.735),(.095,(yr-yf)*.56,.09),interior,door)
        _box('DoorPocket_'+key,(sx*.778,middle+.10,.48),(.06,(yr-yf)*.52,.075),interior,door)
        _box('DoorBeltSeal_'+key,(sx*.802,middle,1.002),(.032,yr-yf-.025,.026),interior,door)
        report[key]={'shellThicknessM':.022,'fixedRevealFaces':len(faces),
            'fixedRevealVertices':len(verts),'fixedRevealClearanceM':.003,
            'sourcePinchVerticesSeparated':separated_vertices,
            'originalSurfaceDisplacementM':0,'before':before,'after':after,
            'signedVolumeM3':volume,
            'status':'closed-consistently-oriented' if after['nonManifoldEdges']==0
                and after['inconsistentWindingEdges']==0 and after['zeroAreaFaces']==0
                else 'requires-source-topology-review'}
    return report
