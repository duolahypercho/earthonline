"""Experimental semantic far-vehicle reduction from the highest assembly.

Call prepare_vehicle_far_lod(source_blend=near_snapshot) before exporting LOD2.
Never call this on an already reduced mid mesh. It owns only the candidate
scene and does not alter the saved near snapshot or runtime manifests.
The current source-derived candidate is visually REJECTED. Numeric budgets
alone do not authorize production promotion; consult the exported review.
"""
import argparse
import hashlib
import json
import math
import sys
from pathlib import Path

import bpy
import bmesh
from mathutils import Vector
from mathutils.bvhtree import BVHTree


FAR_TRIANGLE_CAP = 6000
TRANSFER_SOURCE_NORMALS = False
REQUIRED_NODES = [f'{kind}_{corner}' for kind in ('Wheel','Door','Window') for corner in ('FL','FR','RL','RR')]


def _triangles(ob):
    dg=bpy.context.evaluated_depsgraph_get()
    evaluated=ob.evaluated_get(dg);mesh=evaluated.to_mesh();mesh.calc_loop_triangles()
    count=len(mesh.loop_triangles);evaluated.to_mesh_clear()
    return count


def _census():
    return [{'node':o.name,'triangles':_triangles(o)} for o in bpy.context.scene.objects if o.type=='MESH']


def _repair_winding(ob):
    """Make adjacency consistent, retaining each component's majority facing."""
    bm=bmesh.new();bm.from_mesh(ob.data);bm.normal_update()
    reference={f:(f.normal.copy(),f.calc_area()) for f in bm.faces}
    def inconsistent():
        return sum(len(e.link_loops)==2 and e.link_loops[0].vert==e.link_loops[1].vert for e in bm.edges)
    before=inconsistent()
    bmesh.ops.recalc_face_normals(bm,faces=list(bm.faces))
    seen=set();components=[]
    for seed in bm.faces:
        if seed in seen:continue
        faces=[];stack=[seed];seen.add(seed)
        while stack:
            face=stack.pop();faces.append(face)
            for edge in face.edges:
                if len(edge.link_faces)!=2:continue
                for other in edge.link_faces:
                    if other not in seen:seen.add(other);stack.append(other)
        if sum(reference[f][0].dot(f.normal)*reference[f][1] for f in faces)<0:
            bmesh.ops.reverse_faces(bm,faces=faces)
        components.append(faces)
    after=inconsistent()
    boundaries=[e for e in bm.edges if e.is_boundary];visited=set();loops=[]
    for seed in boundaries:
        if seed in visited:continue
        edges=[];stack=[seed];visited.add(seed)
        while stack:
            edge=stack.pop();edges.append(edge)
            for vertex in edge.verts:
                for other in vertex.link_edges:
                    if other.is_boundary and other not in visited:visited.add(other);stack.append(other)
        points=[ob.matrix_world@v.co for edge in edges for v in edge.verts]
        loops.append({'edges':len(edges),'perimeterM':sum(e.calc_length() for e in edges),
                      'min':[min(p[a] for p in points) for a in range(3)],'max':[max(p[a] for p in points) for a in range(3)],
                      'materials':sorted({ob.data.materials[f.material_index].name for e in edges for f in e.link_faces})})
    bm.to_mesh(ob.data);bm.free();ob.data.update()
    return {'node':ob.name,'inconsistentWindingBefore':before,'inconsistentWindingAfter':after,
            'manifoldFaceComponents':len(components),'boundaryComponents':sorted(loops,key=lambda x:-x['edges'])}


def _replace_grid(ob, indices, faces):
    old=ob.data;vertices=[old.vertices[i].co.copy() for i in indices]
    mesh=bpy.data.meshes.new(old.name+'-far-grid');mesh.from_pydata(vertices,[],faces)
    for material in old.materials:mesh.materials.append(material)
    for face in mesh.polygons:face.use_smooth=True
    # These original analytic panes/rings have no texture maps; their material
    # binding and node transforms remain exact. Keep UVs for loader contracts.
    uv=mesh.uv_layers.new(name='UVMap')
    for loop in mesh.loops:
        p=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(p.x,p.y)
    ob.data=mesh


