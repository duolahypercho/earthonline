"""Repair generated windshield interfaces in the normalized sedan frame.

Called by the vehicle builder after separating body panels. Works on the
candidate scene only; does not read or overwrite a source asset on disk.
"""
import math
from collections import defaultdict
import bpy
import bmesh
from mathutils import Vector


APERTURES = {
    'Windscreen': dict(front=True, bottom=(0, -1.10, 1.015), top=(0, -.405, 1.35),
                       bottom_width=.72, top_width=.54, bow=.09),
    'RearWindow': dict(front=False, bottom=(0, 1.88, 1.12), top=(0, 1.225, 1.325),
                       bottom_width=.61, top_width=.525, bow=-.10),
}


def _surface(spec, u, t):
    bottom=Vector(spec['bottom']);top=Vector(spec['top'])
    p=bottom.lerp(top,t)
    p.x=u*(spec['bottom_width']+(spec['top_width']-spec['bottom_width'])*t)
    p.y+=spec['bow']*u*u
    p.z-=.024*u*u*t
    # A modest compound bow, shared by glass and every perimeter component.
    p.z+=.012*math.sin(math.pi*max(0,min(1,t)))*(1-u*u)
    return p


def _frame(spec):
    bottom=Vector(spec['bottom']);along=(Vector(spec['top'])-bottom)
    length=along.length;along.normalize()
    normal=Vector((0,-along.z,along.y))
    if not spec['front']:normal.negate()
    return bottom,along,normal,length


def _trim_material(name,color):
    mat=bpy.data.materials.get(name) or bpy.data.materials.new(name)
    mat.use_nodes=True
    p=mat.node_tree.nodes.get('Principled BSDF')
    p.inputs['Base Color'].default_value=(*color,1)
    p.inputs['Metallic'].default_value=0
    p.inputs['Roughness'].default_value=.72
    mat['envClass']='painted-metal'
    return mat


def _mesh(name,verts,faces,material):
    mesh=bpy.data.meshes.new(name);mesh.from_pydata(verts,[],faces)
    mesh.materials.append(material)
    uv=mesh.uv_layers.new(name='UVMap')
    for poly in mesh.polygons:
        poly.use_smooth=True
        for index in poly.loop_indices:
            point=mesh.vertices[mesh.loops[index].vertex_index].co
            uv.data[index].uv=(point.x,point.y)
    ob=bpy.data.objects.new(name,mesh);bpy.context.scene.collection.objects.link(ob)
    return ob


def _glass(spec,name,material):
    rows=32;cols=32;verts=[];faces=[]
    for j in range(rows+1):
        for i in range(cols+1):verts.append(_surface(spec,2*i/cols-1,j/rows))
    for j in range(rows):
        for i in range(cols):
            a=j*(cols+1)+i;face=(a,a+1,a+cols+2,a+cols+1)
            faces.append(face if spec['front'] else face[::-1])
    return _mesh(name,verts,faces,material)


def _perimeter(width,low,high,count=32):
    result=[]
    for i in range(count):result.append((-width+2*width*i/count,low))
    for i in range(count):result.append((width,low+(high-low)*i/count))
    for i in range(count):result.append((width-2*width*i/count,high))
    for i in range(count):result.append((-width,high-(high-low)*i/count))
    return result


def _ring(spec,name,material,inside,outside,offset=0,outer_points=None):
    _,_,normal,_=_frame(spec)
    inner=_perimeter(*inside);outer=_perimeter(*outside)
    n=len(inner);faces=[];verts=[];rows=4
    for row in range(rows+1):
        weight=row/rows
        for i,((ui,ti),(uo,to)) in enumerate(zip(inner,outer)):
            p=_surface(spec,ui+(uo-ui)*weight,ti+(to-ti)*weight)
            if outer_points:p+=(outer_points[i]-_surface(spec,uo,to))*weight*weight*(3-2*weight)
            verts.append(p+normal*offset)
    for row in range(rows):
        for i in range(n):
            nxt=(i+1)%n;a=row*n;face=(a+i,a+n+i,a+n+nxt,a+nxt)
            faces.append(face if spec['front'] else face[::-1])
    return _mesh(name,verts,faces,material)


