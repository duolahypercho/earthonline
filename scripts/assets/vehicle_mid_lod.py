"""Prepare the mid vehicle tier while preserving articulated surface joins.

Always start from the saved complete near assembly. This helper changes the
candidate scene only, preserving source files, transforms and material names.
"""
from pathlib import Path
import bpy
import bmesh


MID_TRIANGLE_CAP = 22000


def _triangles(ob):
    ob.data.calc_loop_triangles()
    return len(ob.data.loop_triangles)


def _key(point):
    return tuple(round(c, 6) for c in point)


def _protected_geometry(ob):
    bm=bmesh.new();bm.from_mesh(ob.data);bm.verts.ensure_lookup_table()
    edges={edge for edge in bm.edges if edge.is_boundary}
    if ob.name.startswith('DoorJamb_'):
        # Each reveal pair shares longitudinal/vertical coordinates and is
        # separated laterally by65mm. Preserve the outer reveal against the
        # chassis; its recessed inner edge may be simplified independently.
        groups={}
        for vertex in bm.verts:
            p=ob.matrix_world@vertex.co;k=(round(p.y,6),round(p.z,6))
            groups[k]=max(groups.get(k,0),abs(p.x))
        outer={v for v in bm.verts if abs((ob.matrix_world@v.co).x)>=
               groups[(round((ob.matrix_world@v.co).y,6),round((ob.matrix_world@v.co).z,6))]-1e-5}
        edges={e for e in bm.edges if all(v in outer for v in e.verts)}
    if ob.get('_protectZones'):
        # paint-zone outlines (the black window band, mask, bumper band) are held like boundaries: a
        # collapse across them smeared black wedges over the paint at the mid and far tiers
        edges.update(e for e in bm.edges if len(e.link_faces) == 2 and e.link_faces[0].material_index != e.link_faces[1].material_index)
    if ob.name.startswith('Panel_'):
        # Door shells are closed: their outer/inner rim must be protected even
        # though none of those edges is a mesh boundary after extrusion.
        edges.update(edge for face in bm.faces if not face.smooth for edge in face.edges)
    vertices={v for edge in edges for v in edge.verts}
    indices=[v.index for v in vertices]
    keys={_key(v.co) for v in vertices}
    edge_keys={tuple(sorted(_key(v.co) for v in edge.verts)) for edge in edges}
    bm.free()
    return indices,keys,edge_keys


def _reduce(ob,target,protect=False):
    before=_triangles(ob)
    indices,keys,edge_keys=_protected_geometry(ob) if protect else ([],set(),set())
    record=dict(node=ob.name,before=before,target=target,protectedVertices=len(keys),protectedEdges=len(edge_keys))
    if before<=target:return dict(record,after=before,missingProtectedVertices=0,missingProtectedEdges=0)
    original=ob.data.copy();attempts=[]
    trial=max(4,int(target))
    while True:
        previous=ob.data;ob.data=original.copy()
        if previous.users==0:bpy.data.meshes.remove(previous)
        group=ob.vertex_groups.get('MidBoundaryPriority')
        if group:ob.vertex_groups.remove(group)
        if indices:
            group=ob.vertex_groups.new(name='MidBoundaryPriority');group.add(indices,1.,'REPLACE')
        modifier=ob.modifiers.new('Preserve mid surface joins','DECIMATE')
        modifier.ratio=min(1,trial/before);modifier.use_collapse_triangulate=True
        if indices:
            modifier.vertex_group=group.name
            modifier.invert_vertex_group=True;modifier.vertex_group_factor=1000
        bpy.context.view_layer.objects.active=ob
        bpy.ops.object.modifier_apply(modifier=modifier.name)
        faces_before_validation=len(ob.data.polygons)
        ob.data.validate(clean_customdata=False)
        ob.data.update()
        present={_key(v.co) for v in ob.data.vertices}
        present_edges={tuple(sorted((_key(ob.data.vertices[e.vertices[0]].co),
                                   _key(ob.data.vertices[e.vertices[1]].co)))) for e in ob.data.edges}
        missing_vertices=len(keys-present);missing_edges=len(edge_keys-present_edges)
        attempts.append(dict(target=trial,triangles=_triangles(ob),missingProtectedVertices=missing_vertices,
                             missingProtectedEdges=missing_edges,
                             invalidFacesRemoved=faces_before_validation-len(ob.data.polygons)))
        if not missing_vertices and not missing_edges:break
        if trial>=before:raise ValueError(f'{ob.name}: source boundary cannot be retained')
        # The weighting is a priority, not a hard constraint. Verify the actual
        # result and back off instead of accepting a moved/deleted seam vertex.
        trial=min(before,max(trial+32,int(trial*1.22)))
    group=ob.vertex_groups.get('MidBoundaryPriority')
    if group:ob.vertex_groups.remove(group)
    if original.users==0:bpy.data.meshes.remove(original)
    return dict(record,after=_triangles(ob),attempts=attempts,missingProtectedVertices=0,missingProtectedEdges=0)