def _resample_surface(ob, rows, cols, row_step, col_step):
    if len(ob.data.vertices)!=(rows+1)*(cols+1):return False
    js=list(range(0,rows+1,row_step));ks=list(range(0,cols+1,col_step))
    if js[-1]!=rows:js.append(rows)
    if ks[-1]!=cols:ks.append(cols)
    indices=[j*(cols+1)+i for j in js for i in ks];width=len(ks)
    faces=[]
    original=ob.data.polygons[0].normal.copy()
    for j in range(len(js)-1):
        for i in range(width-1):
            a=j*width+i;faces.append((a,a+1,a+width+1,a+width))
    _replace_grid(ob,indices,faces)
    if ob.data.polygons[0].normal.dot(original)<0:ob.data.flip_normals()
    return True


def _resample_ring(ob):
    count=len(ob.data.vertices)
    if count not in (256,640):return False
    n=128;rows=count//n
    # Sample the same source-fitted perimeter at16 points; preserve all four
    # corners and both the inner and outer loops instead of arbitrary collapse.
    perimeter=list(range(0,n,8))
    indices=[row*n+i for row in (0,rows-1) for i in perimeter]
    m=len(perimeter);faces=[];normal=ob.data.polygons[0].normal.copy()
    for i in range(m):
        k=(i+1)%m;faces.append((i,m+i,m+k,k))
    _replace_grid(ob,indices,faces)
    if ob.data.polygons[0].normal.dot(normal)<0:ob.data.flip_normals()
    return True


def _strip_inner_door(ob):
    bm=bmesh.new();bm.from_mesh(ob.data)
    inner={i for i,m in enumerate(ob.data.materials) if m and m.name=='VehicleInterior'}
    unwanted=[f for f in bm.faces if f.material_index in inner]
    count=len(unwanted)
    bmesh.ops.delete(bm,geom=unwanted,context='FACES')
    bmesh.ops.delete(bm,geom=[v for v in bm.verts if not v.link_faces],context='VERTS')
    bm.to_mesh(ob.data);bm.free();ob.data.update()
    return count


def _weld_position_seams(ob, distance=1e-5):
    """Merge duplicated GLB vertices at shared positions.

    A GLB splits vertices at UV and normal seams, so every such vertex looks
    like a boundary vertex. The decimator's boundary protection then refuses to
    reduce anything on a source-imported panel. Welding by position restores the
    real edge topology; UVs are re-derived per loop by the exporter.
    """
    bm=bmesh.new();bm.from_mesh(ob.data)
    before=len(bm.verts)
    bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=distance)
    bm.to_mesh(ob.data);bm.free();ob.data.update()
    return before-len(ob.data.vertices)


def _lamp_group_name(material, world_centre):
    """Canonical far-lens node name for a source lamp face."""
    channel=material.get('installedLampChannel')
    part='Head' if channel=='head' else 'Tail'
    side='L' if world_centre.x<0 else 'R'
    return f'SourceLamp_{part}_{side}'


def _transfer_normals(ob, reference_mesh, reference_matrix):
    if not TRANSFER_SOURCE_NORMALS:return
    reference=bpy.data.objects.new('FarNormalsReference',reference_mesh)
    bpy.context.scene.collection.objects.link(reference)
    reference.matrix_world=reference_matrix
    normal_transfer=ob.modifiers.new('Source normal transfer','DATA_TRANSFER')
    normal_transfer.object=reference;normal_transfer.use_loop_data=True
    normal_transfer.data_types_loops={'CUSTOM_NORMAL'}
    normal_transfer.loop_mapping='POLYINTERP_NEAREST'
    bpy.context.view_layer.objects.active=ob
    bpy.ops.object.modifier_apply(modifier=normal_transfer.name)
    bpy.data.objects.remove(reference,do_unlink=True)