def _key(point):
    # Match float32 source coordinates across independently articulated skins.
    return tuple(round(c, 7) for c in point)


def _clip_interface(ob,spec):
    """Bisect before deletion so surviving source ends on clean edges."""
    origin,along,normal,length=_frame(spec)
    low=-.18*length;high=(1.18 if spec['front'] else 1.30)*length
    slope=(spec['top_width']-spec['bottom_width'])/length
    width=spec['bottom_width']*1.085
    # Each plane points outward from the volume. The skin beyond this small
    # interface remains source geometry, including the primary roof silhouette.
    planes=[(origin+along*low,-along), (origin+along*high,along),
            (origin+Vector((width,0,0)),Vector((1,0,0))-along*slope*1.085),
            (origin-Vector((width,0,0)),Vector((-1,0,0))-along*slope*1.085),
            (origin+normal*.20,normal),(origin-normal*.24,-normal)]
    world=ob.matrix_world;inv=world.inverted()
    local_planes=[(inv@p,(world.to_3x3().transposed()@n).normalized()) for p,n in planes]
    bm=bmesh.new();bm.from_mesh(ob.data)
    def inside(p,tolerance=1e-6):return all((p-co).dot(n)<=tolerance for co,n in local_planes)
    candidates=[f for f in bm.faces if any(inside(v.co,.035) for v in f.verts)]
    if not candidates:bm.free();return dict(removedFaces=0,boundary=[],cutEdges=[])
    # Restrict bisects to faces touching the repair volume so an aperture cut
    # does not subdivide doors, fenders or the entire roof on the same mesh.
    for co,n in local_planes:
        candidates=[f for f in bm.faces if any(inside(v.co,.035) for v in f.verts)]
        edges={e for f in candidates for e in f.edges};verts={v for f in candidates for v in f.verts}
        bm.edges.index_update();bm.verts.index_update();bm.faces.index_update()
        geom=sorted(verts,key=lambda v:v.index)+sorted(edges,key=lambda e:e.index)+sorted(candidates,key=lambda f:f.index)
        bmesh.ops.bisect_plane(bm,geom=geom,dist=1e-6,plane_co=co,plane_no=n)
    removed=[f for f in bm.faces if inside(f.calc_center_median())]
    count=len(removed)
    removed_set=set(removed)
    cut_edges=[tuple(_key(world@v.co) for v in e.verts) for e in bm.edges
               if any(f in removed_set for f in e.link_faces)
               and any(f not in removed_set for f in e.link_faces)]
    bmesh.ops.delete(bm,geom=removed,context='FACES')
    # Remove only small disconnected islands wholly inside this repair band.
    # A surviving pillar, roof, or articulated skin cannot satisfy this bound.
    seen=set();fragments=[]
    for face in bm.faces:
        if face in seen:continue
        todo=[face];component=[]
        while todo:
            current=todo.pop()
            if current in seen:continue
            seen.add(current);component.append(current)
            todo.extend(neighbor for edge in current.edges for neighbor in edge.link_faces if neighbor not in seen)
        if (len(component)<64 and sum(f.calc_area() for f in component)<.003
                and all(inside(v.co,.04) for f in component for v in f.verts)):
            fragments.extend(component)
    if fragments:bmesh.ops.delete(bm,geom=fragments,context='FACES')
    # Existing tears can run outside the clipping prism. Capture their actual
    # exposed edges too; the bridge follows the closed source boundary, not a
    # nearest-point approximation or a straight line across a missing section.
    bm.normal_update()
    boundary=[]
    normal_matrix=world.to_3x3().inverted().transposed()
    for edge in bm.edges:
        if edge.is_boundary and all(inside(v.co,.20) for v in edge.verts):
            boundary.append(dict(points=[list(world@v.co) for v in edge.verts],
                                 normals=[list((normal_matrix@v.normal).normalized()) for v in edge.verts]))
    bm.to_mesh(ob.data);bm.free();ob.data.update()
    return dict(removedFaces=count,removedFragmentFaces=len(fragments),boundary=boundary,cutEdges=cut_edges)