def _replace_indices(ob,indices,faces):
    old=ob.data;normal=old.polygons[0].normal.copy()
    mesh=bpy.data.meshes.new(old.name+'-mid')
    mesh.from_pydata([old.vertices[i].co.copy() for i in indices],[],faces)
    for material in old.materials:mesh.materials.append(material)
    mesh.update()
    if mesh.polygons[0].normal.dot(normal)<0:mesh.flip_normals()
    for face in mesh.polygons:face.use_smooth=True
    uv=mesh.uv_layers.new(name='UVMap')
    for loop in mesh.loops:
        point=mesh.vertices[loop.vertex_index].co;uv.data[loop.index].uv=(point.x,point.y)
    ob.data=mesh


def _grid_perimeter(low,high,step,stride):
    return ([low*stride+i for i in range(low,high,step)]
            +[j*stride+high for j in range(low,high,step)]
            +[high*stride+i for i in range(high,low,-step)]
            +[j*stride+low for j in range(high,low,-step)])


def _glass_target(ob,size):
    """Pane budget for a source-fitted perimeter rather than an authored grid."""
    return 640 if size==32 else max(96,_triangles(ob)//3)


def _reduce_glass_mesh(ob,target):
    """Decimate a measured source pane while keeping its fitted silhouette.

    The authored fleet supplies quad-grid glass, which gets the analytic
    perimeter treatment. Panes fitted to the Tripo apertures are arbitrary
    perimeters, so they collapse with a boundary-preserving decimator instead of
    being deleted; the pane still has to fill its aperture at the far tier.
    """
    before=_triangles(ob)
    if before<=target:return dict(node=ob.name,before=before,after=before,method='source-fitted pane under budget')
    ob.data.calc_loop_triangles()
    modifier=ob.modifiers.new('Mid pane decimate','DECIMATE')
    modifier.decimate_type='COLLAPSE';modifier.ratio=max(1e-3,target/max(1,before))
    modifier.use_collapse_triangulate=True
    bpy.context.view_layer.objects.active=ob
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    after=_triangles(ob)
    if after>target:raise ValueError(f'{ob.name}: pane decimation missed its budget {after}>{target}')
    return dict(node=ob.name,before=before,after=after,method='source-fitted pane collapse')


def _reduce_glass_grid(ob,size):
    if len(ob.data.vertices)!=(size+1)**2:return _reduce_glass_mesh(ob,target=_glass_target(ob,size))
    before=_triangles(ob);stride=size+1
    low=size//4;high=size-low;step=4 if size==32 else 3
    outer=_grid_perimeter(0,size,1,stride)
    inner=_grid_perimeter(low,high,step,stride)
    grid=[j*stride+i for j in range(low,high+1,step) for i in range(low,high+1,step)]
    indices=outer+grid;lookup={old:new for new,old in enumerate(indices)}
    faces=[];n=len(outer);m=len(inner);i=j=0
    while i<n or j<m:
        a=lookup[outer[i%n]];b=lookup[inner[j%m]]
        if i<n and (j==m or (i+1)/n<=(j+1)/m):
            faces.append((a,lookup[outer[(i+1)%n]],b));i+=1
        else:
            faces.append((a,lookup[inner[(j+1)%m]],b));j+=1
    width=(high-low)//step+1
    for j in range(width-1):
        for i in range(width-1):
            a=n+j*width+i;faces.append((a,a+1,a+width+1,a+width))
    _replace_indices(ob,indices,faces)
    return dict(node=ob.name,before=before,after=_triangles(ob),retainedPerimeterVertices=n)


def _reduce_gasket(ob):
    if len(ob.data.vertices)!=640:raise ValueError(f'{ob.name}: unexpected gasket layout')
    before=_triangles(ob);n=128
    indices=list(range(n))+list(range(4*n,5*n))
    faces=[(i,n+i,n+(i+1)%n,(i+1)%n) for i in range(n)]
    _replace_indices(ob,indices,faces)
    return dict(node=ob.name,before=before,after=_triangles(ob),retainedPerimeterVertices=256)


def prepare_vehicle_mid_lod(*,source_blend=None):
    if source_blend:bpy.ops.wm.open_mainfile(filepath=str(Path(source_blend).resolve()))
    required=['Body','Hood','HoodSkin','Trunk','TrunkSkin']
    required += [f'{prefix}_{side}' for prefix in ('Wheel','Door','Panel','Window') for side in ('FL','FR','RL','RR')]
    missing=[name for name in required if not bpy.data.objects.get(name)]
    if missing:raise ValueError(f'Mid source lacks articulated nodes: {missing}')
    transforms={o.name:(o.parent.name if o.parent else None,tuple(v for row in o.matrix_world for v in row))
                for o in bpy.context.scene.objects}
    # Resolve bevels before counting or reducing; evaluated triangles, not the
    # unmodified cage, determine the export budget.
    for ob in list(bpy.context.scene.objects):
        if ob.type!='MESH':continue
        bpy.context.view_layer.objects.active=ob
        for modifier in list(ob.modifiers):bpy.ops.object.modifier_apply(modifier=modifier.name)
    before=sum(_triangles(o) for o in bpy.context.scene.objects if o.type=='MESH')
    removed=[];resampled=[];reductions=[]
    # Source articles carry articulated seams the authored fleet does not, so
    # allocate the shell first and let the protected parts follow. Solving the
    # body last let seam back-off consume the entire tier budget.
    body_object=bpy.data.objects['Body']
    body_budget=MID_TRIANGLE_CAP//2
    for ob in list(bpy.context.scene.objects):
        if ob.type!='MESH':continue
        name=ob.name
        if name=='Body':continue
        if name.startswith('Wheel_') and '_Groove' in name:
            removed.append(name);bpy.data.objects.remove(ob,do_unlink=True);continue
        if name in ('Windscreen','RearWindow'):
            resampled.append(_reduce_glass_grid(ob,32));continue
        if name.startswith('Window_'):
            resampled.append(_reduce_glass_grid(ob,12));continue
        if name in ('Windscreen_Gasket','RearWindow_Gasket'):
            resampled.append(_reduce_gasket(ob));continue
        if name.startswith(('Windscreen','RearWindow')) and _triangles(ob)>0:
            resampled.append(_reduce_glass_mesh(ob,_glass_target(ob,32)));continue
        if name.endswith('_Surround'):continue
        if name=='Body':continue
        protect=False
        if name.startswith('SourceLamp_'):target=650;protect=True
        elif name.startswith('DoorJamb_'):target=int(_triangles(ob)*.58);protect=True
        elif name in ('HoodSkin','TrunkSkin'):target=450 if name=='HoodSkin' else 750;protect=True
        elif name.startswith('Panel_'):target=1050 if name[6]=='F' else 650;protect=True
        elif name.startswith('Source'):target=min(_triangles(ob),180)
        elif name.startswith('Wheel_'):
            role=name.split('_',2)[-1]
            target=256 if role=='Tyre' else 64 if role=='Lip' else 36 if role in ('Hub','Barrel') else 8 if role.startswith('Spoke') else 12
        elif name.startswith('Seat'):target=24
        elif name.startswith(('Dash','Console','Steering')):target=48
        elif name.startswith(('DoorCard_','DoorArmrest_','DoorPocket_','DoorBeltSeal_')):target=24
        else:target=min(_triangles(ob),20)
        reductions.append(_reduce(ob,target,protect))
    body=body_object
    remaining=MID_TRIANGLE_CAP-sum(_triangles(o) for o in bpy.context.scene.objects if o.type=='MESH' and o!=body)
    # Source articles (inner cards, belts, cabin fittings) are small and dense.
    # Shave the non-protected ones until the body has room, then re-check.
    if remaining<1000:
        shaveable=sorted((o for o in bpy.context.scene.objects if o.type=='MESH' and o!=body
                          and not o.name.startswith(('SourceLamp_','DoorJamb_','Panel_','HoodSkin','TrunkSkin'))
                          and _triangles(o)>40),
                         key=lambda o:_triangles(o),reverse=True)
        for ob in shaveable:
            if remaining>=1000:break
            current=_triangles(ob);floor=24
            if current<=floor:continue
            target=max(floor,int(current*.35))
            reductions.append(_reduce(ob,target,False))
            remaining=MID_TRIANGLE_CAP-sum(_triangles(o) for o in bpy.context.scene.objects if o.type=='MESH' and o!=body)
    if remaining<1000:raise ValueError(f'Protected mid parts leave only {remaining} triangles for body')
    reductions.append(_reduce(body,remaining-16,True))
    # Boundary back-off can leave any single object above its target. Enforce the
    # tier total by shaving the largest remaining budget before failing.
    for _ in range(24):
        parts=[dict(node=o.name,triangles=_triangles(o)) for o in bpy.context.scene.objects if o.type=='MESH']
        total=sum(p['triangles'] for p in parts)
        if total<=MID_TRIANGLE_CAP:break
        body_triangles=_triangles(body)
        donors=[o for o in bpy.context.scene.objects if o.type=='MESH' and _triangles(o)>8]
        donors.sort(key=_triangles,reverse=True)
        reduced=False
        for ob in donors:
            current=_triangles(ob)
            slack=MID_TRIANGLE_CAP-total
            if ob is body and body_triangles and current<=max(4,body_triangles*0.5):
                continue
            target=max(4,current+slack)
            if target>=current:continue
            reductions.append(_reduce(ob,target,False))
            reduced=True
            break
        if not reduced:
            body_budget=max(4,int(_triangles(body)+MID_TRIANGLE_CAP-total))
            reductions.append(_reduce(body,body_budget,False))
    parts=[dict(node=o.name,triangles=_triangles(o)) for o in bpy.context.scene.objects if o.type=='MESH']
    total=sum(p['triangles'] for p in parts)
    if total>MID_TRIANGLE_CAP:
        worst=sorted(parts,key=lambda p:p['triangles'],reverse=True)[:18]
        raise ValueError(f'Preserved mid boundaries require {total} triangles, over {MID_TRIANGLE_CAP}; '
                         f'largest parts: {worst}')
    for name,(parent,transform) in transforms.items():
        ob=bpy.data.objects.get(name)
        if ob is None:
            if name not in removed:raise ValueError(f'Mid preparation lost {name}')
            continue
        if (ob.parent.name if ob.parent else None)!=parent or tuple(v for row in ob.matrix_world for v in row)!=transform:
            raise ValueError(f'Mid preparation changed articulation transform: {name}')
    return dict(trianglesBefore=before,triangles=total,cap=MID_TRIANGLE_CAP,removed=removed,
                resampled=resampled,reductions=reductions,parts=parts,requiredNodes=required,
                status='candidate; exported seam and visual review required')