def _reduce(ob, target, *, boundary=False):
    before=_triangles(ob)
    if before<=target:return {'node':ob.name,'before':before,'after':before,'target':target}
    reference_mesh=ob.data.copy()
    shading_before={'smoothFaces':sum(p.use_smooth for p in ob.data.polygons),
                    'sharpEdges':sum(e.use_edge_sharp for e in ob.data.edges),
                    'customNormals':ob.data.has_custom_normals}
    group=None
    bm=bmesh.new();bm.from_mesh(ob.data);bm.verts.ensure_lookup_table()
    def must_keep(v):
        if not any(e.is_boundary for e in v.link_edges):return False
        if ob.name!='Body':return True
        # Lock the shared door cut exactly. Dense original grille/slat
        # boundaries are distance detail, not independent moving seams.
        p=ob.matrix_world@v.co
        door=abs(p.x)>.65 and -.99<p.y<1.23 and .29<p.z<1.01 and (
            min(abs(p.y-y) for y in (-.98,.20,1.22))<.003 or min(abs(p.z-z) for z in (.30,1.0))<.003)
        arch=abs(p.x)>.60 and p.z<.83 and min(abs(p.y-y) for y in (-1.40,1.42))<.58
        side_glass=abs(p.x)>.42 and -.85<p.y<1.85 and p.z>1.0
        return door or arch or side_glass
    protected=[v.index for v in bm.verts if must_keep(v)] if boundary else []
    # Source lens faces must survive the shared shell collapse with area intact
    # even when the shell itself is collapsed without boundary protection.
    lamp_slots={i for i,m in enumerate(ob.data.materials)
                if m and (m.get('installedLampChannel') in ('head','tail')
                          or m.name in ('VehicleHeadlamp','VehicleTaillamp'))}
    lens=[v.index for v in bm.verts if any(f.material_index in lamp_slots for f in v.link_faces)] if lamp_slots else []
    bm.free()
    if protected or lens:
        group=ob.vertex_groups.new(name='FarBoundaryPriority')
        # A source lens is a shallow, high-curvature shell: at a 0.05 priority
        # the shared collapse still ate its interior and dropped ~33% of its
        # area. The authored lens kept >=70% of its source area, so hold the
        # source lens faces at a weight that preserves that contract.
        #
        # 0.20 was measured against a lens that was FLAT on the shell - a
        # painted material region with no relief, which a collapse can flatten
        # further without losing area. Once the lens became real geometry (a
        # lens body standing proud of the skin on a skirt), the same weight let
        # the shared shell collapse hollow it out: measured on the Tripo sedan,
        # `SourceLamp_Head_R` came back at 15 triangles and 0.399 of its source
        # area against this stage's own >=16-triangle / >=0.70-area contract.
        # The weight is protection WITHIN the fixed budget - the decimator still
        # hits its target face count, it just spends it elsewhere - so raising
        # it strengthens the contract instead of buying the lens more triangles.
        if lens:group.add(lens,.88,'REPLACE')
        if protected:group.add(protected,1.,'REPLACE')
    # Direct per-object reduction from the highest snapshot. Moving parts and
    # resampled panes keep independent allocations; the static shell is joint.
    modifier=ob.modifiers.new('Semantic far budget','DECIMATE')
    modifier.ratio=target/before;modifier.use_collapse_triangulate=True
    if group:
        modifier.vertex_group=group.name;modifier.vertex_group_factor=1000;modifier.invert_vertex_group=True
    bpy.context.view_layer.objects.active=ob
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    fresh_group=ob.vertex_groups.get('FarBoundaryPriority')
    if fresh_group:ob.vertex_groups.remove(fresh_group)
    _transfer_normals(ob,reference_mesh,ob.matrix_world.copy())
    geometry_error=None
    if ob.name=='Body':
        bm=bmesh.new();bm.from_mesh(ob.data);tree=BVHTree.FromBMesh(bm)
        samples=sorted((hit[3],tuple(v.co)) for v in reference_mesh.vertices if (hit:=tree.find_nearest(v.co)) and hit[3] is not None)
        distances=[s[0] for s in samples]
        geometry_error={'sourceVertexToFarSurfaceMaxM':max(distances),'sourceVertexToFarSurfaceP95M':distances[int(len(distances)*.95)],
                        'maxErrorSourcePosition':samples[-1][1]}
        bm.free()
    bpy.data.meshes.remove(reference_mesh)
    return {'node':ob.name,'before':before,'after':_triangles(ob),'target':target,
            'normalTransfer':'source nearest-face interpolated' if TRANSFER_SOURCE_NORMALS else 'disabled; geometric normals reviewed better',
            'sourceShading':shading_before,
            'geometryError':geometry_error}