def _cut_loop(spec,results):
    """Trace an actual closed edge cycle after cancelling shared panel seams."""
    records=defaultdict(list);points={};normals=defaultdict(list)
    for result in results.values():
        for edge in result['boundary']:
            keys=tuple(_key(p) for p in edge['points'])
            records[tuple(sorted(keys))].append(edge)
            for key,point,normal in zip(keys,edge['points'],edge['normals']):
                points[key]=Vector(point);normals[key].append(Vector(normal))
    adjacency=defaultdict(set)
    for (a,b),records_for_edge in records.items():
        if len(records_for_edge)!=1:continue
        adjacency[a].add(b);adjacency[b].add(a)
    seen=set();cycles=[]
    origin,along,_,length=_frame(spec)
    for start in sorted(adjacency):
        if start in seen:continue
        todo=[start];component=[]
        while todo:
            key=todo.pop()
            if key in seen:continue
            seen.add(key);component.append(key);todo.extend(adjacency[key]-seen)
        if len(component)<16 or any(len(adjacency[k])!=2 for k in component):continue
        ordered=[min(component)];previous=None;current=ordered[0]
        while True:
            nxt=next(k for k in sorted(adjacency[current]) if k!=previous)
            if nxt==ordered[0]:break
            ordered.append(nxt);previous,current=current,nxt
        area=sum(points[a].x*(points[b]-origin).dot(along)-points[b].x*(points[a]-origin).dot(along)
                 for a,b in zip(ordered,ordered[1:]+ordered[:1]))/2
        if area<0:ordered.reverse()
        cycles.append((abs(area),ordered))
    if not cycles:raise ValueError('No closed source aperture boundary; repair cannot use a fitted skirt')
    area,ordered=max(cycles)
    if area<.35:raise ValueError('Source boundary does not enclose the complete windshield')
    normal_list=[]
    for key in ordered:
        n=sum(normals[key],Vector()).normalized()
        if n.dot(_frame(spec)[2])<0:n.negate()
        normal_list.append(n)
    return [points[k] for k in ordered],normal_list,dict(boundaryVertices=len(ordered),projectedAreaM2=area)


def _phase(spec,point):
    origin,along,_,length=_frame(spec)
    t=(point-origin).dot(along)/length
    width=1.085*(spec['bottom_width']+(spec['top_width']-spec['bottom_width'])*t)
    x=point.x/width;y=2*(t+.18)/((1.18 if spec['front'] else 1.30)+.18)-1
    if abs(x)>abs(y):
        return 1+(y/abs(x)+1)/2 if x>0 else 3+(1-y/abs(x))/2
    return (x/abs(y)+1)/2 if y<0 else 2+(1-x/abs(y))/2


def _at_phase(phase,width=1.022,low=-.016,high=1.016):
    side=int(phase)%4;w=phase-int(phase)
    return [(-width+2*width*w,low),(width,low+(high-low)*w),
            (width-2*width*w,high),(-width,high-(high-low)*w)][side]