def prepare_vehicle_far_lod(*, source_blend=None):
    """Destructively prepare current candidate scene, optionally reload source.

    Near/mid caps and exports are unaffected. Only LOD2 is produced here.
    Source lamps share collapse topology with their mating shell and retain
    independent exported nodes plus an area floor.
    """
    if source_blend:bpy.ops.wm.open_mainfile(filepath=str(Path(source_blend).resolve()))
    missing=[n for n in REQUIRED_NODES if n not in bpy.data.objects]
    if missing:raise ValueError(f'Highest assembly lacks rig nodes: {missing}')
    before=_census();removed=[];reductions=[];lamps=[];resampled=[];topology=[]
    exterior={'Body','HoodSkin','TrunkSkin','Underbody','RearShelf'}
    exterior.update('Panel_'+c for c in ('FL','FR','RL','RR'))
    exterior.update('Window_'+c for c in ('FL','FR','RL','RR'))
    exterior.update(name+suffix for name in ('Windscreen','RearWindow') for suffix in ('','_Gasket','_Surround','_OuterClosure'))
    wheel_roles=('Tyre','Lip','Hub','Barrel','Cap','Spoke')
    for ob in list(bpy.context.scene.objects):
        if ob.type!='MESH':continue
        wheel=ob.name.startswith('Wheel_') and any(ob.name.split('_',2)[-1].startswith(role) for role in wheel_roles)
        keep=ob.name in exterior or wheel or ob.name.startswith('SourceLamp_')
        # The authored far LODs carry no Panel_* node at all: at 240 m the door
        # skin is folded into the shell and only the Wheel/Door pivots survive.
        # A source-trimmed panel is an open sheet that cannot reach a proxy
        # budget, so the far tier follows the shipped policy and drops it.
        if ob.name.startswith('Panel_'):keep=False
        if not keep:
            removed.append(ob.name);bpy.data.objects.remove(ob,do_unlink=True)
    # A greenhouse/source cut must be simplified as one welded surface. Two
    # independent collapses of matching boundary loops reopen the aperture.
    # Hood/Trunk pivots remain, but far tier deliberately has no moving lids.
    # The Tripo source binds its lamps by material on Body/TrunkSkin rather than
    # as separate nodes, so measure the four lens groups from the source faces
    # and let the post-join material split name them. Keys must match
    # `_lamp_group_name` exactly.
    lens_objects=[]
    for ob in bpy.context.scene.objects:
        if ob.type!='MESH':
            continue
        for material in ob.data.materials:
            if material and material.get('installedLampChannel') in ('head','tail'):
                lens_objects.append(ob)
                break
    source_areas={}
    for ob in lens_objects:
        slots={i for i,m in enumerate(ob.data.materials)
               if m and m.get('installedLampChannel') in ('head','tail')}
        if not slots:
            continue
        for poly in ob.data.polygons:
            if poly.material_index not in slots:
                continue
            name=_lamp_group_name(ob.data.materials[poly.material_index], ob.matrix_world @ poly.center)
            source_areas[name]=source_areas.get(name,0.0)+poly.area
    if set(source_areas)!={f'SourceLamp_{c}_{s}' for c in ('Head','Tail') for s in ('L','R')}:
        raise ValueError(f'Source lamps do not form four lens groups: {None if not source_areas else sorted(source_areas)}')
    source_lamps=lens_objects
    shell=[bpy.data.objects.get(n) for n in ('Body','HoodSkin','TrunkSkin','Windscreen_Surround','RearWindow_Surround','Windscreen_OuterClosure','RearWindow_OuterClosure')]+source_lamps
    shell=[o for o in shell if o is not None]
    shell_names=[o.name for o in shell]
    bpy.ops.object.select_all(action='DESELECT')
    for ob in shell:ob.select_set(True)
    bpy.context.view_layer.objects.active=bpy.data.objects['Body']
    bpy.ops.object.join()
    body=bpy.data.objects['Body'];bm=bmesh.new();bm.from_mesh(body.data)
    bmesh.ops.remove_doubles(bm,verts=list(bm.verts),dist=.00001)
    bm.to_mesh(body.data);bm.free();body.data.update()
    topology.append(_repair_winding(body))
    # SPLIT THE LENSES OUT FIRST, not after the shell reduction.
    #
    # The four lamp groups used to be carved out of the JOINED, ALREADY-REDUCED
    # shell, so the lens competed with 116k triangles of bodywork for a 3784
    # triangle budget and the collapse ate it. Measured on the Tripo sedan with
    # the real lens geometry in place: `SourceLamp_Head_R` came back as 12
    # triangles at 0.46 of its source area against this stage's own >=16 /
    # >=0.70 contract, and vertex-group protection could only rescue it by
    # holding the whole body at 5979 triangles - over the tier cap once the
    # other parts are added. With the split first, each lens is reduced on its
    # own 192-triangle budget by the `SourceLamp_*` arm below, and the shell
    # takes what is left.
    #
    # The shell stays closed underneath: the lamp authoring stage repaints the
    # faces a lens was raised from rather than deleting them, so a lens leaving
    # the shell cannot leave a hole in it.
    body=bpy.data.objects['Body'];bm=bmesh.new();bm.from_mesh(body.data);bm.faces.ensure_lookup_table()
    layer=bm.faces.layers.int.new('far_lamp_part')
    groups={name:[] for name in source_areas}
    for face in bm.faces:
        material=body.data.materials[face.material_index]
        channel=material.get('installedLampChannel') if material else None
        if channel in ('head','tail'):
            name=_lamp_group_name(material, body.matrix_world @ face.calc_center_median())
            if name in groups:groups[name].append(face)
    for serial,(name,faces) in enumerate(groups.items(),1):
        for face in faces:face[layer]=serial
    if any(not f for f in groups.values()):
        raise ValueError(f'Far lens split found an empty group: '
                         f'{[n for n, f in groups.items() if not f]} '
                         f'(expected {sorted(groups)}; shell has '
                         f'{[m.name for m in body.data.materials]})')
    for serial,(name,faces) in enumerate(groups.items(),1):
        copy=bm.copy();labels=copy.faces.layers.int['far_lamp_part']
        bmesh.ops.delete(copy,geom=[f for f in copy.faces if f[labels]!=serial],context='FACES')
        bmesh.ops.delete(copy,geom=[v for v in copy.verts if not v.link_faces],context='VERTS')
        mesh=bpy.data.meshes.new(name+'-far');copy.to_mesh(mesh);copy.free()
        for material in body.data.materials:mesh.materials.append(material)
        ob=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(ob)
        # The lens mesh is a copy of the BODY's mesh, so its vertices are in the
        # body's LOCAL frame. A fresh object carries an identity transform and
        # this body is scaled 4.95x, so without this line the four far lenses
        # shipped at a fifth of their size, clustered at the car's centre
        # instead of on its corners: measured on the far GLB, `SourceLamp_Head_L`
        # was 0.09 m across with its centre at (0.43, -0.15, -0.02), against a
        # body 2.13 x 4.81 m. The area contract above could not catch it because
        # it compares `p.area`, which is in local units on both sides, and so
        # passed while the lamps drew in the wrong place at the wrong scale.
        ob.matrix_world = body.matrix_world.copy()
    bmesh.ops.delete(bm,geom=[f for faces in groups.values() for f in faces],context='FACES')
    bm.to_mesh(body.data);bm.free();body.data.update()
    for ob in list(bpy.context.scene.objects):
        if ob.type!='MESH':continue
        name=ob.name
        if name.startswith('SourceLamp_'):
            source_area=sum(p.area for p in ob.data.polygons)
            # A lens is reduced on its OWN budget, so the shell's vertex-group
            # weighting must not apply: every vertex here carries the lamp
            # material, `_reduce` would put all of them in the protected group,
            # and the tier would then pay ~4x the lens budget it reserved. The
            # area floor stays the binding contract, as it was.
            lens_before=_triangles(ob)
            modifier=ob.modifiers.new('Far lens budget','DECIMATE')
            modifier.ratio=min(1.0,192/max(lens_before,1))
            modifier.use_collapse_triangulate=True
            bpy.context.view_layer.objects.active=ob
            bpy.ops.object.modifier_apply(modifier=modifier.name)
            area=sum(p.area for p in ob.data.polygons)
            after=_triangles(ob)
            if area<source_area*.70 or after<16:raise ValueError(f'Far lamp collapsed: {name}')
            lamps.append({'node':name,'before':lens_before,'after':after,'target':192,
                          'retainedArea':area/source_area,'method':'lens-own-budget'});continue
        if name in ('Windscreen','RearWindow'):
            rows=32 if len(ob.data.vertices)==33*33 else 16
            if _resample_surface(ob,rows,32,rows//2,4):resampled.append(name);continue
        if name.startswith('Window_') and _resample_surface(ob,12,12,4,4):
            resampled.append(name);continue
        if name.startswith(('Windscreen_','RearWindow_')) and _resample_ring(ob):
            resampled.append(name);continue
        if name=='Body':continue
        elif name.startswith('Panel_'):
            _strip_inner_door(ob);topology.append(_repair_winding(ob))
            welded=_weld_position_seams(ob)
            if welded:topology.append({'node':name,'weldedSeamVertices':welded})
            target=330
        elif name in ('HoodSkin','TrunkSkin'):target=120
        elif name.startswith('Wheel_'):
            role=name.split('_',2)[-1]
            target=128 if role=='Tyre' else 32 if role=='Lip' else 18 if role in ('Hub','Barrel') else 8 if role=='Cap' else 6
        elif name.startswith(('Windscreen','RearWindow','Window_')):target=40
        else:target=12
        reductions.append(_reduce(ob,target,boundary=name=='Body' or name.startswith('Panel_') or name.endswith('Skin')))
    # Rigid shell receives the remaining explicitly capped allowance. This
    # never revisits or steals vertices from any finished moving part/lens.
    other=sum(_triangles(o) for o in bpy.context.scene.objects if o.type=='MESH' and o.name!='Body')
    # Small articulated parts can floor above their share on a source assembly
    # that carries more pieces than the authored fleet. Shave the largest
    # non-exterior part until the rigid shell has a legal budget, instead of
    # failing the tier on a total the caller cannot influence.
    # The authored fleet keeps a 2200-triangle shell floor because its doors are
    # closed shells: a 330-triangle panel there still has interior support. This
    # source assembly's door skins are open trimmed surfaces whose vertices are
    # mostly boundary (1330 of 2393 on Panel_FL), so protected boundary
    # reduction floors each panel near 900 and leaves the shell only ~870.
    # The tier cap is unchanged; only this floor is source-appropriate.
    shell_floor=max(800,int(FAR_TRIANGLE_CAP*0.13))
    for _ in range(40):
        other=sum(_triangles(o) for o in bpy.context.scene.objects if o.type=='MESH' and o.name!='Body')
        if min(3784,FAR_TRIANGLE_CAP-other-24)>=shell_floor:break
        donors=sorted((o for o in bpy.context.scene.objects if o.type=='MESH'
                       and o.name!='Body' and _triangles(o)>8
                       and not o.name.startswith('SourceLamp_')),
                      key=_triangles,reverse=True)
        if not donors:break
        ob=donors[0];current=_triangles(ob)
        target=max(6,int(current*.55))
        if ob.name.startswith(('Panel_','HoodSkin','TrunkSkin')):
            welded=_weld_position_seams(ob)
            if welded:
                topology.append({'node':ob.name,'weldedSeamVertices':welded})
        # The authored fleet protects every boundary vertex on a door panel
        # because those panels are closed shells with a short aperture loop. A
        # source-trimmed panel is an open sheet whose boundary is most of its
        # vertices (1330 of 2393 before welding), so the same rule floors it
        # near 900 and overspends the whole tier. Reduce the panel unconstrained
        # instead of overspending; the shipped tier cap is unchanged.
        reductions.append(_reduce(ob,target))
    other=sum(_triangles(o) for o in bpy.context.scene.objects if o.type=='MESH' and o.name!='Body')
    body_budget=min(3784,FAR_TRIANGLE_CAP-other-24)
    if body_budget<shell_floor:
        census=sorted(({'node':o.name,'triangles':_triangles(o)} for o in bpy.context.scene.objects
                       if o.type=='MESH' and o.name!='Body'),key=lambda r:r['triangles'],reverse=True)[:16]
        raise ValueError(f'Far semantic parts leave too little body budget: {body_budget} '
                         f'(cap {FAR_TRIANGLE_CAP}, other {other}, floor {shell_floor}); largest non-body: {census}')
    shell_normal_reference=body.data.copy();shell_normal_matrix=body.matrix_world.copy()
    # The joined shell is a GLB import, so UV/normal seams split its vertices and
    # the boundary protection then treats most of the surface as a seam. Welding
    # by position restores real edge topology so the shell can reach its budget.
    welded_shell=_weld_position_seams(body)
    if welded_shell:topology.append({'node':'Body','weldedSeamVertices':welded_shell,'stage':'far shell'})
    # At 240 m the authored far body is 833 triangles of plain proxy shell with
    # no boundary protection. A source-imported shell carries far more true
    # boundary than the authored one, so protecting every seam leaves it at
    # ~5.0k and overspends the tier. The cap, the rig nodes and the lamp material
    # binding are unchanged; only the seam protection is dropped at this tier.
    reductions.append(_reduce(bpy.data.objects['Body'],body_budget))
    # BMesh partitioning discards custom split normals; transfer after the final
    # topology operation to both the source shell and its four lens partitions.
    for ob in [body]+[bpy.data.objects[n] for n in source_areas]:
        _transfer_normals(ob,shell_normal_reference,shell_normal_matrix)
    bpy.data.meshes.remove(shell_normal_reference)
    # Far source texture copy only; the saved HD/near images stay immutable.
    copied={}
    for ob in bpy.context.scene.objects:
        if ob.type!='MESH':continue
        for material in ob.data.materials:
            if not material or not material.use_nodes:continue
            for node in material.node_tree.nodes:
                if node.type=='TEX_IMAGE' and node.image and max(node.image.size)>512:
                    original=node.image
                    if original not in copied:
                        image=original.copy();image.scale(512,512);copied[original]=image
                    node.image=copied[original]
    after=_census();total=sum(r['triangles'] for r in after)
    missing=[n for n in REQUIRED_NODES if n not in bpy.data.objects]
    if missing or total>FAR_TRIANGLE_CAP:
        print(json.dumps({'failedCensus':after,'reductions':reductions}))
        raise ValueError(f'Far candidate fails contract: triangles={total}, missing={missing}')
    if len(lamps)!=4:raise ValueError('Far candidate must retain four actual source lenses')
    return {'version':'vehicle-semantic-far-v10-candidate','sourceTriangles':sum(r['triangles'] for r in before),
            'triangles':total,'cap':FAR_TRIANGLE_CAP,'removed':removed,'reductions':reductions,
            'resampled':resampled,'lamps':lamps,'parts':after,'requiredNodes':REQUIRED_NODES,
            'mergedStaticShell':shell_names,'farLidArticulation':False,
            'topology':topology,
            'status':'candidate; round-trip and visual acceptance required'}


def _render(output):
    scene=bpy.context.scene;scene.render.engine='CYCLES';scene.cycles.samples=24
    scene.render.resolution_x=1400;scene.render.resolution_y=850;scene.render.resolution_percentage=100
    scene.world.color=(.18,.18,.18)
    for name,loc,power,size in [('Key',(-3,-4,6),1200,5),('Fill',(4,1,4),850,4),('Rim',(0,4,5),1100,4)]:
        data=bpy.data.lights.new(name,'AREA');data.energy=power;data.shape='DISK';data.size=size
        ob=bpy.data.objects.new(name,data);scene.collection.objects.link(ob);ob.location=loc
        ob.rotation_euler=(Vector((0,0,.6))-ob.location).to_track_quat('-Z','Y').to_euler()
    camera=bpy.data.objects.new('FarReview',bpy.data.cameras.new('FarReview'))
    scene.collection.objects.link(camera);scene.camera=camera;camera.data.type='ORTHO';camera.data.ortho_scale=5.7
    for name,loc in [('quarter',(-5,-6,3.2)),('front',(0,-8,1.2)),('rear',(0,8,1.2)),('side',(-8,0,.85))]:
        camera.location=loc;camera.rotation_euler=(Vector((0,0,.72))-camera.location).to_track_quat('-Z','Y').to_euler()
        scene.render.filepath=str(output/f'{name}.png');bpy.ops.render.render(write_still=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--source',required=True);parser.add_argument('--output',required=True);parser.add_argument('--render',action='store_true')
    args=parser.parse_args(sys.argv[sys.argv.index('--')+1:]);output=Path(args.output).resolve();output.mkdir(parents=True,exist_ok=True)
    report=prepare_vehicle_far_lod(source_blend=args.source)
    bpy.ops.wm.save_as_mainfile(filepath=str(output/'far-candidate.blend'))
    glb=output/'blue-sedan-lod2.glb'
    bpy.ops.export_scene.gltf(filepath=str(glb),export_format='GLB',export_apply=True,export_yup=True,export_animations=False,export_skins=False,export_extras=True)
    bpy.ops.object.select_all(action='SELECT');bpy.ops.object.delete(use_global=False)
    bpy.ops.import_scene.gltf(filepath=str(glb))
    report['roundtripTriangles']=sum(r['triangles'] for r in _census());report['sha256']=hashlib.sha256(glb.read_bytes()).hexdigest()
    report['roundtripMissingNodes']=[n for n in REQUIRED_NODES if n not in bpy.data.objects]
    if report['roundtripTriangles']>FAR_TRIANGLE_CAP or report['roundtripMissingNodes']:raise ValueError('Exported far GLB fails rig/budget contract')
    (output/'far-report.json').write_text(json.dumps(report,indent=2)+'\n')
    if args.render:_render(output)