def _boundary_bridge(spec,name,material,outer,normals):
    """Exact cut vertices outside; continuous bridge to the regular gasket."""
    phases=[_phase(spec,p) for p in outer]
    start=min(range(len(phases)),key=lambda i:phases[i])
    outer=outer[start:]+outer[:start];normals=normals[start:]+normals[:start]
    phases=phases[start:]+phases[:start]
    reversals=sum(phases[i+1]<phases[i]-1e-5 for i in range(len(phases)-1))
    # Tiny pre-existing tears can double back in projection. Parameterize each
    # side by its actual edge lengths, keeping the full boundary in order;
    # sorting projected vertices would cross or discard those source edges.
    anchors=[0]+[min(range(len(phases)),key=lambda i:abs(phases[i]-k)) for k in (1,2,3)]+[len(outer)]
    if anchors!=sorted(set(anchors)):raise ValueError(f'{name} boundary corners are not ordered')
    for side,(a,b) in enumerate(zip(anchors,anchors[1:])):
        lengths=[(outer[(i+1)%len(outer)]-outer[i]).length for i in range(a,b)]
        total=sum(lengths);walked=0
        for i,edge_length in zip(range(a,b),lengths):
            phases[i]=side+walked/total;walked+=edge_length
    n=len(outer);verts=list(outer);faces=[]
    # A ruled first strip preserves the source edge without extending noisy or
    # inverted source normals into spikes across the new sheet metal.
    for p,normal,phase in zip(outer,normals,phases):
        u,t=_at_phase(phase);delta=_surface(spec,u,t)-p
        verts.append(p+delta*.30)
    for i in range(n):faces.append((i,(i+1)%n,(i+1)%n+n,i+n))
    regular=[i/32 for i in range(128)];inner_start=len(verts)
    verts.extend(_surface(spec,*_at_phase(phase)) for phase in regular)
    # Zipper two ordered loops, keeping every source edge, including tiny tear
    # edges that an evenly sampled or nearest-surface perimeter would miss.
    i=j=0
    while i<n or j<128:
        a=n+i%n;b=inner_start+j%128
        next_outer=phases[(i+1)%n]+(4 if i+1>=n else 0)
        next_inner=regular[(j+1)%128]+(4 if j+1>=128 else 0)
        if i<n and (j==128 or next_outer<=next_inner):
            faces.append((a,n+(i+1)%n,b));i+=1
        else:
            faces.append((a,inner_start+(j+1)%128,b));j+=1
    # The outer cycle is counterclockwise in x/along. Its normal faces the
    # exterior at the front and the interior at the rear.
    if not spec['front']:faces=[f[::-1] for f in faces]
    ob=_mesh(name,verts,faces,material)
    ob['exactSourceBoundaryVertexCount']=n
    ob['exactSourceBoundaryMaxGapM']=max((ob.data.vertices[i].co-p).length for i,p in enumerate(outer))
    ob['sourceProjectionReversals']=reversals
    return ob


def rebuild_vehicle_greenhouse():
    """Return measured scene changes for the candidate's build report."""
    if any(bpy.data.objects.get(name+'_Surround') for name in APERTURES):
        raise ValueError('Greenhouse must start from the pre-repair source scene, not a previously clipped candidate')
    removed_objects=[]
    for name in ['Windscreen','RearWindow','WindshieldHeader']:
        ob=bpy.data.objects.get(name)
        if ob:bpy.data.objects.remove(ob,do_unlink=True);removed_objects.append(name)
    glass=bpy.data.materials.get('VehicleGlass')
    paint=bpy.data.materials.get('VehiclePaint')
    if not glass or not paint:raise ValueError('Greenhouse requires VehicleGlass and VehiclePaint')
    gasket=_trim_material('VehicleWindowGasket',(.012,.014,.016))
    source=[o for o in bpy.context.scene.objects if o.type=='MESH' and o.name in ['Body','HoodSkin','TrunkSkin']]
    report={'removedObjects':removed_objects,'apertures':{}}
    for name,spec in APERTURES.items():
        cuts={ob.name:_clip_interface(ob,spec) for ob in source}
        outer,normals,boundary_report=_cut_loop(spec,cuts)
        meshes=[_glass(spec,name,glass),
                _ring(spec,name+'_Gasket',gasket,(1,0,1),(1.022,-.016,1.016)),
                _boundary_bridge(spec,name+'_Surround',paint,outer,normals)]
        report['apertures'][name]={'removedFaces':{k:v['removedFaces'] for k,v in cuts.items()},
                                 'removedFragmentFaces':{k:v.get('removedFragmentFaces',0) for k,v in cuts.items()},
                                 'sourceBoundary':boundary_report,
                                 'createdMeshes':[o.name for o in meshes],
                                 'createdFaces':sum(len(o.data.polygons) for o in meshes),
                                 'createdTriangles':sum(len(f.vertices)-2 for o in meshes for f in o.data.polygons),
                                 'outerSeamGapM':meshes[2]['exactSourceBoundaryMaxGapM']}
    return report
